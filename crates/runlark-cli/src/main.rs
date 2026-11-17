use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "runlark")]
#[command(about = "Runlark local daemon and workflow orchestrator", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the Runlark local daemon process
    Daemon {
        /// Host to bind the HTTP server to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port to listen on
        #[arg(short, long, default_value_t = 9876)]
        port: u16,

        /// Data directory for `SQLite` database and logs
        #[arg(long, default_value = ".runlark")]
        data_dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Daemon {
            host,
            port,
            data_dir,
        } => {
            runlark_cli::daemon::run_daemon(host, port, data_dir).await?;
        }
    }

    Ok(())
}
