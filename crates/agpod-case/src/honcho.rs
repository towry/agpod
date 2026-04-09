//! Honcho v3 adapter for case event sync, semantic search, and context retrieval.
//!
//! Keywords: honcho, semantic search, vector digest, case context, hook sink

use crate::config::CaseConfig;
use crate::context::CaseContextProvider;
use crate::error::{CaseError, CaseResult};
use crate::events::CaseEventEnvelope;
use crate::hooks::{CaseEventSink, HookFuture};
use crate::search::{CaseSearchBackend, ContextScope, SearchFuture};
use crate::types::{CaseContextHit, CaseContextResult, CaseSearchResult};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{debug, info, warn};

const HONCHO_MAX_ATTEMPTS: usize = 3;
const HONCHO_RETRY_BASE_DELAY_MS: u64 = 200;

#[derive(Clone)]
pub struct HonchoBackend {
    http: reqwest::Client,
    base_url: String,
    workspace_id: String,
    api_key: String,
    peer_id: String,
}

impl HonchoBackend {
    pub fn from_config(config: &CaseConfig) -> CaseResult<Option<Self>> {
        if !config.honcho_enabled {
            debug!("honcho disabled in case config");
            return Ok(None);
        }

        let base_url = config
            .honcho_base_url
            .clone()
            .ok_or_else(|| CaseError::HonchoConfig("missing `honcho_base_url`".to_string()))?;
        let workspace_id = config
            .honcho_workspace_id
            .clone()
            .ok_or_else(|| CaseError::HonchoConfig("missing `honcho_workspace_id`".to_string()))?;
        let api_key = resolve_api_key(config)?;
        if config.honcho_peer_id.trim().is_empty() {
            warn!("honcho enabled but peer_id is empty");
            return Err(CaseError::HonchoConfig(
                "`honcho_peer_id` must not be empty".to_string(),
            ));
        }

        let http = reqwest::Client::builder()
            .build()
            .map_err(|err| CaseError::HonchoHttp(err.to_string()))?;

        debug!(
            base_url = %base_url,
            workspace_id = %workspace_id,
            peer_id = %config.honcho_peer_id,
            semantic_recall_enabled = config.semantic_recall_enabled,
            honcho_sync_enabled = config.honcho_sync_enabled,
            "honcho backend initialized"
        );

        Ok(Some(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            workspace_id,
            api_key,
            peer_id: config.honcho_peer_id.clone(),
        }))
    }

    async fn post_json<T, R>(&self, path: &str, body: &T) -> CaseResult<R>
    where
        T: Serialize + ?Sized,
        R: for<'de> Deserialize<'de>,
    {
        let body = serde_json::to_value(body).map_err(CaseError::Json)?;
        let url = format!("{}{}", self.base_url, path);
        self.request_with_retry(|client, api_key| {
            client
                .post(url.clone())
                .header(AUTHORIZATION, format!("Bearer {}", api_key))
                .header(CONTENT_TYPE, "application/json")
                .json(&body)
        })
        .await
    }

    async fn get_json<R>(&self, path: &str, query: &[(&str, String)]) -> CaseResult<R>
    where
        R: for<'de> Deserialize<'de>,
    {
        let query: Vec<(String, String)> = query
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect();
        let url = format!("{}{}", self.base_url, path);
        self.request_with_retry(|client, api_key| {
            client
                .get(url.clone())
                .header(AUTHORIZATION, format!("Bearer {}", api_key))
                .query(&query)
        })
        .await
    }

    async fn request_with_retry<R, F>(&self, build_request: F) -> CaseResult<R>
    where
        R: for<'de> Deserialize<'de>,
        F: Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder,
    {
        let mut last_error = None;
        for attempt in 1..=HONCHO_MAX_ATTEMPTS {
            let request = build_request(&self.http, &self.api_key)
                .build()
                .map_err(|err| CaseError::HonchoHttp(err.to_string()));

            let response = match request {
                Ok(request) => self
                    .http
                    .execute(request)
                    .await
                    .map_err(|err| CaseError::HonchoHttp(err.to_string())),
                Err(error) => Err(error),
            };

            match response {
                Ok(response) => match self.decode_response(response).await {
                    Ok(value) => return Ok(value),
                    Err(error) if self.should_retry_error(&error, attempt) => {
                        last_error = Some(error);
                    }
                    Err(error) => return Err(error),
                },
                Err(error) if self.should_retry_error(&error, attempt) => {
                    last_error = Some(error);
                }
                Err(error) => return Err(error),
            }

            let delay = HONCHO_RETRY_BASE_DELAY_MS.saturating_mul(attempt as u64);
            warn!(
                attempt,
                delay_ms = delay,
                "retrying transient honcho request"
            );
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        Err(last_error.unwrap_or_else(|| {
            CaseError::HonchoHttp("honcho request failed after retries".to_string())
        }))
    }

    async fn decode_response<R>(&self, response: reqwest::Response) -> CaseResult<R>
    where
        R: for<'de> Deserialize<'de>,
    {
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown Honcho error".to_string());
            warn!(status = %status, body = %body, "honcho API returned non-success status");
            return Err(CaseError::HonchoApi(format!("{status}: {body}")));
        }
        response
            .json::<R>()
            .await
            .map_err(|err| CaseError::HonchoHttp(err.to_string()))
    }

    fn should_retry_error(&self, error: &CaseError, attempt: usize) -> bool {
        if attempt >= HONCHO_MAX_ATTEMPTS {
            return false;
        }
        match error {
            CaseError::HonchoApi(message) => {
                message.starts_with("429")
                    || message.starts_with("502")
                    || message.starts_with("503")
                    || message.starts_with("504")
            }
            CaseError::HonchoHttp(_) => true,
            _ => false,
        }
    }

    async fn ensure_session(
        &self,
        case_id: &str,
        metadata: Value,
    ) -> CaseResult<HonchoSessionResponse> {
        self.post_json(
            &format!("/v3/workspaces/{}/sessions", self.workspace_id),
            &json!({
                "id": case_id,
                "metadata": metadata,
            }),
        )
        .await
    }

    async fn ensure_peer(&self) -> CaseResult<HonchoPeerResponse> {
        self.post_json(
            &format!("/v3/workspaces/{}/peers", self.workspace_id),
            &json!({
                "id": self.peer_id,
            }),
        )
        .await
    }

    async fn ensure_session_peer(&self, case_id: &str) -> CaseResult<HonchoSessionResponse> {
        self.post_json(
            &format!(
                "/v3/workspaces/{}/sessions/{}/peers",
                self.workspace_id, case_id
            ),
            &json!({
                (self.peer_id.clone()): {}
            }),
        )
        .await
    }

    async fn create_message(&self, event: &CaseEventEnvelope) -> CaseResult<Vec<HonchoMessage>> {
        self.post_json(
            &format!(
                "/v3/workspaces/{}/sessions/{}/messages",
                self.workspace_id, event.case_id
            ),
            &json!({
                "messages": [{
                    "content": event.event.honcho_content(),
                    "peer_id": self.peer_id,
                    "created_at": event.occurred_at,
                    "metadata": honcho_message_metadata(event)
                }]
            }),
        )
        .await
    }

    pub async fn sync_event(&self, event: &CaseEventEnvelope) -> CaseResult<()> {
        debug!(
            case_id = %event.case_id,
            event_id = %event.event_id,
            repo_id = %event.repo_id,
            "syncing case event to honcho"
        );
        let _peer = self.ensure_peer().await?;
        let _session = self
            .ensure_session(
                &event.case_id,
                json!({
                    "repo_id": event.repo_id,
                    "repo_label": event.repo_label,
                    "worktree_id": event.worktree_id,
                    "worktree_root": event.worktree_root,
                }),
            )
            .await?;
        let _session_peer = self.ensure_session_peer(&event.case_id).await?;
        let _message = self.create_message(event).await?;
        info!(case_id = %event.case_id, event_id = %event.event_id, "synced case event to honcho");
        Ok(())
    }

    async fn search_session_raw(
        &self,
        case_id: &str,
        query: &str,
        limit: usize,
    ) -> CaseResult<HonchoSearchResponse> {
        self.post_json(
            &format!(
                "/v3/workspaces/{}/sessions/{}/search",
                self.workspace_id, case_id
            ),
            &json!({
                "query": query,
                "limit": limit,
            }),
        )
        .await
    }

    async fn search_workspace_raw(
        &self,
        query: &str,
        limit: usize,
        repo_id: &str,
    ) -> CaseResult<HonchoSearchResponse> {
        self.post_json(
            &format!("/v3/workspaces/{}/search", self.workspace_id),
            &json!({
                "query": query,
                "limit": limit,
                "filters": {
                    "metadata": {
                        "repo_id": repo_id,
                    }
                }
            }),
        )
        .await
    }

    async fn get_context_raw(
        &self,
        case_id: &str,
        token_limit: Option<u32>,
    ) -> CaseResult<HonchoContextResponse> {
        let mut query = Vec::new();
        if let Some(token_limit) = token_limit {
            query.push(("tokens", token_limit.to_string()));
        }
        self.get_json(
            &format!(
                "/v3/workspaces/{}/sessions/{}/context",
                self.workspace_id, case_id
            ),
            &query,
        )
        .await
    }

    pub async fn get_repo_context(
        &self,
        repo_id: &str,
        query: Option<&str>,
        limit: usize,
        token_limit: Option<u32>,
    ) -> CaseResult<CaseContextResult> {
        debug!(
            repo_id,
            query = query.unwrap_or_default(),
            limit,
            token_limit = token_limit.unwrap_or_default(),
            "requesting honcho repo context"
        );
        let hits = match query {
            Some(query) if !query.trim().is_empty() => {
                let response = self.search_workspace_raw(query, limit, repo_id).await?;
                response
                    .into_iter()
                    .map(|hit| CaseContextHit {
                        case_id: hit.session_id.clone(),
                        source: hit.source.unwrap_or_else(|| "honcho".to_string()),
                        field: hit.field.unwrap_or_else(|| "content".to_string()),
                        excerpt: hit.content,
                        score: hit.score.unwrap_or(0.0) as i64,
                        direction_seq: hit
                            .metadata
                            .get("direction_seq")
                            .and_then(Value::as_u64)
                            .map(|value| value as u32),
                        entry_seq: hit
                            .metadata
                            .get("entry_seq")
                            .and_then(Value::as_u64)
                            .map(|value| value as u32),
                        step_id: hit
                            .metadata
                            .get("step_id")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        kind: hit
                            .metadata
                            .get("kind")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                    })
                    .collect()
            }
            _ => Vec::new(),
        };

        let mut lines = Vec::new();
        lines.push(format!(
            "Repository {repo_id} cross-session context from Honcho"
        ));
        if hits.is_empty() {
            lines.push("No semantic hits found.".to_string());
        } else {
            lines.push("Relevant semantic hits:".to_string());
            for hit in &hits {
                let case_id = hit.case_id.as_deref().unwrap_or("?");
                lines.push(format!(
                    "- case {} {}.{}: {}",
                    case_id, hit.source, hit.field, hit.excerpt
                ));
            }
        }

        let mut context = lines.join("\n");
        if let Some(limit) = token_limit {
            let char_limit = (limit as usize).saturating_mul(4);
            if context.len() > char_limit {
                context.truncate(char_limit);
            }
        }

        Ok(CaseContextResult {
            backend: <Self as CaseContextProvider>::backend_name(self).to_string(),
            scope: "repo".to_string(),
            case_id: None,
            repo_id: Some(repo_id.to_string()),
            query: query.map(ToOwned::to_owned),
            token_limit,
            generated_at: chrono::Utc::now().to_rfc3339(),
            context,
            hits,
        })
    }
}

