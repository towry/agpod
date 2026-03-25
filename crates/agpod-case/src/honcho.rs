//! Honcho v2 adapter for case event sync, semantic search, and context retrieval.
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
            return Err(CaseError::HonchoConfig(
                "`honcho_peer_id` must not be empty".to_string(),
            ));
        }

        let http = reqwest::Client::builder()
            .build()
            .map_err(|err| CaseError::HonchoHttp(err.to_string()))?;

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
        let response = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await
            .map_err(|err| CaseError::HonchoHttp(err.to_string()))?;
        self.decode_response(response).await
    }

    async fn get_json<R>(&self, path: &str, query: &[(&str, String)]) -> CaseResult<R>
    where
        R: for<'de> Deserialize<'de>,
    {
        let response = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .query(query)
            .send()
            .await
            .map_err(|err| CaseError::HonchoHttp(err.to_string()))?;
        self.decode_response(response).await
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
            return Err(CaseError::HonchoApi(format!("{status}: {body}")));
        }
        response
            .json::<R>()
            .await
            .map_err(|err| CaseError::HonchoHttp(err.to_string()))
    }

    async fn ensure_session(
        &self,
        case_id: &str,
        metadata: Value,
    ) -> CaseResult<HonchoSessionResponse> {
        self.post_json(
            &format!("/v2/workspaces/{}/sessions", self.workspace_id),
            &json!({
                "session_id": case_id,
                "metadata": metadata,
            }),
        )
        .await
    }

    async fn create_message(&self, event: &CaseEventEnvelope) -> CaseResult<Value> {
        let event_metadata = event.event.metadata();
        self.post_json(
            &format!(
                "/v2/workspaces/{}/sessions/{}/messages",
                self.workspace_id, event.case_id
            ),
            &json!({
                "content": event.event.summary_text(),
                "peer_id": self.peer_id,
                "created_at": event.occurred_at,
                "metadata": {
                    "event_id": event.event_id,
                    "event_type": event.event.event_type(),
                    "repo_id": event.repo_id,
                    "repo_label": event.repo_label,
                    "worktree_id": event.worktree_id,
                    "worktree_root": event.worktree_root,
                    "direction_seq": event.direction_seq,
                    "entry_seq": event_metadata.get("entry_seq").cloned(),
                    "step_id": event_metadata.get("step_id").cloned(),
                    "kind": event_metadata.get("kind").cloned(),
                    "event": event_metadata,
                },
                "configuration": {
                    "deriver": {
                        "enabled": true
                    }
                }
            }),
        )
        .await
    }

    pub async fn sync_event(&self, event: &CaseEventEnvelope) -> CaseResult<()> {
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
        let _message = self.create_message(event).await?;
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
                "/v2/workspaces/{}/sessions/{}/search",
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
            &format!("/v2/workspaces/{}/search", self.workspace_id),
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
            query.push(("token_limit", token_limit.to_string()));
        }
        self.get_json(
            &format!(
                "/v2/workspaces/{}/sessions/{}/context",
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
        let hits = match query {
            Some(query) if !query.trim().is_empty() => {
                let response = self.search_workspace_raw(query, limit, repo_id).await?;
                response
                    .results
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

fn resolve_api_key(config: &CaseConfig) -> CaseResult<String> {
    if let Some(api_key) = config
        .honcho_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(api_key.to_string());
    }

    let api_key_env = config.honcho_api_key_env.trim();
    if api_key_env.is_empty() {
        return Err(CaseError::HonchoConfig(
            "missing `honcho_api_key` or non-empty `honcho_api_key_env`".to_string(),
        ));
    }

    std::env::var(api_key_env).map_err(|_| {
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
                .results
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
                        repo_id: context
                            .metadata
                            .get("repo_id")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        query: query.map(ToOwned::to_owned),
                        token_limit,
                        generated_at: chrono::Utc::now().to_rfc3339(),
                        context: context.rendered_context,
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

#[derive(Debug, Deserialize)]
struct HonchoSearchResponse {
    #[serde(default)]
    results: Vec<HonchoSearchHit>,
}

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
    #[serde(default, alias = "context")]
    rendered_context: String,
    #[serde(default)]
    metadata: Value,
}

#[cfg(test)]
mod tests {
    use super::resolve_api_key;
    use crate::config::CaseConfig;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn resolve_api_key_prefers_direct_config_value() {
        let mut config = CaseConfig::default();
        config.honcho_api_key = Some(" direct-secret ".to_string());
        config.honcho_api_key_env = "HONCHO_UNUSED".to_string();

        let api_key = resolve_api_key(&config).expect("direct key should resolve");
        assert_eq!(api_key, "direct-secret");
    }

    #[test]
    fn resolve_api_key_uses_env_when_direct_value_missing() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        std::env::set_var("HONCHO_TEST_ENV_KEY", "env-secret");

        let mut config = CaseConfig::default();
        config.honcho_api_key_env = "HONCHO_TEST_ENV_KEY".to_string();

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
}
