use agpod_case::{CaseConfig, CaseOverrides, CaseServer};
use agpod_core::init_logging;
use anyhow::Result;
use clap::Parser;
use tracing::{error, info, warn};

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
    if let Err(err) = init_logging("agpod-case-server") {
        eprintln!("Warning: failed to initialize logging: {err}");
    }
    let cli = Cli::parse();
    let config = CaseConfig::load(CaseOverrides {
        data_dir: cli.data_dir.as_deref(),
        server_addr: cli.server_addr.as_deref(),
    });
    info!(
        server_addr = %config.server_addr,
        data_dir = %config.data_dir.to_string_lossy(),
        "starting agpod-case-server"
    );
    match CaseServer::new(config).await?.serve().await {
        Ok(()) => {
            warn!("agpod-case-server exited");
            Ok(())
        }
        Err(err) => {
            error!(error = %err, "agpod-case-server exited with error");
            Err(err.into())
        }
    }
}
