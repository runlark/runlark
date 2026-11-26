use reqwest::header::{CONTENT_TYPE, HOST, ORIGIN};
use reqwest::{Client, Method, StatusCode};
use runlark_cli::{AppState, VerificationService, build_router, run_daemon_with_dsh};
use runlark_local::{initialize_schema, open_sqlite};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

struct Harness {
    client: Client,
    base: String,
    port: u16,
    _server: tokio::task::JoinHandle<()>,
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn package_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../workflows/solo-default")
}

async fn start(temp: &TempDir) -> Harness {
    let db_path = temp.path().join("guard.db");
    initialize_schema(&open_sqlite(&db_path).expect("open db")).expect("schema");
    let verification =
        VerificationService::with_cas_dir(temp.path().join("wasm-cas")).expect("cas dir");
    verification
        .install_package_dir(&package_dir())
        .expect("install solo-default");
    let app = build_router(AppState::with_verification(db_path, verification));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Harness {
        client: Client::new(),
        base: format!("http://127.0.0.1:{port}"),
        port,
        _server: server,
    }
}

async fn post_ok(h: &Harness, path: &str, body: &Value) -> Value {
    let resp = h
        .client
        .post(format!("{}{path}", h.base))
        .json(body)
        .send()
        .await
        .expect("post");
    assert!(resp.status().is_success(), "POST {path}: {}", resp.status());
    resp.json().await.expect("json")
}

async fn get_json(h: &Harness, path: &str) -> Value {
    let resp = h
        .client
        .get(format!("{}{path}", h.base))
        .send()
        .await
        .expect("get");
    resp.json().await.expect("json")
}

/// Drives a quick fix to Acceptance over HTTP and returns its task ID.
async fn task_in_acceptance(h: &Harness, temp: &TempDir) -> String {
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("mkdir repo");
    git(&repo, &["init"]);
    git(&repo, &["config", "user.email", "test@runlark.dev"]);
    git(&repo, &["config", "user.name", "Runlark Test"]);
    fs::write(repo.join("README.md"), "# Guard\n").expect("write readme");
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "init"]);

    let project = post_ok(
        h,
        "/api/v1/projects",
        &json!({ "name": "Guard", "repo_root": repo.display().to_string(), "default_branch": "main" }),
    )
    .await;
    let task = post_ok(
        h,
        "/api/v1/tasks",
        &json!({ "project_id": project["project_id"], "title": "Guard fix", "kind": "quick_fix" }),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();
    let started = post_ok(h, &format!("/api/v1/tasks/{task_id}/start"), &json!({})).await;
    let run_id = started["run"]["id"].as_str().expect("run id").to_string();
    let worktree = started["run"]["worktree_path"].as_str().expect("worktree");
    fs::write(Path::new(worktree).join("fix.txt"), "fixed\n").expect("write fix");
    let verdict = post_ok(
        h,
        &format!("/api/v1/runs/{run_id}/verify"),
        &json!({ "task_id": task_id }),
    )
    .await;
    assert_eq!(verdict["verdict"], "pass");
    assert_still_in_acceptance(h, &task_id).await;
    task_id
}

async fn assert_still_in_acceptance(h: &Harness, task_id: &str) {
    let entry = get_json(h, &format!("/api/v1/tasks/{task_id}")).await;
    assert_eq!(entry["task"]["stage"], "acceptance");
    assert_eq!(entry["decisions"], json!([]));
}

async fn assert_refused(resp: reqwest::Response, status: StatusCode, code: &str) {
    assert_eq!(resp.status(), status);
    let body: Value = resp.json().await.expect("error json");
    assert_eq!(body["error"], code);
}

