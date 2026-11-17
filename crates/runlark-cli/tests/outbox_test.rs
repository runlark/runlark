use axum::Router;
use axum::http::StatusCode;
use axum::routing::post;
use runlark_cli::{
    DispatchContext, EffectOutcome, HttpDshEffector, OutboxConfig, OutboxDispatcher, OutboxEffector,
};
use runlark_domain::{
    CommandId, Revision, RunId, SendPromptCommand, Session, SessionCommand, SessionId,
    SessionStatus, TaskId, Timestamp,
};
use runlark_local::{
    CommandStore, LocalStore, ReconciliationEngine, SessionStore, initialize_schema, open_sqlite,
};
use std::fs;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

fn init_git_repo(dir: &Path) {
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(out.status.success());
    };
    run(&["init"]);
    run(&["config", "user.email", "test@runlark.dev"]);
    run(&["config", "user.name", "Runlark Test"]);
    fs::write(dir.join("file.txt"), "hello\n").expect("write file");
    run(&["add", "file.txt"]);
    run(&["commit", "-m", "init"]);
}

struct FlakyEffector {
    calls: AtomicUsize,
}

impl OutboxEffector for FlakyEffector {
    fn execute<'a>(
        &'a self,
        _ctx: &'a DispatchContext,
    ) -> Pin<Box<dyn Future<Output = EffectOutcome> + Send + 'a>> {
        Box::pin(async move {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                EffectOutcome::Failed {
                    error: "transient DSH error".to_string(),
                }
            } else {
                EffectOutcome::Acked
            }
        })
    }
}

struct UnknownEffector;

impl OutboxEffector for UnknownEffector {
    fn execute<'a>(
        &'a self,
        _ctx: &'a DispatchContext,
    ) -> Pin<Box<dyn Future<Output = EffectOutcome> + Send + 'a>> {
        Box::pin(async {
            EffectOutcome::Unknown {
                reason: "socket closed before response".to_string(),
            }
        })
    }
}

fn seed_session_and_prompt(db_path: &Path, workspace: &Path, sess_id: &str, run_id: &str) {
    let mut conn = open_sqlite(db_path).expect("open db");
    initialize_schema(&conn).expect("init schema");
    let session = Session {
        id: SessionId::from(sess_id),
        task_id: TaskId::from("task_outbox"),
        cwd: workspace.to_string_lossy().into_owned(),
        status: SessionStatus::Created,
        current_revision: Revision(1),
        active_run_id: None,
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };
    SessionStore::create_session(&conn, &session).expect("create session");
    let cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from(format!("cmd_{run_id}")),
        session_id: session.id,
        run_id: RunId::from(run_id),
        prompt: "Run task".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(1010),
    });
    LocalStore::apply_command(&mut conn, &cmd, Timestamp(1010)).expect("apply command");
}

