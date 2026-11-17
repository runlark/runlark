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
    assert!(tables.contains(&"schema_migrations".to_string()));
}
