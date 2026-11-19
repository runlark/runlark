use runlark_local::{initialize_schema, open_in_memory};

#[test]
fn test_migrations_fresh_and_idempotent() {
    let conn = open_in_memory().expect("open memory db");

    // Apply migrations first time
    initialize_schema(&conn).expect("apply schema first time");

    // Re-applying must be idempotent and succeed without error
    initialize_schema(&conn).expect("re-apply schema must be idempotent");

    // Verify tables exist
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .expect("prepare table query");

    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("query map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect tables");

    assert!(tables.contains(&"sessions".to_string()));
    assert!(tables.contains(&"runs".to_string()));
    assert!(tables.contains(&"commands".to_string()));
    assert!(tables.contains(&"events".to_string()));
    assert!(tables.contains(&"questions".to_string()));
    assert!(tables.contains(&"dsh_cursors".to_string()));
    assert!(tables.contains(&"projects".to_string()));
    assert!(tables.contains(&"tasks".to_string()));
    assert!(tables.contains(&"attempts".to_string()));
    assert!(tables.contains(&"decisions".to_string()));
    assert!(tables.contains(&"artifacts".to_string()));
    assert!(tables.contains(&"run_artifacts".to_string()));
    assert!(tables.contains(&"board_commands".to_string()));
    assert!(tables.contains(&"project_sync_bindings".to_string()));
    assert!(tables.contains(&"task_comments".to_string()));
    assert!(tables.contains(&"offline_drafts".to_string()));
    assert!(tables.contains(&"sync_outbox".to_string()));
    assert!(tables.contains(&"run_ownership".to_string()));
    assert!(tables.contains(&"team_workspaces".to_string()));
    assert!(tables.contains(&"team_members".to_string()));
    assert!(tables.contains(&"team_runners".to_string()));
    assert!(tables.contains(&"team_auth_sessions".to_string()));
    assert!(tables.contains(&"runner_buffered_events".to_string()));
    assert!(tables.contains(&"synced_cas_blobs".to_string()));
    assert!(tables.contains(&"schema_migrations".to_string()));

    let mut mig_stmt = conn
        .prepare("SELECT version FROM schema_migrations ORDER BY version ASC")
        .expect("prepare migrations query");
    let versions: Vec<i64> = mig_stmt
        .query_map([], |row| row.get(0))
        .expect("query versions")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect versions");
    assert_eq!(versions, vec![1, 2, 3, 4, 5]);
}

#[test]
fn test_upgrade_populated_v4_solo_db_to_v5_preserves_data_and_permissions() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let db_path = tmp.path().join("solo_v4.db");
    let conn = runlark_local::open_sqlite(&db_path).expect("open sqlite");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&db_path)
            .expect("db metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "SQLite DB file must be owner-only (0600)");
    }

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            applied_at INTEGER NOT NULL
        );",
    )
    .expect("create schema_migrations");
    for (ver, sql) in [
        (1_i64, include_str!("../migrations/0001_initial.sql")),
        (2_i64, include_str!("../migrations/0002_questions.sql")),
        (3_i64, include_str!("../migrations/0003_dsh_cursors.sql")),
        (
            4_i64,
            include_str!("../migrations/0004_a1_board_and_history.sql"),
        ),
    ] {
        conn.execute_batch(sql).expect("apply v1..v4 migration");
        conn.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, 100)",
            rusqlite::params![ver],
        )
        .expect("record migration");
    }

    let proj = runlark_domain::Project::new(
        runlark_domain::ProjectId::from("prj_v4"),
        "Solo V4",
        "/tmp/v4",
        "main",
        runlark_domain::BudgetPolicy::default(),
        runlark_domain::PolicyDigest::from("pol_v4"),
        runlark_domain::Timestamp(100),
    );
    runlark_local::ProjectStore::create_project(&conn, &proj).expect("seed v4 project");
    let task = runlark_domain::Task {
        id: runlark_domain::TaskId::from("tsk_v4"),
        project_id: proj.id.clone(),
        title: "Existing task".to_string(),
        description: "Preserved across v5 upgrade".to_string(),
        kind: runlark_domain::TaskKind::QuickFix,
        stage: runlark_domain::TaskStage::Inbox,
        workflow_stage: runlark_domain::WorkflowStage::Implementation,
        plan_text: None,
        plan_version: 0,
        base_ref: None,
        head_ref: None,
        pr_url: None,
        active_session_id: None,
        active_run_id: None,
        latest_run_id: None,
        budget_override: None,
        revision: runlark_domain::Revision(1),
        created_at: runlark_domain::Timestamp(110),
        updated_at: runlark_domain::Timestamp(110),
    };
    runlark_local::TaskStore::create_task(&conn, &task).expect("seed v4 task");

    initialize_schema(&conn).expect("upgrade populated v4 DB to v5");

    let loaded_proj = runlark_local::ProjectStore::get_project(&conn, &proj.id)
        .expect("get project")
        .expect("project preserved");
    assert_eq!(loaded_proj.name, "Solo V4");
    assert_eq!(
        runlark_local::SyncStore::get_ownership_mode(&conn, &proj.id).expect("ownership mode"),
        runlark_domain::ProjectOwnershipMode::SoloLocal
    );
    let loaded_task = runlark_local::TaskStore::get_task(&conn, &task.id)
        .expect("get task")
        .expect("task preserved");
    assert_eq!(loaded_task.title, "Existing task");
}
