use reqwest::Client;
use runlark_cli::{
    AppState, CreatePrOutcome, CreatePrRequest, GitPrError, PrCommandError, PrCommandOutput,
    PrCommandRunner, PrCreationService, VerificationService, VerifyPipelineOptions,
    WorkflowPipeline, build_router,
};
use runlark_domain::{CommandId, RunId, Timestamp};
use runlark_local::{initialize_schema, open_sqlite};
use serde_json::{Value, json};
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

struct MockPrRunner {
    calls: Arc<Mutex<Vec<String>>>,
    gh_error: Option<PrCommandError>,
}

impl PrCommandRunner for MockPrRunner {
    fn run_git<'a>(
        &'a self,
        _cwd: &'a Path,
        args: &'a [&'a str],
    ) -> Pin<Box<dyn Future<Output = Result<PrCommandOutput, PrCommandError>> + Send + 'a>> {
        let joined = format!("git {}", args.join(" "));
        Box::pin(async move {
            self.calls.lock().expect("lock").push(joined);
            Ok(PrCommandOutput {
                exit_code: 0,
                stdout: "branch pushed".to_string(),
                stderr: String::new(),
            })
        })
    }

    fn run_gh<'a>(
        &'a self,
        _cwd: &'a Path,
        args: &'a [&'a str],
    ) -> Pin<Box<dyn Future<Output = Result<PrCommandOutput, PrCommandError>> + Send + 'a>> {
        let joined = format!("gh {}", args.join(" "));
        Box::pin(async move {
            self.calls.lock().expect("lock").push(joined);
            match &self.gh_error {
                Some(PrCommandError::Unknown(msg)) => Err(PrCommandError::Unknown(msg.clone())),
                Some(PrCommandError::Failed(msg)) => Err(PrCommandError::Failed(msg.clone())),
                None => Ok(PrCommandOutput {
                    exit_code: 0,
                    stdout: "https://github.com/runlark/runlark/pull/42\n".to_string(),
                    stderr: String::new(),
                }),
            }
        })
    }
}

fn solo_default_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("workflows")
        .join("solo-default")
}

fn git_run(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(out.status.success(), "git {args:?} failed");
}

fn init_git_repo(dir: &Path) {
    git_run(dir, &["init"]);
    git_run(dir, &["config", "user.email", "test@runlark.dev"]);
    git_run(dir, &["config", "user.name", "Runlark Test"]);
    fs::write(dir.join("README.md"), "# PR Test\n").expect("write");
    git_run(dir, &["add", "README.md"]);
    git_run(dir, &["commit", "-m", "init"]);
}

