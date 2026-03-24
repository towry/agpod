//! Search backend abstraction for case recall and per-case semantic lookup.
//!
//! Keywords: semantic search, recall backend, local text search, context hits

use crate::client::CaseClient;
use crate::error::CaseResult;
use crate::types::{CaseContextHit, CaseSearchResult};
use std::future::Future;
use std::pin::Pin;

pub type SearchFuture<'a, T> = Pin<Box<dyn Future<Output = CaseResult<T>> + Send + 'a>>;

pub trait CaseSearchBackend: Send + Sync {
    #[allow(dead_code)]
    fn backend_name(&self) -> &'static str;

    fn recall_cases<'a>(&'a self, query: &'a str) -> SearchFuture<'a, Vec<CaseSearchResult>>;

    fn search_case<'a>(
        &'a self,
        case_id: &'a str,
        query: &'a str,
        limit: usize,
    ) -> SearchFuture<'a, Vec<CaseContextHit>>;
}

#[derive(Clone)]
pub struct LocalTextSearchBackend {
    client: CaseClient,
}

impl LocalTextSearchBackend {
    pub fn new(client: CaseClient) -> Self {
        Self { client }
    }
}

impl CaseSearchBackend for LocalTextSearchBackend {
    fn backend_name(&self) -> &'static str {
        "local_text"
    }

    fn recall_cases<'a>(&'a self, query: &'a str) -> SearchFuture<'a, Vec<CaseSearchResult>> {
        Box::pin(async move { self.client.search_cases(query).await })
    }

    fn search_case<'a>(
        &'a self,
        case_id: &'a str,
        query: &'a str,
        limit: usize,
    ) -> SearchFuture<'a, Vec<CaseContextHit>> {
        Box::pin(async move {
            let needle = query.trim().to_lowercase();
            let case = self.client.get_case(case_id).await?;
            let directions = self.client.get_directions(case_id).await?;
            let steps = self.client.get_all_steps(case_id).await?;
            let entries = self.client.get_entries(case_id).await?;
            let mut hits = Vec::new();

            push_hit(
                &mut hits,
                "case",
                "goal",
                case.goal.as_str(),
                &needle,
                80,
                None,
                None,
                None,
                None,
            );

            for direction in &directions {
                push_hit(
                    &mut hits,
                    "direction",
                    "summary",
                    direction.summary.as_str(),
                    &needle,
                    60,
                    Some(direction.seq),
                    None,
                    None,
                    None,
                );
                if let Some(reason) = direction.reason.as_deref() {
                    push_hit(
                        &mut hits,
                        "direction",
                        "reason",
                        reason,
                        &needle,
                        36,
                        Some(direction.seq),
                        None,
                        None,
                        None,
                    );
                }
                if let Some(context) = direction.context.as_deref() {
                    push_hit(
                        &mut hits,
                        "direction",
                        "context",
                        context,
                        &needle,
                        34,
                        Some(direction.seq),
                        None,
                        None,
                        None,
                    );
                }
            }

            for step in &steps {
                push_hit(
                    &mut hits,
                    "step",
                    "title",
                    step.title.as_str(),
                    &needle,
                    28,
                    Some(step.direction_seq),
                    None,
                    Some(step.id.as_str()),
                    None,
                );
                if let Some(reason) = step.reason.as_deref() {
                    push_hit(
                        &mut hits,
                        "step",
                        "reason",
                        reason,
                        &needle,
                        20,
                        Some(step.direction_seq),
                        None,
                        Some(step.id.as_str()),
                        None,
                    );
                }
            }

            for entry in &entries {
                push_hit(
                    &mut hits,
                    "entry",
                    "summary",
                    entry.summary.as_str(),
                    &needle,
                    48,
                    Some(case.current_direction_seq),
                    Some(entry.seq),
                    None,
                    entry.kind.as_deref(),
                );
                if let Some(reason) = entry.reason.as_deref() {
                    push_hit(
                        &mut hits,
                        "entry",
                        "reason",
                        reason,
                        &needle,
                        24,
                        Some(case.current_direction_seq),
                        Some(entry.seq),
                        None,
                        entry.kind.as_deref(),
                    );
                }
                if let Some(context) = entry.context.as_deref() {
                    push_hit(
                        &mut hits,
                        "entry",
                        "context",
                        context,
                        &needle,
                        30,
                        Some(case.current_direction_seq),
                        Some(entry.seq),
                        None,
                        entry.kind.as_deref(),
                    );
                }
            }

            hits.sort_by(|left, right| {
                right
                    .score
                    .cmp(&left.score)
                    .then_with(|| right.entry_seq.cmp(&left.entry_seq))
                    .then_with(|| right.direction_seq.cmp(&left.direction_seq))
            });
            hits.truncate(limit.max(1));

            Ok(hits)
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn push_hit(
    hits: &mut Vec<CaseContextHit>,
    source: &str,
    field: &str,
    haystack: &str,
    needle: &str,
    score: i64,
    direction_seq: Option<u32>,
    entry_seq: Option<u32>,
    step_id: Option<&str>,
    kind: Option<&str>,
) {
    if !haystack.to_lowercase().contains(needle) {
        return;
    }

    hits.push(CaseContextHit {
        source: source.to_string(),
        field: field.to_string(),
        excerpt: haystack.to_string(),
        score,
        direction_seq,
        entry_seq,
        step_id: step_id.map(ToOwned::to_owned),
        kind: kind.map(ToOwned::to_owned),
    });
}
