//! Client for talking to the case-server, with optional local auto-start.
//!
//! Keywords: rpc client, auto start, local server, remote case server

use crate::cli::CaseCommand;
use crate::config::{CaseAccessMode, CaseConfig};
use crate::error::{CaseError, CaseResult};
use crate::repo_id::RepoIdentity;
use crate::rpc::{CaseRequest, CaseResponse, RepoIdentityPayload};
use serde_json::Value;
use std::process::Command;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout, Duration, Instant};
use tracing::{debug, info, warn};

const SERVER_START_TIMEOUT: Duration = Duration::from_secs(3);
const SERVER_START_RETRY_DELAY: Duration = Duration::from_millis(100);
const SLOW_SERVER_RPC_WARN_THRESHOLD: Duration = Duration::from_secs(1);

pub async fn execute_via_server(
    config: &CaseConfig,
    identity: RepoIdentity,
    command: CaseCommand,
) -> CaseResult<Value> {
    let repo_id = identity.repo_id.clone();
    let command_debug = format!("{command:?}");
    debug!(
        server_addr = %config.server_addr,
        repo_id = %repo_id,
        "executing case command via server"
    );
    let started = Instant::now();
    let response: CaseResponse = timeout(crate::CASE_REQUEST_TIMEOUT, async {
        ensure_server_available(config).await?;

        let mut stream = TcpStream::connect(&config.server_addr)
            .await
            .map_err(|err| {
                CaseError::DbConnection(format!("failed to connect case-server: {err}"))
            })?;
        let payload = serde_json::to_string(&CaseRequest {
            repo: RepoIdentityPayload::from(identity),
            command,
        })
        .map_err(CaseError::Json)?;

        stream
            .write_all(payload.as_bytes())
            .await
            .map_err(CaseError::Io)?;
        stream.write_all(b"\n").await.map_err(CaseError::Io)?;

        let mut line = String::new();
        let mut reader = BufReader::new(stream);
        reader.read_line(&mut line).await.map_err(CaseError::Io)?;

        serde_json::from_str(&line).map_err(CaseError::Json)
    })
    .await
    .map_err(|_| {
        CaseError::DbConnection(format!(
            "timed out waiting for case-server response after {} ms",
            crate::CASE_REQUEST_TIMEOUT.as_millis()
        ))
    })??;

    let elapsed = started.elapsed();
    if elapsed >= SLOW_SERVER_RPC_WARN_THRESHOLD {
        warn!(
            server_addr = %config.server_addr,
            repo_id = %repo_id,
            elapsed_ms = elapsed.as_millis(),
            command = %command_debug,
            "case command round-trip via case-server was slow"
        );
    }
    Ok(response.result)
}

async fn ensure_server_available(config: &CaseConfig) -> CaseResult<()> {
    if TcpStream::connect(&config.server_addr).await.is_ok() {
        debug!(server_addr = %config.server_addr, "case-server already reachable");
        return Ok(());
    }

    if config.access_mode == CaseAccessMode::Remote || !config.auto_start {
        warn!(
            server_addr = %config.server_addr,
            access_mode = ?config.access_mode,
            auto_start = config.auto_start,
            "case-server unreachable and auto-start unavailable"
        );
        return Err(CaseError::DbConnection(format!(
            "case-server is not reachable at {}",
            config.server_addr
        )));
    }

    auto_start_server(config).await?;
    wait_for_server(config).await
}

async fn auto_start_server(config: &CaseConfig) -> CaseResult<()> {
    let current_exe = std::env::current_exe()
        .map_err(|err| CaseError::Other(format!("resolve current executable failed: {err}")))?;
    let sibling = current_exe
        .parent()
        .map(|dir| dir.join("agpod-case-server"))
        .ok_or_else(|| {
            CaseError::Other("current executable has no parent directory".to_string())
        })?;
    let program = if sibling.exists() {
        sibling
    } else {
        current_exe
    };

    let mut command = Command::new(program);
    if let Some(name) = command.get_program().to_str() {
        if name.ends_with("agpod") {
            command.arg("case-server");
        }
    }
    command
        .arg("--server-addr")
        .arg(&config.server_addr)
        .arg("--data-dir")
        .arg(config.data_dir.to_string_lossy().to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    command
        .spawn()
        .map_err(|err| CaseError::Other(format!("failed to start case-server: {err}")))?;
    info!(
        server_addr = %config.server_addr,
        data_dir = %config.data_dir.to_string_lossy(),
        "auto-started case-server"
    );
    Ok(())
}

async fn wait_for_server(config: &CaseConfig) -> CaseResult<()> {
    let started = Instant::now();
    loop {
        if TcpStream::connect(&config.server_addr).await.is_ok() {
            info!(server_addr = %config.server_addr, "case-server became reachable");
            return Ok(());
        }

        if started.elapsed() >= SERVER_START_TIMEOUT {
            warn!(server_addr = %config.server_addr, "timed out waiting for case-server");
            return Err(CaseError::DbConnection(format!(
                "timed out waiting for case-server at {}",
                config.server_addr
            )));
        }
        sleep(SERVER_START_RETRY_DELAY).await;
    }
}
