//! Hook / plugin dispatch seam for case domain events.
//!
//! Keywords: case hook, plugin seam, event sink, dispatcher

use crate::error::CaseResult;
use crate::events::CaseEventEnvelope;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

pub type HookFuture<'a> = Pin<Box<dyn Future<Output = CaseResult<()>> + Send + 'a>>;

pub trait CaseEventSink: Send + Sync {
    fn name(&self) -> &'static str;

    fn is_enabled(&self, _event: &CaseEventEnvelope) -> bool {
        true
    }

    fn handle<'a>(&'a self, event: &'a CaseEventEnvelope) -> HookFuture<'a>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseHookStatus {
    pub sink: String,
    pub ok: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CaseDispatchReport {
    pub statuses: Vec<CaseHookStatus>,
}

impl CaseDispatchReport {
    pub fn is_empty(&self) -> bool {
        self.statuses.is_empty()
    }

    pub fn has_failures(&self) -> bool {
        self.statuses.iter().any(|status| !status.ok)
    }

    pub fn warnings(&self) -> Vec<String> {
        self.statuses
            .iter()
            .filter(|status| !status.ok)
            .map(|status| {
                let message = status.message.as_deref().unwrap_or("hook dispatch failed");
                format!("hook `{}` failed: {message}", status.sink)
            })
            .collect()
    }
}

#[derive(Default)]
pub struct CaseEventDispatcher {
    sinks: Vec<Arc<dyn CaseEventSink>>,
}

impl CaseEventDispatcher {
    pub fn new(sinks: Vec<Arc<dyn CaseEventSink>>) -> Self {
        Self { sinks }
    }

    #[allow(dead_code)]
    pub fn noop() -> Self {
        Self::default()
    }

    pub fn enabled_sink_names(&self, event: &CaseEventEnvelope) -> Vec<String> {
        self.sinks
            .iter()
            .filter(|sink| sink.is_enabled(event))
            .map(|sink| sink.name().to_string())
            .collect()
    }

    #[allow(dead_code)]
    pub async fn dispatch(&self, event: &CaseEventEnvelope) -> CaseDispatchReport {
        self.dispatch_with_timeout(event, crate::CASE_REQUEST_TIMEOUT)
            .await
    }

    pub async fn dispatch_with_timeout(
        &self,
        event: &CaseEventEnvelope,
        timeout_limit: Duration,
    ) -> CaseDispatchReport {
        let sink_names = self.enabled_sink_names(event);
        match timeout(timeout_limit, self.dispatch_inner(event)).await {
            Ok(report) => report,
            Err(_) => CaseDispatchReport {
                statuses: sink_names
                    .into_iter()
                    .map(|sink| CaseHookStatus {
                        sink,
                        ok: false,
                        message: Some(format!(
                            "hook dispatch timed out after {} ms",
                            timeout_limit.as_millis()
                        )),
                    })
                    .collect(),
            },
        }
    }

    async fn dispatch_inner(&self, event: &CaseEventEnvelope) -> CaseDispatchReport {
        let mut statuses = Vec::new();

        for sink in &self.sinks {
            if !sink.is_enabled(event) {
                continue;
            }

            let status = match sink.handle(event).await {
                Ok(()) => CaseHookStatus {
                    sink: sink.name().to_string(),
                    ok: true,
                    message: None,
                },
                Err(error) => CaseHookStatus {
                    sink: sink.name().to_string(),
                    ok: false,
                    message: Some(error.to_string()),
                },
            };
            statuses.push(status);
        }

        CaseDispatchReport { statuses }
    }
}

#[allow(dead_code)]
pub struct NoopSink;

impl CaseEventSink for NoopSink {
    fn name(&self) -> &'static str {
        "noop"
    }

    fn handle<'a>(&'a self, _event: &'a CaseEventEnvelope) -> HookFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::CaseDomainEvent;
    use crate::types::{Case, CaseStatus, Direction};
    use std::time::Duration;

    struct FailingSink;

    impl CaseEventSink for FailingSink {
        fn name(&self) -> &'static str {
            "failing"
        }

        fn handle<'a>(&'a self, _event: &'a CaseEventEnvelope) -> HookFuture<'a> {
            Box::pin(async { Err(crate::error::CaseError::Other("boom".to_string())) })
        }
    }