fn honcho_message_metadata(event: &CaseEventEnvelope) -> Value {
    let mut metadata = event.event.metadata();
    metadata.insert(
        "event_id".to_string(),
        Value::String(event.event_id.clone()),
    );
    metadata.insert(
        "event_type".to_string(),
        Value::String(event.event.event_type().to_string()),
    );
    metadata.insert(
        "session_id".to_string(),
        Value::String(event.case_id.clone()),
    );
    metadata.insert("repo_id".to_string(), Value::String(event.repo_id.clone()));
    if let Some(case_id) = event.associated_case_id.as_ref() {
        metadata.insert(
            "associated_case_id".to_string(),
            Value::String(case_id.clone()),
        );
    }
    if !metadata.contains_key("direction_seq") {
        if let Some(direction_seq) = event.direction_seq {
            metadata.insert("direction_seq".to_string(), json!(direction_seq));
        }
    }

    metadata.retain(|_, value| !value.is_null());
    Value::Object(metadata)
}

fn resolve_api_key(config: &CaseConfig) -> CaseResult<String> {
    if let Some(api_key) = config
        .honcho_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        debug!("using inline honcho api key from config");
        return Ok(api_key.to_string());
    }

    let api_key_env = config.honcho_api_key_env.trim();
    if api_key_env.is_empty() {
        warn!("honcho api key missing: inline key absent and env name empty");
        return Err(CaseError::HonchoConfig(
            "missing `honcho_api_key` or non-empty `honcho_api_key_env`".to_string(),
        ));
    }

    debug!(api_key_env, "using honcho api key from environment");
    std::env::var(api_key_env).map_err(|_| {
        warn!(api_key_env, "honcho api key env var not set");
        CaseError::HonchoConfig(format!(
            "missing `honcho_api_key` and env var `{api_key_env}` for Honcho API key"
        ))
    })
}

