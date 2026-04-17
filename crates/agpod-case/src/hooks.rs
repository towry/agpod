//! Hook / plugin dispatch seam for case domain events.
//!
//! Keywords: case hook, plugin seam, event sink, dispatcher

use crate::error::CaseResult;
use crate::events::CaseEventEnvelope;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{debug, warn};

pub type HookFuture<'a> = Pin<Box<dyn Future<Output = CaseResult<()>> + Send + 'a>>;
const BACKGROUND_DISPATCH_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

pub trait CaseEventSink: Send + Sync {
    fn name(&self) -> &'static str;

    fn is_enabled(&self, _event: &CaseEventEnvelope) -> bool {
        true
    }

    fn handle<'a>(&'a self, event: &'a CaseEventEnvelope) -> HookFuture<'a>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaseHookState {
    Queued,
    Delivered,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseHookStatus {
    pub sink: String,
    pub state: CaseHookState,
    pub message: Option<String>,
}

impl CaseHookStatus {
    pub fn queued(sink: String) -> Self {
        Self {
            sink,
            state: CaseHookState::Queued,
            message: Some("queued for background delivery".to_string()),
        }
    }

    pub fn delivered(sink: String) -> Self {
        Self {
            sink,
            state: CaseHookState::Delivered,
            message: None,
        }
    }

    pub fn failed(sink: String, message: Option<String>) -> Self {
        Self {
            sink,
            state: CaseHookState::Failed,
            message,
        }
    }

    pub fn is_failure(&self) -> bool {
        matches!(self.state, CaseHookState::Failed)
    }
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
        self.statuses.iter().any(CaseHookStatus::is_failure)
    }

    pub fn warnings(&self) -> Vec<String> {
        self.statuses
            .iter()
            .filter(|status| status.is_failure())
            .map(|status| {
                let message = status.message.as_deref().unwrap_or("hook dispatch failed");
                format!("hook `{}` failed: {message}", status.sink)
            })
            .collect()
    }
}

#[derive(Clone, Default)]
pub struct CaseEventDispatcher {
    sinks: Vec<Arc<dyn CaseEventSink>>,
}

#[derive(Clone)]
struct CaseDispatchJob {
    dispatcher: CaseEventDispatcher,
    event: CaseEventEnvelope,
    timeout_limit: Duration,
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
                    .map(|sink| {
                        CaseHookStatus::failed(
                            sink,
                            Some(format!(
                                "hook dispatch timed out after {} ms",
                                timeout_limit.as_millis()
                            )),
                        )
                    })
                    .collect(),
            },
        }
    }

    pub fn dispatch_in_background(
        &self,
        event: CaseEventEnvelope,
        timeout_limit: Duration,
    ) -> CaseDispatchReport {
        let sink_names = self.enabled_sink_names(&event);
        if sink_names.is_empty() {
            return CaseDispatchReport::default();
        }

        let handle = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                return CaseDispatchReport {
                    statuses: sink_names
                        .into_iter()
                        .map(|sink| {
                            CaseHookStatus::failed(
                                sink,
                                Some(
                                    "no Tokio runtime available for background hook delivery"
                                        .to_string(),
                                ),
                            )
                        })
                        .collect(),
                };
            }
        };

        let queue_key = event.case_id.clone();
        let sender = background_dispatch_sender(&queue_key, &handle);
        if sender
            .send(CaseDispatchJob {
                dispatcher: self.clone(),
                event,
                timeout_limit,
            })
            .is_err()
        {
            return CaseDispatchReport {
                statuses: sink_names
                    .into_iter()
                    .map(|sink| {
                        CaseHookStatus::failed(
                            sink,
                            Some("background hook queue is unavailable".to_string()),
                        )
                    })
                    .collect(),
            };
        }

        CaseDispatchReport {
            statuses: sink_names.into_iter().map(CaseHookStatus::queued).collect(),
        }
    }

    async fn dispatch_inner(&self, event: &CaseEventEnvelope) -> CaseDispatchReport {
        let mut statuses = Vec::new();

        for sink in &self.sinks {
            if !sink.is_enabled(event) {
                continue;
            }

            let status = match sink.handle(event).await {
                Ok(()) => CaseHookStatus::delivered(sink.name().to_string()),
                Err(error) => {
                    CaseHookStatus::failed(sink.name().to_string(), Some(error.to_string()))
                }
            };
            statuses.push(status);
        }

        CaseDispatchReport { statuses }
    }
}

fn background_dispatch_queues(
) -> &'static StdMutex<HashMap<String, mpsc::UnboundedSender<CaseDispatchJob>>> {
    static QUEUES: OnceLock<StdMutex<HashMap<String, mpsc::UnboundedSender<CaseDispatchJob>>>> =
        OnceLock::new();
    QUEUES.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn background_dispatch_sender(
    queue_key: &str,
    handle: &tokio::runtime::Handle,
) -> mpsc::UnboundedSender<CaseDispatchJob> {
    let mut queues = background_dispatch_queues()
        .lock()
        .expect("background dispatch queue registry should not be poisoned");
    if let Some(sender) = queues.get(queue_key) {
        if !sender.is_closed() {
            return sender.clone();
        }
        queues.remove(queue_key);
    }

    let (sender, receiver) = mpsc::unbounded_channel();
    queues.insert(queue_key.to_string(), sender.clone());
    spawn_background_dispatch_worker(handle.clone(), queue_key.to_string(), receiver);
    sender
}