async fn setup_pr_harness(
    temp: &TempDir,
) -> (AppState, Client, String, tokio::task::JoinHandle<()>) {
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("mkdir");
    init_git_repo(&repo_dir);

    let db_path = temp.path().join("pr.db");
    let cas_dir = temp.path().join("wasm-cas");
    let conn = open_sqlite(&db_path).expect("open db");
    initialize_schema(&conn).expect("schema");
    drop(conn);

    let verification = VerificationService::with_cas_dir(&cas_dir).expect("init");
    verification
        .install_package_dir(&solo_default_dir())
        .expect("install");
    let app_state = AppState::with_verification(db_path, verification);

    let app = build_router(app_state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let client = Client::new();
    let base = format!("http://{addr}");

    client
        .post(format!("{base}/api/v1/projects"))
        .json(&json!({
            "project_id": "proj_pr",
            "name": "PR Repo",
            "repo_root": repo_dir.to_string_lossy(),
        }))
        .send()
        .await
        .expect("proj");

    (app_state, client, base, server)
}

async fn create_and_commit_task_run(
    client: &Client,
    base: &str,
    task_id: &str,
    run_id: &str,
    session_id: &str,
    file_name: &str,
) -> PathBuf {
    client
        .post(format!("{base}/api/v1/tasks"))
        .json(&json!({
            "task_id": task_id,
            "project_id": "proj_pr",
            "title": format!("Task {task_id}"),
            "kind": "quick_fix",
        }))
        .send()
        .await
        .expect("task");
    let start: Value = client
        .post(format!("{base}/api/v1/tasks/{task_id}/start"))
        .json(&json!({ "run_id": run_id, "session_id": session_id }))
        .send()
        .await
        .expect("start")
        .json()
        .await
        .expect("json");
    let wt = PathBuf::from(start["run"]["worktree_path"].as_str().expect("wt"));
    fs::write(wt.join(file_name), "hello pr\n").expect("write feature");
    git_run(&wt, &["add", file_name]);
    git_run(&wt, &["commit", "-m", "feat: add change"]);
    wt
}

fn pr_request(cmd_id: &str, run_id: &str, draft: bool) -> CreatePrRequest {
    CreatePrRequest {
        command_id: CommandId::from(cmd_id),
        run_id: RunId::from(run_id),
        title: draft.then(|| "Add ok feature".to_string()),
        body: None,
        base_branch: None,
        draft,
        expected_revision: None,
    }
}

#[tokio::test]
async fn pr_creation_rejects_unaccepted_or_stale_acceptance() {
    let temp = TempDir::new().expect("temp dir");
    let (app_state, client, base, server) = setup_pr_harness(&temp).await;
    let wt = create_and_commit_task_run(
        &client,
        &base,
        "task_pr",
        "run_pr_1",
        "sess_pr_1",
        "feature.txt",
    )
    .await;

    let diff_res: Value = client
        .get(format!("{base}/api/v1/runs/run_pr_1/diff"))
        .send()
        .await
        .expect("diff")
        .json()
        .await
        .expect("json");
    assert!(
        diff_res["diff"]
            .as_str()
            .expect("diff")
            .contains("hello pr")
    );
    assert_eq!(diff_res["changed_files"], json!(["feature.txt"]));

    let calls = Arc::new(Mutex::new(Vec::new()));
    let runner_ok = MockPrRunner {
        calls: calls.clone(),
        gh_error: None,
    };
    let unaccepted = PrCreationService::create_pr(
        &app_state,
        &runner_ok,
        &pr_request("cmd_pr_unacc", "run_pr_1", false),
    )
    .await;
    assert!(matches!(unaccepted, Err(GitPrError::Conflict(_))));
    assert!(calls.lock().expect("lock").is_empty());

    WorkflowPipeline::verify_and_advance(
        &app_state,
        &RunId::from("run_pr_1"),
        VerifyPipelineOptions::default(),
        Timestamp(10_000),
    )
    .await
    .expect("verify");
    let accept_res = client
        .post(format!("{base}/api/v1/tasks/task_pr/accept"))
        .json(&json!({ "actor": "alice", "comment": "LGTM" }))
        .send()
        .await
        .expect("accept");
    assert_eq!(accept_res.status(), 200);

    // Mutate worktree head_sha AFTER acceptance -> stale Decision::Accept MUST be rejected!
    fs::write(wt.join("extra.txt"), "unreviewed\n").expect("write extra");
    git_run(&wt, &["add", "extra.txt"]);
    git_run(&wt, &["commit", "-m", "unreviewed change"]);

    let outdated = PrCreationService::create_pr(
        &app_state,
        &runner_ok,
        &pr_request("cmd_pr_stale", "run_pr_1", false),
    )
    .await;
    assert!(matches!(outdated, Err(GitPrError::Conflict(_))));
    assert!(calls.lock().expect("lock").is_empty());

    server.abort();
}

#[tokio::test]
async fn pr_creation_handles_unknown_outcome_and_idempotent_replay() {
    let temp = TempDir::new().expect("temp dir");
    let (app_state, client, base, server) = setup_pr_harness(&temp).await;
    create_and_commit_task_run(
        &client,
        &base,
        "task_pr_2",
        "run_pr_2",
        "sess_pr_2",
        "ok.txt",
    )
    .await;

    WorkflowPipeline::verify_and_advance(
        &app_state,
        &RunId::from("run_pr_2"),
        VerifyPipelineOptions::default(),
        Timestamp(20_000),
    )
    .await
    .expect("verify 2");
    let acc2 = client
        .post(format!("{base}/api/v1/tasks/task_pr_2/accept"))
        .json(&json!({ "actor": "alice" }))
        .send()
        .await
        .expect("accept2");
    assert_eq!(acc2.status(), 200);

    let unknown_runner = MockPrRunner {
        calls: Arc::new(Mutex::new(Vec::new())),
        gh_error: Some(PrCommandError::Unknown("504 gateway timeout".to_string())),
    };
    let unk_outcome = PrCreationService::create_pr(
        &app_state,
        &unknown_runner,
        &pr_request("cmd_pr_unk", "run_pr_2", false),
    )
    .await
    .expect("unknown outcome");
    assert!(matches!(unk_outcome, CreatePrOutcome::Unknown { .. }));

    let calls = Arc::new(Mutex::new(Vec::new()));
    let runner_ok = MockPrRunner {
        calls: calls.clone(),
        gh_error: None,
    };
    let req = pr_request("cmd_pr_ok", "run_pr_2", true);
    let created = PrCreationService::create_pr(&app_state, &runner_ok, &req)
        .await
        .expect("create pr");
    match created {
        CreatePrOutcome::Created {
            pr_url, replayed, ..
        } => {
            assert_eq!(pr_url, "https://github.com/runlark/runlark/pull/42");
            assert!(!replayed);
        }
        other @ CreatePrOutcome::Unknown { .. } => panic!("expected Created, got {other:?}"),
    }
    assert_eq!(calls.lock().expect("lock").len(), 2);

    let replayed = PrCreationService::create_pr(&app_state, &runner_ok, &req)
        .await
        .expect("replay pr");
    assert!(matches!(
        replayed,
        CreatePrOutcome::Created { replayed: true, .. }
    ));
    assert_eq!(calls.lock().expect("lock").len(), 2);

    let mut mutated = req;
    mutated.draft = false;
    let conflict = PrCreationService::create_pr(&app_state, &runner_ok, &mutated).await;
    assert!(matches!(conflict, Err(GitPrError::Storage(_))));

    let task2_after: Value = client
        .get(format!("{base}/api/v1/tasks/task_pr_2"))
        .send()
        .await
        .expect("get task2")
        .json()
        .await
        .expect("json");
    assert_eq!(
        task2_after["task"]["pr_url"],
        "https://github.com/runlark/runlark/pull/42"
    );

    server.abort();
}
