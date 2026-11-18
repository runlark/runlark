use clap::{Parser, Subcommand};
use runlark_domain::{Timestamp, UserId, WorkspaceId};
use runlark_server::config::ServerConfig;
use runlark_server::pg::workspaces::BootstrapWorkspaceSpec;
use runlark_server::pg::{create_pool, run_migrations, workspaces};
use runlark_server::server::run_server;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "runlark-server")]
#[command(about = "Runlark team collaboration server (SaaS & self-host)")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the collaboration HTTP server against `PostgreSQL` and CAS storage
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(short, long, default_value_t = 9880)]
        port: u16,
        #[arg(long)]
        postgres_url: Option<String>,
        #[arg(long, default_value = ".runlark-server-cas")]
        cas_root: PathBuf,
        #[arg(long, default_value_t = 30_000)]
        runner_heartbeat_timeout_ms: u64,
    },
    /// Bootstrap an initial owner user (and optional workspace) with a fresh bearer token
    #[command(name = "bootstrap-owner", alias = "bootstrap-user")]
    BootstrapOwner {
        #[arg(long)]
        postgres_url: Option<String>,
        #[arg(long)]
        user_id: String,
        #[arg(long)]
        display_name: String,
        #[arg(long)]
        email: Option<String>,
        #[arg(long, default_value = "cli-bootstrap")]
        label: String,
        #[arg(long)]
        workspace_id: Option<String>,
        #[arg(long)]
        workspace_name: Option<String>,
        #[arg(long)]
        workspace_slug: Option<String>,
    },
}

fn resolve_postgres_url(
    flag: Option<String>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    flag.or_else(|| std::env::var("RUNLARK_POSTGRES_URL").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "missing --postgres-url or RUNLARK_POSTGRES_URL".into())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Serve {
            host,
            port,
            postgres_url,
            cas_root,
            runner_heartbeat_timeout_ms,
        } => {
            let postgres_url = resolve_postgres_url(postgres_url)?;
            run_server(ServerConfig {
                host,
                port,
                postgres_url,
                cas_root,
                runner_heartbeat_timeout_ms,
            })
            .await?;
        }
        Commands::BootstrapOwner {
            postgres_url,
            user_id,
            display_name,
            email,
            label,
            workspace_id,
            workspace_name,
            workspace_slug,
        } => {
            if user_id.trim().is_empty() {
                return Err("--user-id must not be empty".into());
            }
            if workspace_id
                .as_deref()
                .is_some_and(|id| id.trim().is_empty())
            {
                return Err("--workspace-id must not be empty".into());
            }
            if workspace_slug.is_none() && (workspace_id.is_some() || workspace_name.is_some()) {
                return Err(
                    "--workspace-slug is required when --workspace-id or --workspace-name is specified"
                        .into(),
                );
            }
            let workspace = workspace_slug.map(|slug| BootstrapWorkspaceSpec {
                workspace_id: workspace_id.map(WorkspaceId::from),
                name: workspace_name.unwrap_or_else(|| slug.clone()),
                slug,
            });
            let postgres_url = resolve_postgres_url(postgres_url)?;
            let pool = create_pool(&postgres_url)?;
            run_migrations(&pool).await?;
            let now = Timestamp(ServerConfig::current_timestamp_ms());
            let (issued, created_workspace_id) = workspaces::bootstrap_owner(
                &pool,
                &UserId::from(user_id),
                &display_name,
                email.as_deref(),
                &label,
                workspace,
                now,
            )
            .await?;

            println!(
                "{}",
                serde_json::json!({
                    "user_id": issued.user_id,
                    "display_name": issued.display_name,
                    "bearer_token": issued.raw_token,
                    "workspace_id": created_workspace_id,
                })
            );
        }
    }
    Ok(())
}
