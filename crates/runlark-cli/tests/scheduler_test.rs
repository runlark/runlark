use reqwest::Client;
use runlark_cli::{
    AppState, DispatchContext, EffectOutcome, OutboxConfig, OutboxDispatcher, OutboxEffector,
    Scheduler, build_router,
};
use runlark_domain::{RunStatus, Timestamp};
use runlark_local::{RunQueries, SessionStore, initialize_schema, open_sqlite};
use serde_json::{Value, json};
use std::fs;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::watch;

struct RecordingEffector {
    dispatched_runs: Arc<Mutex<Vec<String>>>,
}

impl OutboxEffector for RecordingEffector {
    fn execute<'a>(
        &'a self,
        ctx: &'a DispatchContext,
    ) -> Pin<Box<dyn Future<Output = EffectOutcome> + Send + 'a>> {
        Box::pin(async move {
            self.dispatched_runs
                .lock()
                .expect("lock")
                .push(ctx.command.run_id.0.clone());
            EffectOutcome::Acked
        })
    }
}

fn init_git_repo(dir: &Path) {
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(out.status.success(), "git {args:?} failed");
    };
    run(&["init"]);
    run(&["config", "user.email", "test@runlark.dev"]);
    run(&["config", "user.name", "Runlark Test"]);
    fs::write(dir.join("README.md"), "# Scheduler Test\n").expect("write");
    run(&["add", "README.md"]);
    run(&["commit", "-m", "init"]);
}

async fn setup_scheduler_harness(
    temp: &TempDir,
) -> (
    Client,
    String,
    std::path::PathBuf,
    Arc<Mutex<Vec<String>>>,
    watch::Sender<bool>,
    tokio::task::JoinHandle<()>,
) {
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("mkdir");
    init_git_repo(&repo_dir);

    let db_path = temp.path().join("sched.db");
    let conn = open_sqlite(&db_path).expect("open db");
    initialize_schema(&conn).expect("schema");
    drop(conn);

    let state = AppState::new(db_path.clone());
    let run_log = Arc::new(Mutex::new(Vec::new()));
    let effector = Arc::new(RecordingEffector {
        dispatched_runs: run_log.clone(),
    });
    let outbox_worker = Arc::new(OutboxDispatcher::new(
        db_path.clone(),
        effector,
        OutboxConfig {
            poll_interval: Duration::from_millis(30),
            retry_backoff_ms: 0,
            cleanup_worktree_on_archive: false,
        },
    ));
    let scheduler =
        Arc::new(Scheduler::new(state.clone()).with_poll_interval(Duration::from_millis(30)));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(outbox_worker.run_loop(state.outbox_notify.clone(), shutdown_rx.clone()));
    tokio::spawn(scheduler.run_loop(state.scheduler_notify.clone(), shutdown_rx));

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let client = Client::new();
    let base = format!("http://{addr}");

    let proj_res = client
        .post(format!("{base}/api/v1/projects"))
        .json(&json!({
            "project_id": "proj_sched",
            "name": "Sched Repo",
            "repo_root": repo_dir.to_string_lossy(),
        }))
        .send()
        .await
        .expect("create project");
    assert_eq!(proj_res.status(), 201);

    for i in 1..=3 {
        let res = client
            .post(format!("{base}/api/v1/tasks"))
            .json(&json!({
                "task_id": format!("task_{i}"),
                "project_id": "proj_sched",
                "title": format!("Task {i}"),
                "kind": "quick_fix",
            }))
            .send()
            .await
            .expect("create task");
        assert_eq!(res.status(), 201);
    }

    (client, base, db_path, run_log, shutdown_tx, server)
}

async fn start_task(client: &Client, base: &str, task: &str, run: &str, sess: &str) -> Value {
    client
        .post(format!("{base}/api/v1/tasks/{task}/start"))
        .json(&json!({ "run_id": run, "session_id": sess }))
        .send()
        .await
        .expect("start task")
        .json()
        .await
        .expect("json")
}