impl CaseEventSink for HonchoBackend {
    fn name(&self) -> &'static str {
        "honcho"
    }

    fn handle<'a>(&'a self, event: &'a CaseEventEnvelope) -> HookFuture<'a> {
        Box::pin(async move { self.sync_event(event).await })
    }
}

impl CaseSearchBackend for HonchoBackend {
    fn backend_name(&self) -> &'static str {
        "honcho"
    }

    fn recall_cases<'a>(&'a self, _query: &'a str) -> SearchFuture<'a, Vec<CaseSearchResult>> {
        Box::pin(async move {
            Err(CaseError::SemanticBackendUnavailable(
                "Honcho workspace-wide recall is not wired yet; use local case recall".to_string(),
            ))
        })
    }

    fn search_case<'a>(
        &'a self,
        case_id: &'a str,
        query: &'a str,
        limit: usize,
    ) -> SearchFuture<'a, Vec<CaseContextHit>> {
        Box::pin(async move {
            let response = self.search_session_raw(case_id, query, limit).await?;
            let hits = response
                .into_iter()
                .map(|hit| CaseContextHit {
                    case_id: hit.session_id.clone(),
                    source: hit.source.unwrap_or_else(|| "honcho".to_string()),
                    field: hit.field.unwrap_or_else(|| "content".to_string()),
                    excerpt: hit.content,
                    score: hit.score.unwrap_or(0.0) as i64,
                    direction_seq: hit
                        .metadata
                        .get("direction_seq")
                        .and_then(Value::as_u64)
                        .map(|value| value as u32),
                    entry_seq: hit
                        .metadata
                        .get("entry_seq")
                        .and_then(Value::as_u64)
                        .map(|value| value as u32),
                    step_id: hit
                        .metadata
                        .get("step_id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    kind: hit
                        .metadata
                        .get("kind")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                })
                .collect();
            Ok(hits)
        })
    }

    fn search_repo<'a>(
        &'a self,
        _query: &'a str,
        _limit: usize,
    ) -> SearchFuture<'a, Vec<CaseContextHit>> {
        Box::pin(async move {
            Err(CaseError::SemanticBackendUnavailable(
                "Honcho repo search requires repo-aware provider invocation".to_string(),
            ))
        })
    }
}

