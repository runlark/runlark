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
    assert!(tables.contains(&"schema_migrations".to_string()));

    let mut mig_stmt = conn
        .prepare("SELECT version FROM schema_migrations ORDER BY version ASC")
        .expect("prepare migrations query");
    let versions: Vec<i64> = mig_stmt
        .query_map([], |row| row.get(0))
        .expect("query versions")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect versions");
    assert_eq!(versions, vec![1, 2, 3, 4]);
}
