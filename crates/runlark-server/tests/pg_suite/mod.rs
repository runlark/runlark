pub mod cas_metadata_test;
pub mod command_guards_test;
pub mod ownership_test;
pub mod reconciliation_and_revision_test;
pub mod sync_and_cas_test;
pub mod workspaces_test;

pub mod harness {
    use reqwest::{Client, Response, StatusCode};
    use runlark_domain::{
        BudgetPolicy, CommandId, CreateTaskCommand, CreateTeamRunCommand, PolicyDigest, ProjectId,
        Revision, RunId, RunnerId, TaskId, TaskKind, TeamCommand, TeamProjectSpec, Timestamp,
        UserId, WorkspaceId, WorkspaceSnapshot,
    };
    use runlark_server::config::ServerConfig;
    use runlark_server::pg::{run_migrations, workspaces};
    use runlark_server::server::{ServerState, build_router};
    use serde_json::{Value, json};
    use std::net::SocketAddr;
    use std::path::PathBuf;
    use tokio::net::TcpListener;
    use tokio_postgres::NoTls;
    use uuid::Uuid;

    pub struct TestServer {
        pub base_url: String,
        pub test_pg_url: String,
        pub cas_root: PathBuf,
        pub http: Client,
        shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
        _cas_dir: tempfile::TempDir,
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
        }
    }

    pub async fn spawn_test_server() -> TestServer {
        spawn_test_server_with_timeout(30_000).await
    }

    pub async fn spawn_test_server_with_timeout(runner_heartbeat_timeout_ms: u64) -> TestServer {
        let base_pg_url = std::env::var("RUNLARK_TEST_POSTGRES_URL").expect(
            "RUNLARK_TEST_POSTGRES_URL must be set to run real PostgreSQL integration tests",
        );
        let db_name = format!("runlark_test_{}", Uuid::new_v4().simple());

        let (admin_client, conn) = tokio_postgres::connect(&base_pg_url, NoTls)
            .await
            .expect("connect to admin postgres");
        tokio::spawn(async move {
            let _ = conn.await;
        });
        admin_client
            .batch_execute(&format!("CREATE DATABASE {db_name}"))
            .await
            .expect("create isolated test database");

        let test_pg_url = replace_db_in_url(&base_pg_url, &db_name);
        let cas_dir = tempfile::tempdir().expect("create temp cas dir");
        let cas_root = cas_dir.path().to_path_buf();
        let config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            postgres_url: test_pg_url.clone(),
            cas_root: cas_root.clone(),
            runner_heartbeat_timeout_ms,
        };

        let state = ServerState::connect_and_migrate(config)
            .await
            .expect("connect and migrate");
        run_migrations(&state.pool)
            .await
            .expect("idempotent re-migration must succeed");

        let app = build_router(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let addr: SocketAddr = listener.local_addr().expect("local addr");
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        TestServer {
            base_url: format!("http://{addr}"),
            test_pg_url,
            cas_root,
            http: Client::new(),
            shutdown_tx: Some(shutdown_tx),
            _cas_dir: cas_dir,
        }
    }

    fn replace_db_in_url(url: &str, db_name: &str) -> String {
        let (prefix, query) = match url.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (url, None),
        };
        let base = match prefix.rsplit_once('/') {
            Some((head, _)) => format!("{head}/{db_name}"),
            None => format!("{prefix}/{db_name}"),
        };
        match query {
            Some(q) => format!("{base}?{q}"),
            None => base,
        }
    }

    pub async fn bootstrap_user(srv: &TestServer, user_id: &str, name: &str) -> String {
        let pool = runlark_server::pg::create_pool(&srv.test_pg_url).expect("create pool");
        workspaces::issue_user_token(
            &pool,
            &UserId::from(user_id),
            name,
            None,
            "test",
            Timestamp(1000),
        )
        .await
        .expect("issue token")
        .raw_token
    }

    pub async fn create_ws(srv: &TestServer, token: &str, wid: &str, slug: &str) {
        let res = srv
            .http
            .post(format!("{}/api/v1/workspaces", srv.base_url))
            .bearer_auth(token)
            .json(&json!({ "workspace_id": wid, "name": "Runlark Team", "slug": slug }))
            .send()
            .await
            .expect("create workspace");
        assert_eq!(res.status(), StatusCode::CREATED);
    }

    pub async fn promote_project(srv: &TestServer, token: &str, wid: &str, pid: &str) {
        let spec = TeamProjectSpec {
            project_id: ProjectId::from(pid),
            workspace_id: WorkspaceId::from(wid),
            name: "Runlark".to_string(),
            default_branch: "main".to_string(),
            max_concurrent_runs: 2,
            budget_policy: BudgetPolicy::default(),
            policy_digest: PolicyDigest::from("pol_v1"),
            ownership_epoch: 1,
            revision: Revision(1),
            created_at: Timestamp(100),
            updated_at: Timestamp(100),
        };
        let res = srv
            .http
            .post(format!(
                "{}/api/v1/workspaces/{wid}/projects/promote",
                srv.base_url
            ))
            .bearer_auth(token)
            .json(&json!({ "project": spec, "tasks": [], "runs": [], "decisions": [] }))
            .send()
            .await
            .expect("promote project");
        assert_eq!(res.status(), StatusCode::OK);
    }

    pub async fn create_invite(
        srv: &TestServer,
        token: &str,
        wid: &str,
        hint: &str,
        role: &str,
    ) -> (String, String) {
        let res = srv
            .http
            .post(format!(
                "{}/api/v1/workspaces/{wid}/invitations",
                srv.base_url
            ))
            .bearer_auth(token)
            .json(&json!({ "invitee_hint": hint, "role": role }))
            .send()
            .await
            .expect("create invite");
        assert_eq!(res.status(), StatusCode::CREATED);
        let body: Value = res.json().await.expect("invite json");
        (
            body["invitation"]["id"]
                .as_str()
                .expect("invite id")
                .to_string(),
            body["invite_token"]
                .as_str()
                .expect("invite token")
                .to_string(),
        )
    }

    pub async fn accept_invite(
        srv: &TestServer,
        invite_token: &str,
        user_id: &str,
        display_name: &str,
    ) -> String {
        let res = srv
            .http
            .post(format!("{}/api/v1/invitations/accept", srv.base_url))
            .json(&json!({
                "invite_token": invite_token,
                "user_id": user_id,
                "display_name": display_name,
            }))
            .send()
            .await
            .expect("accept invite");
        assert_eq!(res.status(), StatusCode::OK);
        res.json::<Value>().await.expect("accept json")["bearer_token"]
            .as_str()
            .expect("bearer")
            .to_string()
    }

    pub async fn post_command(
        srv: &TestServer,
        token: &str,
        wid: &str,
        cmd: &TeamCommand,
    ) -> Response {
        srv.http
            .post(format!("{}/api/v1/workspaces/{wid}/commands", srv.base_url))
            .bearer_auth(token)
            .json(cmd)
            .send()
            .await
            .expect("post command")
    }

    pub async fn get_snapshot(srv: &TestServer, token: &str, wid: &str) -> WorkspaceSnapshot {
        srv.http
            .get(format!(
                "{}/api/v1/workspaces/{wid}/sync/snapshot",
                srv.base_url
            ))
            .bearer_auth(token)
            .send()
            .await
            .expect("get snapshot")
            .json()
            .await
            .expect("snapshot json")
    }

    pub fn sample_task_cmd(
        cmd_id: &str,
        task_id: &str,
        project_id: &str,
        title: &str,
    ) -> TeamCommand {
        TeamCommand::CreateTask(CreateTaskCommand {
            command_id: CommandId::from(cmd_id),
            task_id: TaskId::from(task_id),
            project_id: ProjectId::from(project_id),
            title: title.to_string(),
            description: String::new(),
            kind: TaskKind::QuickFix,
            base_ref: None,
            budget_override: None,
            created_at: Timestamp(100),
        })
    }

    pub fn sample_run_cmd(
        cmd_id: &str,
        run_id: &str,
        project_id: &str,
        task_id: &str,
        runner_id: Option<&str>,
    ) -> TeamCommand {
        TeamCommand::CreateTeamRun(CreateTeamRunCommand {
            command_id: CommandId::from(cmd_id),
            run_id: RunId::from(run_id),
            project_id: ProjectId::from(project_id),
            task_id: TaskId::from(task_id),
            runner_id: runner_id.map(RunnerId::from),
            expected_revision: Revision(1),
            created_at: Timestamp(101),
        })
    }
}