    struct SlowSink;

    impl CaseEventSink for SlowSink {
        fn name(&self) -> &'static str {
            "slow"
        }

        fn handle<'a>(&'a self, _event: &'a CaseEventEnvelope) -> HookFuture<'a> {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok(())
            })
        }
    }

    fn sample_case() -> Case {
        Case {
            id: "C-1".to_string(),
            repo_id: "repo".to_string(),
            repo_label: Some("repo".to_string()),
            worktree_id: Some("wt".to_string()),
            worktree_root: Some("/tmp/repo".to_string()),
            goal: "goal".to_string(),
            goal_constraints: vec![],
            status: CaseStatus::Open,
            current_direction_seq: 1,
            current_step_id: None,
            opened_at: "2026-03-25T00:00:00Z".to_string(),
            updated_at: "2026-03-25T00:00:00Z".to_string(),
            closed_at: None,
            close_summary: None,
            abandoned_at: None,
            abandon_summary: None,
            close_confirm_token: None,
            close_confirm_action: None,
            close_confirm_summary: None,
        }
    }

    #[tokio::test]
    async fn dispatcher_collects_sink_failures_as_report() {
        let event = CaseEventEnvelope {
            event_id: "C-1:case_opened".to_string(),
            case_id: "C-1".to_string(),
            associated_case_id: Some("C-1".to_string()),
            repo_id: "repo".to_string(),
            repo_label: "repo".to_string(),
            worktree_id: "wt".to_string(),
            worktree_root: "/tmp/repo".to_string(),
            direction_seq: Some(1),
            occurred_at: "2026-03-25T00:00:00Z".to_string(),
            event: CaseDomainEvent::CaseOpened {
                case: sample_case(),
                direction: Direction {
                    case_id: "C-1".to_string(),
                    seq: 1,
                    summary: "dir".to_string(),
                    constraints: vec![],
                    success_condition: "".to_string(),
                    abort_condition: "".to_string(),
                    reason: None,
                    context: None,
                    created_at: "2026-03-25T00:00:00Z".to_string(),
                },
            },
        };
        let dispatcher = CaseEventDispatcher::new(vec![Arc::new(FailingSink)]);

        let report = dispatcher.dispatch(&event).await;

        assert!(report.has_failures());
        assert_eq!(report.statuses.len(), 1);
        assert_eq!(report.statuses[0].sink, "failing");
    }

    #[tokio::test]
    async fn dispatcher_times_out_slow_sinks() {
        let event = CaseEventEnvelope {
            event_id: "C-1:case_opened".to_string(),
            case_id: "C-1".to_string(),
            associated_case_id: Some("C-1".to_string()),
            repo_id: "repo".to_string(),
            repo_label: "repo".to_string(),
            worktree_id: "wt".to_string(),
            worktree_root: "/tmp/repo".to_string(),
            direction_seq: Some(1),
            occurred_at: "2026-03-25T00:00:00Z".to_string(),
            event: CaseDomainEvent::CaseOpened {
                case: sample_case(),
                direction: Direction {
                    case_id: "C-1".to_string(),
                    seq: 1,
                    summary: "dir".to_string(),
                    constraints: vec![],
                    success_condition: "".to_string(),
                    abort_condition: "".to_string(),
                    reason: None,
                    context: None,
                    created_at: "2026-03-25T00:00:00Z".to_string(),
                },
            },
        };
        let dispatcher = CaseEventDispatcher::new(vec![Arc::new(SlowSink)]);

        let report = dispatcher
            .dispatch_with_timeout(&event, Duration::from_millis(5))
            .await;

        assert_eq!(report.statuses.len(), 1);
        assert_eq!(report.statuses[0].sink, "slow");
        assert!(!report.statuses[0].ok);
        assert!(report.statuses[0]
            .message
            .as_deref()
            .is_some_and(|message| message.contains("timed out")));
    }
}