#[tokio::test]
async fn cross_origin_requests_get_no_cors_grant() {
    let temp = TempDir::new().expect("tempdir");
    let h = start(&temp).await;
    let url = format!("{}/api/v1/tasks", h.base);

    let preflight = h
        .client
        .request(Method::OPTIONS, &url)
        .header(ORIGIN, "https://example.invalid")
        .header("access-control-request-method", "POST")
        .send()
        .await
        .expect("preflight");
    assert!(
        preflight
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );

    let get = h
        .client
        .get(&url)
        .header(ORIGIN, "https://example.invalid")
        .send()
        .await
        .expect("get");
    assert_eq!(get.status(), StatusCode::OK);
    assert!(get.headers().get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn foreign_host_and_non_json_posts_are_refused_before_any_decision() {
    let temp = TempDir::new().expect("tempdir");
    let h = start(&temp).await;
    let task_id = task_in_acceptance(&h, &temp).await;
    let accept = format!("{}/api/v1/tasks/{task_id}/accept", h.base);

    let foreign = h
        .client
        .post(&accept)
        .header(HOST, "example.invalid:9876")
        .json(&json!({ "comment": "rebound" }))
        .send()
        .await
        .expect("foreign host");
    assert_refused(foreign, StatusCode::FORBIDDEN, "forbidden_host").await;
    assert_still_in_acceptance(&h, &task_id).await;

    let bodyless = h.client.post(&accept).send().await.expect("bodyless");
    assert_refused(
        bodyless,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "json_body_required",
    )
    .await;
    let text_plain = h
        .client
        .post(&accept)
        .header(CONTENT_TYPE, "text/plain")
        .body(r#"{"comment":"simple request"}"#)
        .send()
        .await
        .expect("text/plain");
    assert_refused(
        text_plain,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "json_body_required",
    )
    .await;
    assert_still_in_acceptance(&h, &task_id).await;

    let accepted = h
        .client
        .post(&accept)
        .header(HOST, format!("LOCALHOST:{}", h.port))
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .body(r#"{"comment":"Verified"}"#)
        .send()
        .await
        .expect("accept");
    assert_eq!(accepted.status(), StatusCode::OK);
    let accepted: Value = accepted.json().await.expect("accept json");
    assert_eq!(accepted["task"]["stage"], "done");
}

#[tokio::test]
async fn loopback_hosts_are_accepted() {
    let temp = TempDir::new().expect("tempdir");
    let h = start(&temp).await;
    for host in ["127.0.0.1", "localhost", "LOCALHOST", "[::1]"] {
        let host = format!("{host}:{}", h.port);
        let get = h
            .client
            .get(format!("{}/api/v1/tasks", h.base))
            .header(HOST, &host)
            .send()
            .await
            .expect("get");
        assert_eq!(get.status(), StatusCode::OK, "GET with Host {host}");
        let post = h
            .client
            .post(format!("{}/api/v1/plugins/validate", h.base))
            .header(HOST, &host)
            .json(&json!({ "path": package_dir().display().to_string() }))
            .send()
            .await
            .expect("post");
        assert_eq!(post.status(), StatusCode::OK, "POST with Host {host}");
    }
}

#[tokio::test]
async fn daemon_refuses_non_loopback_hosts_before_touching_data() {
    let temp = TempDir::new().expect("tempdir");
    for host in ["0.0.0.0", "::", "192.0.2.1", "example.invalid"] {
        let data_dir = temp.path().join(format!("data-{}", host.replace(':', "_")));
        let start = run_daemon_with_dsh(host.to_string(), 0, data_dir.clone(), None, None);
        let err = tokio::time::timeout(Duration::from_secs(10), start)
            .await
            .unwrap_or_else(|_| panic!("{host}: daemon started instead of refusing"))
            .expect_err("non-loopback host must be refused");
        assert!(err.to_string().contains("loopback"), "{host}: {err}");
        assert!(!data_dir.exists(), "{host}: data dir was created");
    }

    let mut daemon = tokio::process::Command::new(env!("CARGO_BIN_EXE_runlark"))
        .args(["daemon", "--host", "0.0.0.0", "--port", "0", "--data-dir"])
        .arg(temp.path().join("data-bin"))
        .kill_on_drop(true)
        .spawn()
        .expect("spawn runlark");
    let status = tokio::time::timeout(Duration::from_secs(10), daemon.wait())
        .await
        .expect("runlark daemon --host 0.0.0.0 kept running")
        .expect("wait runlark");
    assert!(!status.success());
    assert!(!temp.path().join("data-bin").exists());
}