#[tokio::test]
async fn scheduler_enforces_two_slot_cap_and_creates_isolated_worktrees() {
    let temp = TempDir::new().expect("temp dir");
    let (client, base, db_path, run_log, shutdown_tx, server) =
        setup_scheduler_harness(&temp).await;

    let s1 = start_task(&client, &base, "task_1", "run_slot_1", "sess_slot_1").await;
    assert_eq!(s1["run_status"], "running");
    assert_eq!(s1["admitted"], true);

    let s2 = start_task(&client, &base, "task_2", "run_slot_2", "sess_slot_2").await;
    assert_eq!(s2["run_status"], "running");
    assert_eq!(s2["admitted"], true);

    // 3rd task enters Queued with empty worktree_path because 2 slots are occupied.
    let s3 = start_task(&client, &base, "task_3", "run_slot_3", "sess_slot_3").await;
    assert_eq!(s3["run_status"], "queued");
    assert_eq!(s3["admitted"], false);
    assert_eq!(s3["run"]["worktree_path"], "");
    assert_ne!(s1["run"]["worktree_path"], s2["run"]["worktree_path"]);
    assert!(
        s1["run"]["worktree_path"]
            .as_str()
            .expect("wt1")
            .ends_with(".runlark/worktrees/run_slot_1")
    );
    assert!(
        s2["run"]["worktree_path"]
            .as_str()
            .expect("wt2")
            .ends_with(".runlark/worktrees/run_slot_2")
    );

    tokio::time::sleep(Duration::from_millis(80)).await;
    {
        let conn = open_sqlite(&db_path).expect("open");
        assert_eq!(
            RunQueries::list_executing_runs(&conn).expect("exec").len(),
            2
        );
        let run3 = SessionStore::get_run(&conn, &runlark_domain::RunId::from("run_slot_3"))
            .expect("get run 3")
            .expect("run 3 exists");
        assert_eq!(run3.status, RunStatus::Queued);
        assert!(
            !run_log
                .lock()
                .expect("lock")
                .contains(&"run_slot_3".to_string())
        );
    }

    // WaitingForApproval frees a slot, causing run_slot_3 to automatically admit & dispatch!
    let q_res = client
        .post(format!("{base}/api/v1/dsh/events"))
        .json(&json!({
            "UserQuestionAsked": {
                "session_id": "sess_slot_1",
                "run_id": "run_slot_1",
                "question_id": "q_slot_1",
                "prompt": "Confirm?",
                "options": ["Yes"]
            }
        }))
        .send()
        .await
        .expect("ask question");
    assert_eq!(q_res.status(), 200);

    wait_for_run_admitted(&client, &base, "run_slot_3", &run_log).await;

    let run3_detail: Value = client
        .get(format!("{base}/api/v1/runs/run_slot_3"))
        .send()
        .await
        .expect("get run 3")
        .json()
        .await
        .expect("json");
    assert_eq!(run3_detail["status"], "running");
    assert!(
        run3_detail["worktree_path"]
            .as_str()
            .expect("wt3")
            .ends_with(".runlark/worktrees/run_slot_3")
    );

    let conn = open_sqlite(&db_path).expect("open");
    let executing = RunQueries::list_executing_runs(&conn).expect("exec");
    assert_eq!(executing.len(), 2);
    assert_ne!(executing[0].worktree_path, executing[1].worktree_path);

    let _ = RunQueries::resolve_default_project_id(&conn, None);
    let _ = Timestamp(0);
    let _ = shutdown_tx.send(true);
    server.abort();
}

async fn wait_for_run_admitted(
    client: &Client,
    base: &str,
    run_id: &str,
    run_log: &Arc<Mutex<Vec<String>>>,
) {
    for _ in 0..40 {
        let sched_status: Value = client
            .get(format!("{base}/api/v1/scheduler/status"))
            .send()
            .await
            .expect("sched status")
            .json()
            .await
            .expect("json");
        let has_run = sched_status["active_run_ids"]
            .as_array()
            .expect("active_run_ids")
            .iter()
            .any(|v| v.as_str() == Some(run_id));
        if has_run && run_log.lock().expect("lock").contains(&run_id.to_string()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("{run_id} must admit after WaitingForApproval frees slot");
}
