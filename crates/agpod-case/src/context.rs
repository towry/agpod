//! Context provider abstraction for current case retrieval.
//!
//! Keywords: case context, semantic context, local context provider

use crate::client::CaseClient;
use crate::search::{CaseSearchBackend, ContextScope, LocalTextSearchBackend, SearchFuture};
use crate::types::{CaseContextResult, StepStatus};
use chrono::Utc;

pub trait CaseContextProvider: Send + Sync {
    fn backend_name(&self) -> &'static str;

    fn get_context<'a>(
        &'a self,
        scope: ContextScope<'a>,
        query: Option<&'a str>,
        limit: usize,
        token_limit: Option<u32>,
    ) -> SearchFuture<'a, CaseContextResult>;
}

#[derive(Clone)]
pub struct LocalCaseContextProvider {
    client: CaseClient,
    search: LocalTextSearchBackend,
}

impl LocalCaseContextProvider {
    pub fn new(client: CaseClient) -> Self {
        Self {
            search: LocalTextSearchBackend::new(client.clone()),
            client,
        }
    }
}

impl CaseContextProvider for LocalCaseContextProvider {
    fn backend_name(&self) -> &'static str {
        "local_text"
    }

    fn get_context<'a>(
        &'a self,
        scope: ContextScope<'a>,
        query: Option<&'a str>,
        limit: usize,
        token_limit: Option<u32>,
    ) -> SearchFuture<'a, CaseContextResult> {
        Box::pin(async move {
            let hits = match query {
                Some(query) if !query.trim().is_empty() => {
                    self.search.search_scope(scope, query, limit).await?
                }
                _ => Vec::new(),
            };
            let mut body = Vec::new();

            let (scope_name, case_id, repo_id) = match scope {
                ContextScope::Case { case_id } => {
                    let case = self.client.get_case(case_id).await?;
                    let direction = self
                        .client
                        .get_current_direction(case_id, case.current_direction_seq)
                        .await?;
                    let steps = self
                        .client
                        .get_steps(case_id, case.current_direction_seq)
                        .await?;
                    let entries = self.client.get_entries(case_id).await?;

                    body.push(format!("Case {} goal: {}", case.id, case.goal));
                    body.push(format!(
                        "Current direction {}: {}",
                        direction.seq, direction.summary
                    ));

                    if let Some(active) =
                        steps.iter().find(|step| step.status == StepStatus::Active)
                    {
                        body.push(format!("Active step {}: {}", active.id, active.title));
                    }

                    let pending = steps
                        .iter()
                        .filter(|step| step.status == StepStatus::Pending)
                        .map(|step| format!("{} {}", step.id, step.title))
                        .collect::<Vec<_>>();
                    if !pending.is_empty() {
                        body.push(format!("Pending steps: {}", pending.join("; ")));
                    }

                    if !hits.is_empty() {
                        body.push("Relevant hits:".to_string());
                        for hit in &hits {
                            body.push(format!("- {}.{}: {}", hit.source, hit.field, hit.excerpt));
                        }
                    }

                    let recent = entries.iter().rev().take(limit.max(1)).collect::<Vec<_>>();
                    if !recent.is_empty() {
                        body.push("Recent entries:".to_string());
                        for entry in recent.into_iter().rev() {
                            body.push(format!(
                                "- [{}] {}",
                                entry.entry_type.as_str(),
                                entry.summary
                            ));
                        }
                    }

                    (
                        "case".to_string(),
                        Some(case.id),
                        Some(self.client.repo_id().to_string()),
                    )
                }
                ContextScope::Repo => {
                    let mut cases = self.client.list_cases().await?;
                    cases.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
                    body.push(format!(
                        "Repository {} cross-session context",
                        self.client.repo_label()
                    ));
                    body.push(format!("Cases in scope: {}", cases.len()));

                    for case in cases.iter().take(limit.max(1)) {
                        body.push(format!(
                            "- Case {} [{}]: {}",
                            case.id,
                            case.status.as_str(),
                            case.goal
                        ));
                        let direction = self
                            .client
                            .get_current_direction(&case.id, case.current_direction_seq)
                            .await?;
                        body.push(format!(
                            "  Current direction {}: {}",
                            direction.seq, direction.summary
                        ));
                        let steps = self
                            .client
                            .get_steps(&case.id, case.current_direction_seq)
                            .await?;
                        if let Some(active) =
                            steps.iter().find(|step| step.status == StepStatus::Active)
                        {
                            body.push(format!("  Active step {}: {}", active.id, active.title));
                        }
                        let latest = self.client.get_latest_entry(&case.id).await?;
                        if let Some(entry) = latest {
                            body.push(format!(
                                "  Latest entry [{}{}]: {}",
                                entry.entry_type.as_str(),
                                entry
                                    .kind
                                    .as_deref()
                                    .map(|kind| format!("/{kind}"))
                                    .unwrap_or_default(),
                                entry.summary
                            ));
                        }
                    }

                    if !hits.is_empty() {
                        body.push("Relevant cross-session hits:".to_string());
                        for hit in &hits {
                            let label = hit.case_id.as_deref().unwrap_or("?");
                            body.push(format!(
                                "- case {} {}.{}: {}",
                                label, hit.source, hit.field, hit.excerpt
                            ));
                        }
                    }

                    (
                        "repo".to_string(),
                        None,
                        Some(self.client.repo_id().to_string()),
                    )
                }
            };

            let mut context = body.join("\n");
            if let Some(limit) = token_limit {
                let char_limit = (limit as usize).saturating_mul(4);
                if context.len() > char_limit {
                    context.truncate(char_limit);
                }
            }

            Ok(CaseContextResult {
                backend: self.backend_name().to_string(),
                scope: scope_name,
                case_id,
                repo_id,
                query: query.map(ToOwned::to_owned),
                token_limit,
                generated_at: Utc::now().to_rfc3339(),
                context,
                hits,
            })
        })
    }
}