impl CaseContextProvider for HonchoBackend {
    fn backend_name(&self) -> &'static str {
        "honcho"
    }

    fn get_context<'a>(
        &'a self,
        scope: ContextScope<'a>,
        query: Option<&'a str>,
        limit: usize,
        token_limit: Option<u32>,
    ) -> SearchFuture<'a, CaseContextResult> {
        Box::pin(async move {
            match scope {
                ContextScope::Case { case_id } => {
                    let context = self.get_context_raw(case_id, token_limit).await?;
                    let hits = match query {
                        Some(query) if !query.trim().is_empty() => {
                            self.search_case(case_id, query, limit).await?
                        }
                        _ => Vec::new(),
                    };

                    Ok(CaseContextResult {
                        backend: <Self as CaseContextProvider>::backend_name(self).to_string(),
                        scope: "case".to_string(),
                        case_id: Some(case_id.to_string()),
                        repo_id: context.messages.iter().find_map(|message| {
                            message
                                .metadata
                                .get("repo_id")
                                .and_then(Value::as_str)
                                .map(ToOwned::to_owned)
                        }),
                        query: query.map(ToOwned::to_owned),
                        token_limit,
                        generated_at: chrono::Utc::now().to_rfc3339(),
                        context: context.rendered_text(),
                        hits,
                    })
                }
                ContextScope::Repo => Err(CaseError::ContextProviderUnavailable(
                    "Honcho repo scope requires repo-aware context provider".to_string(),
                )),
            }
        })
    }
}

