//! Search backend abstraction for case recall and per-case semantic lookup.
//!
//! Keywords: semantic search, recall backend, local text search, context hits

use crate::client::CaseClient;
use crate::error::CaseResult;
use crate::types::{CaseContextHit, CaseSearchResult};
use std::future::Future;
use std::pin::Pin;

pub type SearchFuture<'a, T> = Pin<Box<dyn Future<Output = CaseResult<T>> + Send + 'a>>;

#[derive(Debug, Clone, Copy)]
pub enum ContextScope<'a> {
    Case { case_id: &'a str },
    Repo,
}

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

    fn search_scope<'a>(
        &'a self,
        scope: ContextScope<'a>,
        query: &'a str,
        limit: usize,
    ) -> SearchFuture<'a, Vec<CaseContextHit>> {
        Box::pin(async move {
            match scope {
                ContextScope::Case { case_id } => self.search_case(case_id, query, limit).await,
                ContextScope::Repo => self.search_repo(query, limit).await,
            }
        })
    }

    fn search_repo<'a>(
        &'a self,
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
                Some(case.id.as_str()),
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
                    Some(case.id.as_str()),
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
                        Some(case.id.as_str()),
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
                        Some(case.id.as_str()),
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
                    Some(case.id.as_str()),
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
                        Some(case.id.as_str()),
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
                    Some(case.id.as_str()),
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
                        Some(case.id.as_str()),
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
                        Some(case.id.as_str()),
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
            let session_records = self.client.list_session_records().await?;
            for session_record in session_records
                .iter()
                .filter(|record| record.case_id.as_deref() == Some(case.id.as_str()))
            {
                push_hit(
                    &mut hits,
                    Some(case.id.as_str()),
                    "session_record",
                    "summary",
                    session_record.summary.as_str(),
                    &needle,
                    44,
                    Some(case.current_direction_seq),
                    Some(session_record.seq),
                    None,
                    Some(session_record.kind.as_str()),
                );
                if let Some(context) = session_record.context.as_deref() {
                    push_hit(
                        &mut hits,
                        Some(case.id.as_str()),
                        "session_record",
                        "context",
                        context,
                        &needle,
                        28,
                        Some(case.current_direction_seq),
                        Some(session_record.seq),
                        None,
                        Some(session_record.kind.as_str()),
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

    fn search_repo<'a>(
        &'a self,
        query: &'a str,
        limit: usize,
    ) -> SearchFuture<'a, Vec<CaseContextHit>> {
        Box::pin(async move {
            let needle = query.trim().to_lowercase();
            let cases = self.client.list_cases().await?;
            let mut hits = Vec::new();

            for case in cases {
                push_hit(
                    &mut hits,
                    Some(case.id.as_str()),
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

                let directions = self.client.get_directions(&case.id).await?;
                for direction in &directions {
                    push_hit(
                        &mut hits,
                        Some(case.id.as_str()),
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
                            Some(case.id.as_str()),
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
                            Some(case.id.as_str()),
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

                let steps = self.client.get_all_steps(&case.id).await?;
                for step in &steps {
                    push_hit(
                        &mut hits,
                        Some(case.id.as_str()),
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
                            Some(case.id.as_str()),
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

                let entries = self.client.get_entries(&case.id).await?;
                for entry in &entries {
                    push_hit(
                        &mut hits,
                        Some(case.id.as_str()),
                        "entry",
                        "summary",
                        entry.summary.as_str(),
                        &needle,
                        48,
                        None,
                        Some(entry.seq),
                        None,
                        entry.kind.as_deref(),
                    );
                    if let Some(reason) = entry.reason.as_deref() {
                        push_hit(
                            &mut hits,
                            Some(case.id.as_str()),
                            "entry",
                            "reason",
                            reason,
                            &needle,
                            24,
                            None,
                            Some(entry.seq),
                            None,
                            entry.kind.as_deref(),
                        );
                    }
                    if let Some(context) = entry.context.as_deref() {
                        push_hit(
                            &mut hits,
                            Some(case.id.as_str()),
                            "entry",
                            "context",
                            context,
                            &needle,
                            30,
                            None,
                            Some(entry.seq),
                            None,
                            entry.kind.as_deref(),
                        );
                    }
                }
            }
            let session_records = self.client.list_session_records().await?;
            for session_record in &session_records {
                push_hit(
                    &mut hits,
                    session_record.case_id.as_deref(),
                    "session_record",
                    "summary",
                    session_record.summary.as_str(),
                    &needle,
                    44,
                    None,
                    Some(session_record.seq),
                    None,
                    Some(session_record.kind.as_str()),
                );
                if let Some(context) = session_record.context.as_deref() {
                    push_hit(
                        &mut hits,
                        session_record.case_id.as_deref(),
                        "session_record",
                        "context",
                        context,
                        &needle,
                        28,
                        None,
                        Some(session_record.seq),
                        None,
                        Some(session_record.kind.as_str()),
                    );
                }
            }

            hits.sort_by(|left, right| {
                right
                    .score
                    .cmp(&left.score)
                    .then_with(|| right.entry_seq.cmp(&left.entry_seq))
                    .then_with(|| right.direction_seq.cmp(&left.direction_seq))
                    .then_with(|| left.case_id.cmp(&right.case_id))
            });
            hits.truncate(limit.max(1));

            Ok(hits)
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn push_hit(
    hits: &mut Vec<CaseContextHit>,
    case_id: Option<&str>,
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
        case_id: case_id.map(ToOwned::to_owned),
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
