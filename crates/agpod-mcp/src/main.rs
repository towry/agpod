use agpod_core::init_logging;
use anyhow::Result;
use clap::Parser;
use tracing::{error, warn};

#[derive(Debug, Parser)]
#[command(name = "agpod-mcp")]
struct Args {
    /// Run the MCP server in read-only mode; only read tools for the current open case are exposed
    #[arg(long)]
    readonly: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if let Err(err) = init_logging("agpod-mcp") {
        eprintln!("Warning: failed to initialize logging: {err}");
    }
    warn!("agpod-mcp starting");

    let server = agpod_mcp::AgpodMcpServer::with_options(
        std::env::var("AGPOD_CASE_DATA_DIR").ok(),
        std::env::var("AGPOD_CASE_SERVER_ADDR").ok(),
        args.readonly,
    );

    match server.serve_stdio().await {
        Ok(()) => Ok(()),
        Err(err) => {
            error!(error = %err, "agpod-mcp exited with error");
            Err(err)
        }
    }
}
