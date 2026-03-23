use agpod_case::{CaseConfig, CaseOverrides, CaseServer};
use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "agpod-case-server")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"), long_about = None)]
struct Cli {
    /// SurrealDB data directory (default: shared case config)
    #[arg(long, env = "AGPOD_CASE_DATA_DIR")]
    data_dir: Option<String>,

    /// Case server address (default: shared case config)
    #[arg(long, env = "AGPOD_CASE_SERVER_ADDR")]
    server_addr: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = CaseConfig::load(CaseOverrides {
        data_dir: cli.data_dir.as_deref(),
        server_addr: cli.server_addr.as_deref(),
    });
    CaseServer::new(config).await?.serve().await?;
    Ok(())
}
