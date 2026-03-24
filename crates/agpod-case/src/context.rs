//! Context provider abstraction for current case retrieval.
//!
//! Keywords: case context, semantic context, local context provider

use crate::client::CaseClient;
use crate::search::{CaseSearchBackend, LocalTextSearchBackend, SearchFuture};
use crate::types::{CaseContextResult, StepStatus};
use chrono::Utc;

pub trait CaseContextProvider: Send + Sync {
    fn backend_name(&self) -> &'static str;

    fn get_context<'a>(
        &'a self,
        case_id: &'a str,
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
        case_id: &'a str,
        query: Option<&'a str>,
        limit: usize,
        token_limit: Option<u32>,
    ) -> SearchFuture<'a, CaseContextResult> {
        Box::pin(async move {
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
            let hits = match query {
                Some(query) if !query.trim().is_empty() => {
                    self.search.search_case(case_id, query, limit).await?
                }
                _ => Vec::new(),
            };

            let mut body = Vec::new();
            body.push(format!("Case {} goal: {}", case.id, case.goal));
            body.push(format!(
                "Current direction {}: {}",
                direction.seq, direction.summary
            ));

            if let Some(active) = steps.iter().find(|step| step.status == StepStatus::Active) {
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

            let mut context = body.join("\n");
            if let Some(limit) = token_limit {
                let char_limit = (limit as usize).saturating_mul(4);
                if context.len() > char_limit {
                    context.truncate(char_limit);
                }
            }

            Ok(CaseContextResult {
                backend: self.backend_name().to_string(),
                case_id: case.id,
                query: query.map(ToOwned::to_owned),
                token_limit,
                generated_at: Utc::now().to_rfc3339(),
                context,
                hits,
            })
        })
    }
}