#[derive(Debug, Deserialize)]
struct HonchoSessionResponse {
    #[allow(dead_code)]
    id: Option<String>,
}

type HonchoSearchResponse = Vec<HonchoSearchHit>;

#[derive(Debug, Deserialize)]
struct HonchoSearchHit {
    content: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    field: Option<String>,
    #[serde(default)]
    metadata: Value,
}

#[derive(Debug, Deserialize)]
struct HonchoContextResponse {
    #[serde(default)]
    messages: Vec<HonchoMessage>,
    #[serde(default)]
    summary: Option<HonchoSummary>,
    #[serde(default)]
    peer_representation: Option<String>,
    #[serde(default)]
    peer_card: Option<Vec<String>>,
}

impl HonchoContextResponse {
    fn rendered_text(&self) -> String {
        let mut sections = Vec::new();
        if let Some(summary) = self
            .summary
            .as_ref()
            .and_then(|summary| summary.content.clone())
        {
            let summary = summary.trim();
            if !summary.is_empty() {
                sections.push(format!("Summary:\n{summary}"));
            }
        }

        if !self.messages.is_empty() {
            let mut lines = Vec::with_capacity(self.messages.len() + 1);
            lines.push("Messages:".to_string());
            for message in &self.messages {
                let peer_id = message.peer_id.as_deref().unwrap_or("unknown");
                lines.push(format!("- {peer_id}: {}", message.content));
            }
            sections.push(lines.join("\n"));
        }

        if let Some(peer_representation) = self.peer_representation.as_deref() {
            let peer_representation = peer_representation.trim();
            if !peer_representation.is_empty() {
                sections.push(format!("Peer Representation:\n{peer_representation}"));
            }
        }

        if let Some(peer_card) = self.peer_card.as_ref().filter(|card| !card.is_empty()) {
            let mut lines = Vec::with_capacity(peer_card.len() + 1);
            lines.push("Peer Card:".to_string());
            for fact in peer_card {
                lines.push(format!("- {fact}"));
            }
            sections.push(lines.join("\n"));
        }

        sections.join("\n\n")
    }
}

#[derive(Debug, Deserialize)]
struct HonchoPeerResponse {
    #[allow(dead_code)]
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HonchoMessage {
    content: String,
    #[serde(default)]
    peer_id: Option<String>,
    #[serde(default)]
    metadata: Value,
}

#[derive(Debug, Deserialize)]
struct HonchoSummary {
    #[serde(default)]
    content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{
        honcho_message_metadata, resolve_api_key, HonchoBackend, HonchoContextResponse,
        HonchoSearchResponse, HONCHO_MAX_ATTEMPTS,
    };
    use crate::config::CaseConfig;
    use crate::error::CaseError;
    use crate::events::{CaseDomainEvent, CaseEventEnvelope};
    use crate::types::{Case, CaseStatus, Constraint, Entry, EntryType};
    use serde_json::{json, Value};
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn resolve_api_key_prefers_direct_config_value() {
        let config = CaseConfig {
            honcho_api_key: Some(" direct-secret ".to_string()),
            honcho_api_key_env: "HONCHO_UNUSED".to_string(),
            ..CaseConfig::default()
        };

        let api_key = resolve_api_key(&config).expect("direct key should resolve");
        assert_eq!(api_key, "direct-secret");
    }

