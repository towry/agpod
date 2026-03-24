//! Typed domain events emitted from successful case mutations.
//!
//! Keywords: case event, domain event, hook event, semantic sync

use crate::client::CaseClient;
use crate::types::{Case, Direction, Entry, Step};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum CaseDomainEvent {
    CaseOpened {
        case: Case,
        direction: Direction,
    },
    CaseReopened {
        case: Case,
        direction: Direction,
        reopened_entry: Entry,
    },
    RecordAppended {
        case: Case,
        entry: Entry,
    },
    DecisionAppended {
        case: Case,
        entry: Entry,
    },
    RedirectCommitted {
        case: Case,
        from_direction: Direction,
        to_direction: Direction,
        entry: Entry,
    },
    RedirectRecovered {
        case: Case,
        from_direction: Direction,
        to_direction: Direction,
    },
    StepAdded {
        case: Case,
        step: Step,
    },
    StepStarted {
        case: Case,
        step: Step,
    },
    StepDone {
        case: Case,
        step: Step,
    },
    StepBlocked {
        case: Case,
        step: Step,
    },
    StepsReordered {
        case: Case,
        moved_step_id: String,
        before_step_id: String,
        steps: Vec<Step>,
    },
    CaseClosed {
        case: Case,
        summary: String,
    },
    CaseAbandoned {
        case: Case,
        summary: String,
    },
}

