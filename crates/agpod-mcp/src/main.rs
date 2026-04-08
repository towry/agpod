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
    let case_data_dir = std::env::var("AGPOD_CASE_DATA_DIR").ok();
    let case_server_addr = std::env::var("AGPOD_CASE_SERVER_ADDR").ok();
    let tmux = std::env::var("TMUX").ok();
    let tmux_pane = std::env::var("TMUX_PANE").ok();
    let term = std::env::var("TERM").ok();
    warn!(
        has_case_data_dir = case_data_dir.is_some(),
        has_case_server_addr = case_server_addr.is_some(),
        has_tmux = tmux.is_some(),
        tmux_pane = tmux_pane.as_deref().unwrap_or("<missing>"),
        term = term.as_deref().unwrap_or("<missing>"),
        "agpod-mcp starting"
    );

    let server =
        agpod_mcp::AgpodMcpServer::with_options(case_data_dir, case_server_addr, args.readonly);

    match server.serve_stdio().await {
        Ok(()) => Ok(()),
        Err(err) => {
            error!(error = %err, "agpod-mcp exited with error");
            Err(err)
        }
    }
}