    #[test]
    fn resolve_api_key_uses_env_when_direct_value_missing() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        std::env::set_var("HONCHO_TEST_ENV_KEY", "env-secret");

        let config = CaseConfig {
            honcho_api_key_env: "HONCHO_TEST_ENV_KEY".to_string(),
            ..CaseConfig::default()
        };

        let api_key = resolve_api_key(&config).expect("env key should resolve");
        assert_eq!(api_key, "env-secret");

        std::env::remove_var("HONCHO_TEST_ENV_KEY");
    }

    #[test]
    fn resolve_api_key_fails_when_both_sources_missing() {
        let config = CaseConfig {
            honcho_api_key: None,
            honcho_api_key_env: " ".to_string(),
            ..CaseConfig::default()
        };

        let error = resolve_api_key(&config).expect_err("missing key should fail");
        assert_eq!(
            error.to_string(),
            "honcho config error: missing `honcho_api_key` or non-empty `honcho_api_key_env`"
        );
    }

    #[test]
    fn honcho_context_response_renders_summary_and_messages() {
        let response: HonchoContextResponse = serde_json::from_value(json!({
            "id": "case-1",
            "messages": [
                {
                    "content": "opened case",
                    "peer_id": "agpod-system",
                    "session_id": "case-1",
                    "metadata": { "repo_id": "repo-1" }
                }
            ],
            "summary": {
                "content": "brief summary"
            }
        }))
        .expect("context response should deserialize");

        let rendered = response.rendered_text();
        assert!(rendered.contains("brief summary"));
        assert!(rendered.contains("agpod-system: opened case"));
    }

    #[test]
    fn honcho_search_response_accepts_v3_array_shape() {
        let response: HonchoSearchResponse = serde_json::from_value(json!([
            {
                "content": "opened case",
                "peer_id": "agpod-system",
                "session_id": "case-1",
                "metadata": {
                    "direction_seq": 1,
                    "entry_seq": 2,
                    "kind": "finding"
                }
            }
        ]))
        .expect("search response should deserialize");

        assert_eq!(response.len(), 1);
        assert_eq!(response[0].session_id.as_deref(), Some("case-1"));
        assert_eq!(
            response[0].metadata.get("kind").and_then(Value::as_str),
            Some("finding")
        );
    }

    #[test]
    fn honcho_retry_policy_retries_transient_errors_only() {
        let backend = HonchoBackend {
            http: reqwest::Client::new(),
            base_url: "https://api.honcho.dev".to_string(),
            workspace_id: "ws".to_string(),
            api_key: "key".to_string(),
            peer_id: "peer".to_string(),
        };

        assert!(backend.should_retry_error(&CaseError::HonchoHttp("network".to_string()), 1));
        assert!(backend.should_retry_error(
            &CaseError::HonchoApi("429 Too Many Requests".to_string()),
            1
        ));
        assert!(
            !backend.should_retry_error(&CaseError::HonchoApi("400 Bad Request".to_string()), 1)
        );
        assert!(!backend.should_retry_error(
            &CaseError::HonchoHttp("network".to_string()),
            HONCHO_MAX_ATTEMPTS
        ));
    }