impl CaseDomainEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::CaseOpened { .. } => "case_opened",
            Self::CaseReopened { .. } => "case_reopened",
            Self::RecordAppended { .. } => "record_appended",
            Self::DecisionAppended { .. } => "decision_appended",
            Self::RedirectCommitted { .. } => "redirect_committed",
            Self::RedirectRecovered { .. } => "redirect_recovered",
            Self::StepAdded { .. } => "step_added",
            Self::StepStarted { .. } => "step_started",
            Self::StepDone { .. } => "step_done",
            Self::StepBlocked { .. } => "step_blocked",
            Self::StepsReordered { .. } => "steps_reordered",
            Self::CaseClosed { .. } => "case_closed",
            Self::CaseAbandoned { .. } => "case_abandoned",
        }
    }

    pub fn occurred_at(&self) -> &str {
        match self {
            Self::CaseOpened { case, .. } => case.opened_at.as_str(),
            Self::CaseReopened { reopened_entry, .. } => reopened_entry.created_at.as_str(),
            Self::RecordAppended { entry, .. } => entry.created_at.as_str(),
            Self::DecisionAppended { entry, .. } => entry.created_at.as_str(),
            Self::RedirectCommitted { entry, .. } => entry.created_at.as_str(),
            Self::RedirectRecovered { case, .. } => case.updated_at.as_str(),
            Self::StepAdded { step, .. } => step.created_at.as_str(),
            Self::StepStarted { step, .. } => step.updated_at.as_str(),
            Self::StepDone { step, .. } => step.updated_at.as_str(),
            Self::StepBlocked { step, .. } => step.updated_at.as_str(),
            Self::StepsReordered { case, .. } => case.updated_at.as_str(),
            Self::CaseClosed { case, .. } => case.closed_at.as_deref().unwrap_or(&case.updated_at),
            Self::CaseAbandoned { case, .. } => {
                case.abandoned_at.as_deref().unwrap_or(&case.updated_at)
            }
        }
    }

    pub fn case_id(&self) -> &str {
        match self {
            Self::CaseOpened { case, .. }
            | Self::CaseReopened { case, .. }
            | Self::RecordAppended { case, .. }
            | Self::DecisionAppended { case, .. }
            | Self::RedirectCommitted { case, .. }
            | Self::RedirectRecovered { case, .. }
            | Self::StepAdded { case, .. }
            | Self::StepStarted { case, .. }
            | Self::StepDone { case, .. }
            | Self::StepBlocked { case, .. }
            | Self::StepsReordered { case, .. }
            | Self::CaseClosed { case, .. }
            | Self::CaseAbandoned { case, .. } => case.id.as_str(),
        }
    }

    pub fn direction_seq(&self) -> Option<u32> {
        match self {
            Self::CaseOpened { direction, .. } | Self::CaseReopened { direction, .. } => {
                Some(direction.seq)
            }
            Self::RecordAppended { case, .. }
            | Self::DecisionAppended { case, .. }
            | Self::StepAdded { case, .. }
            | Self::StepStarted { case, .. }
            | Self::StepDone { case, .. }
            | Self::StepBlocked { case, .. }
            | Self::StepsReordered { case, .. }
            | Self::CaseClosed { case, .. }
            | Self::CaseAbandoned { case, .. } => Some(case.current_direction_seq),
            Self::RedirectRecovered { to_direction, .. } => Some(to_direction.seq),
            Self::RedirectCommitted { to_direction, .. } => Some(to_direction.seq),
        }
    }

    pub fn metadata(&self) -> Map<String, Value> {
        match self {
            Self::CaseOpened { case, direction } => json!({
                "case_id": case.id,
                "direction_seq": direction.seq,
            }),
            Self::CaseReopened {
                case,
                direction,
                reopened_entry,
            } => json!({
                "case_id": case.id,
                "direction_seq": direction.seq,
                "entry_seq": reopened_entry.seq,
            }),
            Self::RecordAppended { case, entry } => json!({
                "case_id": case.id,
                "direction_seq": case.current_direction_seq,
                "entry_seq": entry.seq,
                "entry_type": entry.entry_type.as_str(),
                "kind": entry.kind,
            }),
            Self::DecisionAppended { case, entry } => json!({
                "case_id": case.id,
                "direction_seq": case.current_direction_seq,
                "entry_seq": entry.seq,
                "entry_type": entry.entry_type.as_str(),
            }),
            Self::RedirectCommitted {
                case,
                from_direction,
                to_direction,
                entry,
            } => json!({
                "case_id": case.id,
                "entry_seq": entry.seq,
                "from_direction_seq": from_direction.seq,
                "to_direction_seq": to_direction.seq,
            }),
            Self::RedirectRecovered {
                case,
                from_direction,
                to_direction,
            } => json!({
                "case_id": case.id,
                "from_direction_seq": from_direction.seq,
                "to_direction_seq": to_direction.seq,
            }),
            Self::StepAdded { case, step }
            | Self::StepStarted { case, step }
            | Self::StepDone { case, step }
            | Self::StepBlocked { case, step } => json!({
                "case_id": case.id,
                "direction_seq": step.direction_seq,
                "step_id": step.id,
                "step_status": step.status.as_str(),
            }),
            Self::StepsReordered {
                case,
                moved_step_id,
                before_step_id,
                ..
            } => json!({
                "case_id": case.id,
                "direction_seq": case.current_direction_seq,
                "moved_step_id": moved_step_id,
                "before_step_id": before_step_id,
            }),
            Self::CaseClosed { case, summary } => json!({
                "case_id": case.id,
                "status": case.status.as_str(),
                "summary": summary,
            }),
            Self::CaseAbandoned { case, summary } => json!({
                "case_id": case.id,
                "status": case.status.as_str(),
                "summary": summary,
            }),
        }
        .as_object()
        .cloned()
        .unwrap_or_default()
    }

    pub fn summary_text(&self) -> String {
        match self {
            Self::CaseOpened { case, direction } => {
                format!(
                    "Case {} opened. Goal: {}. Direction: {}.",
                    case.id, case.goal, direction.summary
                )
            }
            Self::CaseReopened {
                case, direction, ..
            } => {
                format!(
                    "Case {} reopened. Current direction {}: {}.",
                    case.id, direction.seq, direction.summary
                )
            }
            Self::RecordAppended { case, entry } => {
                format!("Record appended to case {}: {}.", case.id, entry.summary)
            }
            Self::DecisionAppended { case, entry } => {
                format!("Decision recorded for case {}: {}.", case.id, entry.summary)
            }
            Self::RedirectCommitted {
                case,
                from_direction,
                to_direction,
                ..
            } => format!(
                "Case {} redirected from direction {} to {}.",
                case.id, from_direction.summary, to_direction.summary
            ),
            Self::RedirectRecovered {
                case,
                from_direction,
                to_direction,
            } => format!(
                "Case {} recovered redirect from direction {} to {}.",
                case.id, from_direction.summary, to_direction.summary
            ),
            Self::StepAdded { case, step } => {
                format!("Step added to case {}: {}.", case.id, step.title)
            }
            Self::StepStarted { case, step } => {
                format!("Step started in case {}: {}.", case.id, step.title)
            }
            Self::StepDone { case, step } => {
                format!("Step done in case {}: {}.", case.id, step.title)
            }
            Self::StepBlocked { case, step } => {
                format!("Step blocked in case {}: {}.", case.id, step.title)
            }
            Self::StepsReordered {
                case,
                moved_step_id,
                before_step_id,
                ..
            } => format!(
                "Steps reordered in case {}: {} moved before {}.",
                case.id, moved_step_id, before_step_id
            ),
            Self::CaseClosed { case, summary } => {
                format!("Case {} closed: {}.", case.id, summary)
            }
            Self::CaseAbandoned { case, summary } => {
                format!("Case {} abandoned: {}.", case.id, summary)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseEventEnvelope {
    pub event_id: String,
    pub case_id: String,
    pub repo_id: String,
    pub repo_label: String,
    pub worktree_id: String,
    pub worktree_root: String,
    pub direction_seq: Option<u32>,
    pub occurred_at: String,
    pub event: CaseDomainEvent,
}

impl CaseEventEnvelope {
    pub fn new(client: &CaseClient, event: CaseDomainEvent) -> Self {
        let occurred_at = event.occurred_at().to_string();
        let metadata = event.metadata();
        let discriminator = metadata
            .get("entry_seq")
            .or_else(|| metadata.get("step_id"))
            .or_else(|| metadata.get("to_direction_seq"))
            .or_else(|| metadata.get("summary"))
            .map(Value::to_string)
            .unwrap_or_else(|| "na".to_string());
        Self {
            event_id: format!(
                "{}:{}:{}:{}",
                event.case_id(),
                event.event_type(),
                discriminator,
                occurred_at
            ),
            case_id: event.case_id().to_string(),
            repo_id: client.repo_id().to_string(),
            repo_label: client.repo_label().to_string(),
            worktree_id: client.worktree_id().to_string(),
            worktree_root: client.worktree_root().to_string(),
            direction_seq: event.direction_seq(),
            occurred_at,
            event,
        }
    }
}
