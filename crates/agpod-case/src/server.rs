//! Single-instance local/remote case server.
//!
//! Keywords: case server, tcp server, single writer, remote ready

use crate::client::CaseClient;
use crate::commands::execute_command_json;
use crate::config::CaseConfig;
use crate::error::{CaseError, CaseResult};
use crate::repo_id::RepoIdentity;
use crate::rpc::{CaseRequest, CaseResponse};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

#[derive(Clone)]
pub struct CaseServer {
    listener_addr: String,
    base_client: CaseClient,
    db_gate: Arc<Mutex<()>>,
}

impl CaseServer {
    pub async fn new(config: CaseConfig) -> CaseResult<Self> {
        let base_client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "__bootstrap__".to_string(),
                repo_label: "__bootstrap__".to_string(),
                worktree_id: "__bootstrap__".to_string(),
                worktree_root: "__bootstrap__".to_string(),
            },
        )
        .await?;

        Ok(Self {
            listener_addr: config.server_addr,
            base_client,
            db_gate: Arc::new(Mutex::new(())),
        })
    }

    pub async fn serve(self) -> CaseResult<()> {
        let listener = TcpListener::bind(&self.listener_addr)
            .await
            .map_err(CaseError::Io)?;
        info!(listener_addr = %self.listener_addr, "case-server listening");

        loop {
            let (stream, _) = listener.accept().await.map_err(CaseError::Io)?;
            let server = self.clone();
            tokio::spawn(async move {
                if let Err(error) = server.handle_connection(stream).await {
                    warn!(error = %error, "case-server connection handling failed");
                }
            });
        }
    }

    async fn handle_connection(&self, stream: TcpStream) -> CaseResult<()> {
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = BufReader::new(read_half).lines();

        while let Some(line) = lines.next_line().await.map_err(CaseError::Io)? {
            if line.trim().is_empty() {
                continue;
            }
            debug!("case-server received request line");
            let request: CaseRequest = serde_json::from_str(&line).map_err(CaseError::Json)?;
            let response = self.handle_request(request).await;
            let payload = serde_json::to_string(&response).map_err(CaseError::Json)?;
            write_half
                .write_all(payload.as_bytes())
                .await
                .map_err(CaseError::Io)?;
            write_half.write_all(b"\n").await.map_err(CaseError::Io)?;
        }

        Ok(())
    }

    async fn handle_request(&self, request: CaseRequest) -> CaseResponse {
        let identity: RepoIdentity = request.repo.into();
        debug!(repo_id = %identity.repo_id, "handling case-server request");
        let _guard = self.db_gate.lock().await;
        let client = self.base_client.clone_with_identity(identity);

        let result = crate::commands::finish_json_value(
            execute_command_json(&client, &request.command).await,
            &client,
            &request.command,
            true,
        )
        .await;

        if result.get("ok").and_then(|value| value.as_bool()) == Some(false) {
            if let Some(message) = result.get("message").and_then(|value| value.as_str()) {
                warn!(message, "case-server request completed with error payload");
            } else {
                warn!("case-server request completed with error payload");
            }
        } else {
            debug!("case-server request completed successfully");
        }

        CaseResponse { result }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::CaseCommand;
    use crate::config::CaseOverrides;
    use crate::rpc::RepoIdentityPayload;
    use tempfile::TempDir;

    fn temp_config(temp_dir: &TempDir) -> CaseConfig {
        CaseConfig::load(CaseOverrides {
            data_dir: Some(
                temp_dir
                    .path()
                    .join("case.db")
                    .to_str()
                    .expect("temp db path should be valid utf-8"),
            ),
            server_addr: Some("127.0.0.1:6160"),
        })
    }

    fn repo_payload(name: &str, root: &str) -> RepoIdentityPayload {
        RepoIdentityPayload {
            repo_id: format!("{name}-repo-id"),
            repo_label: format!("github.com/smoke/{name}"),
            worktree_id: format!("{name}-wt"),
            worktree_root: root.to_string(),
        }
    }

    #[tokio::test]
    async fn server_reuses_single_db_client_across_repositories() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let server = CaseServer::new(temp_config(&temp_dir))
            .await
            .expect("server should initialize");

        let repo_a = CaseRequest {
            repo: repo_payload("repo-a", "/tmp/repo-a"),
            command: CaseCommand::Open {
                mode: crate::cli::OpenModeArg::New,
                case_id: None,
                goal: Some("goal a".to_string()),
                direction: Some("dir a".to_string()),
                goal_constraints: vec![],
                constraints: vec![],
                success_condition: None,
                abort_condition: None,
            },
        };
        let repo_b = CaseRequest {
            repo: repo_payload("repo-b", "/tmp/repo-b"),
            command: CaseCommand::Open {
                mode: crate::cli::OpenModeArg::New,
                case_id: None,
                goal: Some("goal b".to_string()),
                direction: Some("dir b".to_string()),
                goal_constraints: vec![],
                constraints: vec![],
                success_condition: None,
                abort_condition: None,
            },
        };

        let result_a = server.handle_request(repo_a).await.result;
        let result_b = server.handle_request(repo_b).await.result;

        assert_eq!(result_a["ok"].as_bool(), Some(true));
        assert_eq!(result_b["ok"].as_bool(), Some(true));
        assert_ne!(result_a["case"]["id"], result_b["case"]["id"]);
    }

    #[tokio::test]
    async fn server_handles_concurrent_open_requests_for_different_repositories() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let server = CaseServer::new(temp_config(&temp_dir))
            .await
            .expect("server should initialize");

        let repo_a = CaseRequest {
            repo: repo_payload("repo-a", "/tmp/repo-a"),
            command: CaseCommand::Open {
                mode: crate::cli::OpenModeArg::New,
                case_id: None,
                goal: Some("goal a".to_string()),
                direction: Some("dir a".to_string()),
                goal_constraints: vec![],
                constraints: vec![],
                success_condition: None,
                abort_condition: None,
            },
        };
        let repo_b = CaseRequest {
            repo: repo_payload("repo-b", "/tmp/repo-b"),
            command: CaseCommand::Open {
                mode: crate::cli::OpenModeArg::New,
                case_id: None,
                goal: Some("goal b".to_string()),
                direction: Some("dir b".to_string()),
                goal_constraints: vec![],
                constraints: vec![],
                success_condition: None,
                abort_condition: None,
            },
        };

        let (result_a, result_b) =
            tokio::join!(server.handle_request(repo_a), server.handle_request(repo_b),);

        assert_eq!(result_a.result["ok"].as_bool(), Some(true));
        assert_eq!(result_b.result["ok"].as_bool(), Some(true));
        assert_ne!(result_a.result["case"]["id"], result_b.result["case"]["id"]);
    }
}