    #[test]
    fn honcho_message_metadata_is_flat_and_non_null() {
        let event = CaseEventEnvelope {
            event_id: "evt-1".to_string(),
            case_id: "C-1".to_string(),
            associated_case_id: Some("C-1".to_string()),
            repo_id: "repo-1".to_string(),
            repo_label: "github.com/example/repo".to_string(),
            worktree_id: "wt-1".to_string(),
            worktree_root: "/tmp/repo".to_string(),
            direction_seq: Some(2),
            occurred_at: "2026-03-25T08:00:00Z".to_string(),
            event: CaseDomainEvent::RecordAppended {
                case: sample_case(),
                entry: sample_entry(
                    7,
                    Some("finding"),
                    "Found the recall regression trigger in fresh Honcho sync payloads.",
                ),
            },
        };

        let metadata = honcho_message_metadata(&event)
            .as_object()
            .cloned()
            .expect("metadata should be an object");

        assert_eq!(
            metadata.get("event_type").and_then(Value::as_str),
            Some("record_appended")
        );
        assert_eq!(
            metadata.get("repo_id").and_then(Value::as_str),
            Some("repo-1")
        );
        assert_eq!(
            metadata.get("session_id").and_then(Value::as_str),
            Some("C-1")
        );
        assert_eq!(metadata.get("case_id").and_then(Value::as_str), Some("C-1"));
        assert_eq!(
            metadata.get("associated_case_id").and_then(Value::as_str),
            Some("C-1")
        );
        assert_eq!(metadata.get("entry_seq").and_then(Value::as_u64), Some(7));
        assert_eq!(
            metadata.get("kind").and_then(Value::as_str),
            Some("finding")
        );
        assert_eq!(
            metadata.get("direction_seq").and_then(Value::as_u64),
            Some(2)
        );
        assert!(!metadata.contains_key("event"));
        assert!(!metadata.contains_key("repo_label"));
        assert!(!metadata.contains_key("worktree_root"));
        assert!(!metadata.values().any(Value::is_null));
    }

    #[test]
    fn honcho_message_metadata_drops_null_optional_fields() {
        let event = CaseEventEnvelope {
            event_id: "evt-2".to_string(),
            case_id: "C-1".to_string(),
            associated_case_id: Some("C-1".to_string()),
            repo_id: "repo-1".to_string(),
            repo_label: "github.com/example/repo".to_string(),
            worktree_id: "wt-1".to_string(),
            worktree_root: "/tmp/repo".to_string(),
            direction_seq: Some(2),
            occurred_at: "2026-03-25T08:00:00Z".to_string(),
            event: CaseDomainEvent::RecordAppended {
                case: sample_case(),
                entry: sample_entry(8, None, "A summary without record kind."),
            },
        };

        let metadata = honcho_message_metadata(&event)
            .as_object()
            .cloned()
            .expect("metadata should be an object");

        assert!(!metadata.contains_key("kind"));
        assert!(!metadata.values().any(Value::is_null));
    }

    fn sample_case() -> Case {
        Case {
            id: "C-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_label: Some("github.com/example/repo".to_string()),
            worktree_id: Some("wt-1".to_string()),
            worktree_root: Some("/tmp/repo".to_string()),
            goal: "Investigate Honcho recall behavior".to_string(),
            goal_constraints: vec![Constraint {
                rule: "evidence-first".to_string(),
                reason: None,
            }],
            status: CaseStatus::Open,
            current_direction_seq: 2,
            current_step_id: Some("C-1/S-1".to_string()),
            opened_at: "2026-03-25T08:00:00Z".to_string(),
            updated_at: "2026-03-25T08:00:00Z".to_string(),
            closed_at: None,
            close_summary: None,
            abandoned_at: None,
            abandon_summary: None,
            close_confirm_token: None,
            close_confirm_action: None,
            close_confirm_summary: None,
        }
    }

    fn sample_entry(seq: u32, kind: Option<&str>, summary: &str) -> Entry {
        Entry {
            case_id: "C-1".to_string(),
            seq,
            entry_type: EntryType::Record,
            kind: kind.map(ToOwned::to_owned),
            step_id: None,
            summary: summary.to_string(),
            reason: None,
            context: None,
            files: Vec::new(),
            artifacts: Vec::new(),
            created_at: "2026-03-25T08:00:00Z".to_string(),
        }
    }
}
