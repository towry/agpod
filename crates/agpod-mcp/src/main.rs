use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    agpod_mcp::AgpodMcpServer::new().serve_stdio().await
}
