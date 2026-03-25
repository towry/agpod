use agpod_core::init_logging;
use anyhow::Result;
use tracing::{error, warn};

#[tokio::main]
async fn main() -> Result<()> {
    if let Err(err) = init_logging("agpod-mcp") {
        eprintln!("Warning: failed to initialize logging: {err}");
    }
    warn!("agpod-mcp starting");

    match agpod_mcp::AgpodMcpServer::new().serve_stdio().await {
        Ok(()) => Ok(()),
        Err(err) => {
            error!(error = %err, "agpod-mcp exited with error");
            Err(err)
        }
    }
}
