use clap::Parser;
use runlark_cli::cli::{Cli, run_cli};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    run_cli(cli).await
}
