pub mod cas;
pub mod codec;
pub mod commands;
pub mod invitations;
pub mod promote;
pub mod runner_fencing;
pub mod runners;
pub mod sync;
pub mod workspaces;

use crate::error::ServerError;
use deadpool_postgres::{
    Client, Config, ManagerConfig, Pool, RecyclingMethod, Runtime, Transaction,
};
use tokio_postgres::{IsolationLevel, NoTls};

const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../../migrations/0001_initial_team.sql"))];
const MIGRATION_LOCK_KEY: &str = "runlark-schema-migrations";

/// Creates a `deadpool_postgres::Pool` from a `PostgreSQL` connection URL.
///
/// # Errors
/// Returns [`ServerError::Pool`] if the pool configuration or URL is invalid.
pub fn create_pool(postgres_url: &str) -> Result<Pool, ServerError> {
    let mut cfg = Config::new();
    cfg.url = Some(postgres_url.to_string());
    cfg.manager = Some(ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    });
    cfg.create_pool(Some(Runtime::Tokio1), NoTls)
        .map_err(|e| ServerError::Pool(e.to_string()))
}

/// Applies all embedded `PostgreSQL` migrations idempotently inside `schema_migrations`.
///
/// # Errors
/// Returns [`ServerError`] if acquiring a connection or executing DDL fails.
pub async fn run_migrations(pool: &Pool) -> Result<(), ServerError> {
    let mut client = pool
        .get()
        .await
        .map_err(|e| ServerError::Pool(e.to_string()))?;

    let tx = client.transaction().await?;
    tx.query_one(
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
        &[&MIGRATION_LOCK_KEY],
    )
    .await?;

    tx.batch_execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version BIGINT PRIMARY KEY,
            applied_at BIGINT NOT NULL
        );",
    )
    .await?;

    let now = ServerError::to_pg_i64(crate::config::ServerConfig::current_timestamp_ms())?;

    for &(version, sql) in MIGRATIONS {
        let existing = tx
            .query_opt(
                "SELECT version FROM schema_migrations WHERE version = $1",
                &[&version],
            )
            .await?;
        if existing.is_none() {
            tx.batch_execute(sql).await?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES ($1, $2)",
                &[&version, &now],
            )
            .await?;
        }
    }

    tx.commit().await?;
    Ok(())
}

/// Borrows a `PostgreSQL` client from `pool`.
///
/// # Errors
/// Returns [`ServerError::Pool`] if acquiring a connection fails.
pub async fn get_client(pool: &Pool) -> Result<Client, ServerError> {
    pool.get()
        .await
        .map_err(|e| ServerError::Pool(e.to_string()))
}

/// Starts a `REPEATABLE READ` read-only `PostgreSQL` transaction so snapshot queries and their
/// `cursor_seq` agree consistently without blocking concurrent writers.
///
/// # Errors
/// Returns [`ServerError`] if starting the transaction fails.
pub async fn begin_repeatable_read(client: &mut Client) -> Result<Transaction<'_>, ServerError> {
    client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .map_err(Into::into)
}

/// Acquires a transaction-scoped advisory lock per workspace so `workspace_events` `BIGSERIAL`
/// cursor sequences commit in strict monotonic order with zero `REPEATABLE READ` snapshot gaps.
///
/// # Errors
/// Returns [`ServerError`] if executing the advisory lock query fails.
pub(crate) async fn lock_workspace_write(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &runlark_domain::WorkspaceId,
) -> Result<(), ServerError> {
    tx.query_one(
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
        &[&workspace_id.as_ref()],
    )
    .await?;
    Ok(())
}
