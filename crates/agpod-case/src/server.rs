//! Single-instance local/remote case server.
//!
//! Keywords: case server, tcp server, single writer, remote ready

use crate::cli::{CaseCommand, StepCommand};
use crate::client::CaseClient;
use crate::config::CaseConfig;
use crate::error::{CaseError, CaseResult};
use crate::output;
use crate::repo_id::RepoIdentity;
use crate::rpc::{CaseRequest, CaseResponse};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration, Instant};
use tracing::{debug, info, warn};

const SLOW_SERVER_WAIT_WARN_MS: u128 = 1_000;
const SLOW_SERVER_EXEC_WARN_MS: u128 = 1_000;

#[derive(Clone)]
pub struct CaseServer {
    listener_addr: String,
    base_client: CaseClient,
    write_gate: Arc<Mutex<()>>,
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
            write_gate: Arc::new(Mutex::new(())),
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
        self.handle_request_with_timeout(request, crate::CASE_REQUEST_TIMEOUT)
            .await
    }

    async fn handle_request_with_timeout(
        &self,
        request: CaseRequest,
        timeout_limit: Duration,
    ) -> CaseResponse {
        let identity: RepoIdentity = request.repo.into();
        debug!(repo_id = %identity.repo_id, "handling case-server request");
        let command_debug = format!("{:?}", request.command);
        let client = self.base_client.clone_with_identity(identity);
        let exec_started = Instant::now();

        let result = if command_requires_write_gate(&request.command) {
            let queued_at = Instant::now();
            let _guard = match timeout(timeout_limit, self.write_gate.lock()).await {
                Ok(guard) => guard,
                Err(_) => {
                    warn!(
                        repo_id = %client.repo_id(),
                        wait_ms = timeout_limit.as_millis(),
                        command = %command_debug,
                        "case-server request timed out waiting on write gate"
                    );
                    return CaseResponse {
                        result: output::error_json(
                            "error",
                            &format!(
                                "case-server request timed out waiting for execution slot after {} ms",
                                timeout_limit.as_millis()
                            ),
                            None,
                        ),
                    };
                }
            };
            let wait_elapsed = queued_at.elapsed();
            if wait_elapsed.as_millis() >= SLOW_SERVER_WAIT_WARN_MS {
                warn!(
                    repo_id = %client.repo_id(),
                    wait_ms = wait_elapsed.as_millis(),
                    command = %command_debug,
                    "case-server request waited on write gate"
                );
            }
            crate::commands::finish_json_value(
                crate::commands::execute_command_json(&client, &request.command).await,
                &client,
                &request.command,
                true,
            )
            .await
        } else {
            crate::commands::finish_json_value(
                crate::commands::execute_command_json(&client, &request.command).await,
                &client,
                &request.command,
                true,
            )
            .await
        };
        let exec_elapsed = exec_started.elapsed();
        if exec_elapsed.as_millis() >= SLOW_SERVER_EXEC_WARN_MS {
            warn!(
                repo_id = %client.repo_id(),
                elapsed_ms = exec_elapsed.as_millis(),
                command = %command_debug,
                "case-server request executed slowly"
            );
        }

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

fn command_requires_write_gate(command: &CaseCommand) -> bool {
    match command {
        CaseCommand::Current { .. }
        | CaseCommand::Show { .. }
        | CaseCommand::List { .. }
        | CaseCommand::Recall { .. }
        | CaseCommand::Context { .. } => false,
        CaseCommand::Step { command } => matches!(
            command,
            StepCommand::Add { .. }
                | StepCommand::Start { .. }
                | StepCommand::Done { .. }
                | StepCommand::Move { .. }
                | StepCommand::Block { .. }
                | StepCommand::Advance { .. }
        ),
        CaseCommand::Open { .. }
        | CaseCommand::SessionRecord { .. }
        | CaseCommand::Decide { .. }
        | CaseCommand::Redirect { .. }
        | CaseCommand::Close { .. }
        | CaseCommand::Abandon { .. } => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::CaseCommand;
    use crate::config::CaseOverrides;
    use crate::rpc::RepoIdentityPayload;
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::time::Duration;

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

    #[test]
    fn case_request_accepts_legacy_open_payload_without_startup_context_fields() {
        let request: CaseRequest = serde_json::from_value(json!({
            "repo": repo_payload("legacy", "/tmp/legacy"),
            "command": {
                "Open": {
                    "mode": "new",
                    "case_id": null,
                    "goal": "legacy client goal",
                    "direction": "legacy client direction",
                    "goal_constraints": [],
                    "constraints": [],
                    "success_condition": null,
                    "abort_condition": null
                }
            }
        }))
        .expect("legacy open RPC payload should deserialize");

        match request.command {
            CaseCommand::Open {
                how_to,
                doc_about,
                pitfalls_about,
                known_patterns_for,
                steps,
                ..
            } => {
                assert!(how_to.is_empty());
                assert!(doc_about.is_empty());
                assert!(pitfalls_about.is_empty());
                assert!(known_patterns_for.is_empty());
                assert!(steps.is_empty());
            }
            other => panic!("unexpected command: {other:?}"),
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
                how_to: vec![],
                doc_about: vec![],
                pitfalls_about: vec![],
                known_patterns_for: vec![],
                steps: vec![],
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
                how_to: vec![],
                doc_about: vec![],
                pitfalls_about: vec![],
                known_patterns_for: vec![],
                steps: vec![],
            },
        };

        let result_a = server
            .handle_request_with_timeout(repo_a, Duration::from_secs(15))
            .await
            .result;
        let result_b = server
            .handle_request_with_timeout(repo_b, Duration::from_secs(15))
            .await
            .result;

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
                how_to: vec![],
                doc_about: vec![],
                pitfalls_about: vec![],
                known_patterns_for: vec![],
                steps: vec![],
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
                how_to: vec![],
                doc_about: vec![],
                pitfalls_about: vec![],
                known_patterns_for: vec![],
                steps: vec![],
            },
        };

        let (result_a, result_b) = tokio::join!(
            server.handle_request_with_timeout(repo_a, Duration::from_secs(15)),
            server.handle_request_with_timeout(repo_b, Duration::from_secs(15)),
        );

        assert_eq!(result_a.result["ok"].as_bool(), Some(true));
        assert_eq!(result_b.result["ok"].as_bool(), Some(true));
        assert_ne!(result_a.result["case"]["id"], result_b.result["case"]["id"]);
    }

    #[tokio::test]
    async fn server_read_request_bypasses_write_gate() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let server = CaseServer::new(temp_config(&temp_dir))
            .await
            .expect("server should initialize");
        let _guard = server.write_gate.lock().await;

        let request = CaseRequest {
            repo: repo_payload("repo-a", "/tmp/repo-a"),
            command: CaseCommand::Current { state: false },
        };

        let started = Instant::now();
        let response = server
            .handle_request_with_timeout(request, Duration::from_millis(5))
            .await;

        assert_eq!(response.result["ok"].as_bool(), Some(false));
        assert_eq!(
            response.result["message"].as_str(),
            Some("no open case in this repository")
        );
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn server_write_request_still_times_out_when_write_gate_is_held() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let server = CaseServer::new(temp_config(&temp_dir))
            .await
            .expect("server should initialize");
        let _guard = server.write_gate.lock().await;

        let request = CaseRequest {
            repo: repo_payload("repo-a", "/tmp/repo-a"),
            command: CaseCommand::Current { state: true },
        };

        assert!(!command_requires_write_gate(&request.command));

        let write_request = CaseRequest {
            repo: repo_payload("repo-a", "/tmp/repo-a"),
            command: CaseCommand::Open {
                mode: crate::cli::OpenModeArg::New,
                case_id: None,
                goal: Some("goal".to_string()),
                direction: Some("direction".to_string()),
                goal_constraints: vec![],
                constraints: vec![],
                success_condition: None,
                abort_condition: None,
                how_to: vec![],
                doc_about: vec![],
                pitfalls_about: vec![],
                known_patterns_for: vec![],
                steps: vec![],
            },
        };

        let started = Instant::now();
        let response = server
            .handle_request_with_timeout(write_request, Duration::from_millis(5))
            .await;

        assert_eq!(response.result["ok"].as_bool(), Some(false));
        assert!(response.result["message"]
            .as_str()
            .is_some_and(|message| message.contains("timed out waiting for execution slot")));
        assert!(started.elapsed() >= Duration::from_millis(5));
    }
}