fn spawn_background_dispatch_worker(
    handle: tokio::runtime::Handle,
    queue_key: String,
    mut receiver: mpsc::UnboundedReceiver<CaseDispatchJob>,
) {
    handle.spawn(async move {
        loop {
            let job = match timeout(BACKGROUND_DISPATCH_IDLE_TIMEOUT, receiver.recv()).await {
                Ok(Some(job)) => job,
                Ok(None) | Err(_) => break,
            };

            let case_id = job.event.case_id.clone();
            let event_id = job.event.event_id.clone();
            let report = job
                .dispatcher
                .dispatch_with_timeout(&job.event, job.timeout_limit)
                .await;
            log_background_dispatch_result(&case_id, &event_id, &report);
        }

        if let Ok(mut queues) = background_dispatch_queues().lock() {
            queues.remove(&queue_key);
        }
    });
}

fn log_background_dispatch_result(case_id: &str, event_id: &str, report: &CaseDispatchReport) {
    if report.is_empty() {
        return;
    }

    if report.has_failures() {
        for status in report.statuses.iter().filter(|status| status.is_failure()) {
            warn!(
                case_id,
                event_id,
                sink = %status.sink,
                message = %status.message.as_deref().unwrap_or("hook dispatch failed"),
                "case hook background delivery failed"
            );
        }
        return;
    }

    debug!(
        case_id,
        event_id,
        sink_count = report.statuses.len(),
        "case hook background delivery completed"
    );
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
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
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

    struct CountingSlowSink {
        completions: Arc<AtomicUsize>,
    }

    impl CaseEventSink for CountingSlowSink {
        fn name(&self) -> &'static str {
            "counting_slow"
        }

        fn handle<'a>(&'a self, _event: &'a CaseEventEnvelope) -> HookFuture<'a> {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                self.completions.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    struct OrderingSink {
        delivered: Arc<StdMutex<Vec<String>>>,
    }

    impl CaseEventSink for OrderingSink {
        fn name(&self) -> &'static str {
            "ordering"
        }

        fn handle<'a>(&'a self, event: &'a CaseEventEnvelope) -> HookFuture<'a> {
            Box::pin(async move {
                if event.event.event_type() == "case_opened" {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
                self.delivered
                    .lock()
                    .expect("ordering sink buffer should not be poisoned")
                    .push(event.event.event_type().to_string());
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
        assert_eq!(report.statuses[0].state, CaseHookState::Failed);
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
        assert_eq!(report.statuses[0].state, CaseHookState::Failed);
        assert!(report.statuses[0]
            .message
            .as_deref()
            .is_some_and(|message| message.contains("timed out")));
    }

    #[tokio::test]
    async fn dispatcher_can_queue_background_delivery_without_blocking_response() {
        let completions = Arc::new(AtomicUsize::new(0));
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
        let dispatcher = CaseEventDispatcher::new(vec![Arc::new(CountingSlowSink {
            completions: completions.clone(),
        })]);

        let started = std::time::Instant::now();
        let report = dispatcher.dispatch_in_background(event, Duration::from_secs(1));

        assert!(started.elapsed() < Duration::from_millis(20));
        assert_eq!(report.statuses.len(), 1);
        assert_eq!(report.statuses[0].sink, "counting_slow");
        assert_eq!(report.statuses[0].state, CaseHookState::Queued);
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(completions.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dispatcher_preserves_event_order_per_case_when_queued() {
        let delivered = Arc::new(StdMutex::new(Vec::new()));
        let dispatcher = CaseEventDispatcher::new(vec![Arc::new(OrderingSink {
            delivered: delivered.clone(),
        })]);
        let case_opened = CaseEventEnvelope {
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
        let step_done = CaseEventEnvelope {
            event_id: "C-1:step_done".to_string(),
            case_id: "C-1".to_string(),
            associated_case_id: Some("C-1".to_string()),
            repo_id: "repo".to_string(),
            repo_label: "repo".to_string(),
            worktree_id: "wt".to_string(),
            worktree_root: "/tmp/repo".to_string(),
            direction_seq: Some(1),
            occurred_at: "2026-03-25T00:00:01Z".to_string(),
            event: CaseDomainEvent::StepDone {
                case: sample_case(),
                step: crate::types::Step {
                    id: "C-1/S-1".to_string(),
                    case_id: "C-1".to_string(),
                    direction_seq: 1,
                    order_index: 1,
                    title: "step".to_string(),
                    reason: None,
                    status: crate::types::StepStatus::Done,
                    created_at: "2026-03-25T00:00:00Z".to_string(),
                    updated_at: "2026-03-25T00:00:01Z".to_string(),
                },
            },
        };

        dispatcher.dispatch_in_background(case_opened, Duration::from_secs(1));
        dispatcher.dispatch_in_background(step_done, Duration::from_secs(1));

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if delivered
                    .lock()
                    .expect("ordering sink buffer should not be poisoned")
                    .len()
                    == 2
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("queued events should drain in order");
        let delivered = delivered
            .lock()
            .expect("ordering sink buffer should not be poisoned")
            .clone();
        assert_eq!(
            delivered,
            vec!["case_opened".to_string(), "step_done".to_string()]
        );
    }
}