#[tokio::test]
async fn test_outbox_creates_worktree_retries_failure_and_clears_error() {
    let temp = TempDir::new().expect("create temp dir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    init_git_repo(&repo_dir);

    let db_path = temp.path().join("runlark.db");
    seed_session_and_prompt(&db_path, &repo_dir, "sess_retry", "run_retry");

    let effector = Arc::new(FlakyEffector {
        calls: AtomicUsize::new(0),
    });
    let dispatcher = OutboxDispatcher::new(
        db_path.clone(),
        effector,
        OutboxConfig {
            retry_backoff_ms: 0,
            ..OutboxConfig::default()
        },
    );

    // 1. First pass: fails externally -> status 'failed', error_message set, worktree already updated
    let stats1 = dispatcher.dispatch_once().await.expect("dispatch 1");
    assert_eq!(stats1.attempted, 1);
    assert_eq!(stats1.failed, 1);
    assert_eq!(stats1.acked, 0);

    let conn = open_sqlite(&db_path).expect("open db");
    let cmd_after_fail = CommandStore::get_command(&conn, &CommandId::from("cmd_run_retry"))
        .expect("get cmd")
        .expect("exists");
    assert_eq!(cmd_after_fail.status, "failed");
    assert_eq!(cmd_after_fail.dispatch_attempts, 1);
    assert_eq!(
        cmd_after_fail.error_message.as_deref(),
        Some("transient DSH error")
    );

    let run = SessionStore::get_run(&conn, &RunId::from("run_retry"))
        .expect("get run")
        .expect("exists");
    assert!(run.worktree_path.ends_with(".runlark/worktrees/run_retry"));
    assert!(Path::new(&run.worktree_path).is_dir());
    drop(conn);

    // 2. Second pass: retry succeeds -> 'acked', error_message cleared to NULL
    let stats2 = dispatcher.dispatch_once().await.expect("dispatch 2");
    assert_eq!(stats2.attempted, 1);
    assert_eq!(stats2.acked, 1);

    let conn = open_sqlite(&db_path).expect("open db");
    let cmd_after_ack = CommandStore::get_command(&conn, &CommandId::from("cmd_run_retry"))
        .expect("get cmd")
        .expect("exists");
    assert_eq!(cmd_after_ack.status, "acked");
    assert_eq!(cmd_after_ack.dispatch_attempts, 2);
    assert_eq!(cmd_after_ack.error_message, None);
}

#[tokio::test]
async fn test_outbox_unknown_outcome_stays_dispatched_without_duplicates() {
    let temp = TempDir::new().expect("create temp dir");
    let ws = temp.path().join("ws");
    fs::create_dir_all(&ws).expect("create ws");
    let db_path = temp.path().join("runlark.db");
    seed_session_and_prompt(&db_path, &ws, "sess_unk", "run_unk");

    let dispatcher = OutboxDispatcher::new(
        db_path.clone(),
        Arc::new(UnknownEffector),
        OutboxConfig::default(),
    );

    let stats1 = dispatcher.dispatch_once().await.expect("dispatch 1");
    assert_eq!(stats1.attempted, 1);
    assert_eq!(stats1.unknown, 1);
    assert_eq!(stats1.acked, 0);
    assert_eq!(stats1.failed, 0);

    // Second pass must NOT re-dispatch an `unknown` command (`dispatched` state)
    let stats2 = dispatcher.dispatch_once().await.expect("dispatch 2");
    assert_eq!(stats2.attempted, 0);

    let conn = open_sqlite(&db_path).expect("open db");
    let cmd = CommandStore::get_command(&conn, &CommandId::from("cmd_run_unk"))
        .expect("get cmd")
        .expect("exists");
    assert_eq!(cmd.status, "dispatched");
    assert_eq!(cmd.dispatch_attempts, 1);

    let runs = SessionStore::list_runs_for_session(&conn, &SessionId::from("sess_unk"))
        .expect("list runs");
    assert_eq!(runs.len(), 1);

    let inflight = ReconciliationEngine::scan_inflight(&conn).expect("scan inflight");
    assert_eq!(inflight.unacked_commands.len(), 1);
    assert_eq!(inflight.unacked_commands[0].3, "dispatched");
}

#[tokio::test]
async fn test_http_dsh_effector_maps_200_500_and_504() {
    let mode = Arc::new(AtomicUsize::new(0));
    let mode_clone = Arc::clone(&mode);

    let app = Router::new().route(
        "/api/runlark/dispatch",
        post(move || {
            let m = mode_clone.load(Ordering::SeqCst);
            async move {
                match m {
                    0 => StatusCode::OK,
                    1 => StatusCode::INTERNAL_SERVER_ERROR,
                    _ => StatusCode::GATEWAY_TIMEOUT,
                }
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let temp = TempDir::new().expect("temp");
    let db_path = temp.path().join("http.db");
    seed_session_and_prompt(&db_path, temp.path(), "sess_http", "run_http");

    let conn = open_sqlite(&db_path).expect("open db");
    let cmd = CommandStore::get_command(&conn, &CommandId::from("cmd_run_http"))
        .expect("get cmd")
        .expect("exists");
    let parsed: SessionCommand = serde_json::from_str(&cmd.payload).expect("parse cmd");
    let ctx = DispatchContext {
        command: cmd,
        parsed,
        worktree_path: Some(temp.path().to_path_buf()),
    };

    let effector = HttpDshEffector::new(&format!("http://{addr}"));
    mode.store(0, Ordering::SeqCst);
    assert_eq!(effector.execute(&ctx).await, EffectOutcome::Acked);

    mode.store(1, Ordering::SeqCst);
    assert!(matches!(
        effector.execute(&ctx).await,
        EffectOutcome::Failed { .. }
    ));

    mode.store(2, Ordering::SeqCst);
    assert!(matches!(
        effector.execute(&ctx).await,
        EffectOutcome::Unknown { .. }
    ));
}
