//! Typed domain events emitted from successful case mutations.
//!
//! Keywords: case event, domain event, hook event, semantic sync

use crate::client::CaseClient;
use crate::types::{Case, Direction, Entry, SessionRecord, Step};
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
    SessionRecordAppended {
        case: Option<Case>,
        session_record: SessionRecord,
        linked_entry: Option<Entry>,
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
            Self::SessionRecordAppended { .. } => "session_record_appended",
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
            Self::SessionRecordAppended { session_record, .. } => {
                session_record.created_at.as_str()
            }
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

    pub fn case_id(&self) -> Option<&str> {
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
            | Self::CaseAbandoned { case, .. } => Some(case.id.as_str()),
            Self::SessionRecordAppended { case, .. } => case.as_ref().map(|case| case.id.as_str()),
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
            Self::SessionRecordAppended { case, .. } => {
                case.as_ref().map(|case| case.current_direction_seq)
            }
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
                "step_id": entry.step_id,
            }),
            Self::SessionRecordAppended {
                case,
                session_record,
                linked_entry,
            } => json!({
                "case_id": case.as_ref().map(|case| case.id.clone()),
                "session_record_id": session_record.id,
                "session_record_seq": session_record.seq,
                "kind": session_record.kind.as_str(),
                "entry_seq": linked_entry.as_ref().map(|entry| entry.seq),
                "entry_type": linked_entry.as_ref().map(|entry| entry.entry_type.as_str()),
                "step_id": linked_entry.as_ref().and_then(|entry| entry.step_id.clone()),
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

    pub fn honcho_content(&self) -> String {
        match self {
            Self::CaseOpened { case, direction } => format!(
                "Opened case. Goal: {}. Direction: {}.",
                compact_text(&case.goal),
                compact_text(&direction.summary)
            ),
            Self::CaseReopened { direction, .. } => format!(
                "Reopened case. Direction {}: {}.",
                direction.seq,
                compact_text(&direction.summary)
            ),
            Self::RecordAppended { entry, .. } => match entry.kind.as_deref() {
                Some(kind) if !kind.trim().is_empty() => {
                    format!("Recorded {kind}: {}.", compact_text(&entry.summary))
                }
                _ => format!("Recorded: {}.", compact_text(&entry.summary)),
            },
            Self::SessionRecordAppended {
                case,
                session_record,
                ..
            } => {
                let label = session_record.kind.as_str();
                if let Some(case) = case {
                    format!(
                        "Session record ({}) in {}: {}.",
                        label,
                        compact_text(&case.id),
                        compact_text(&session_record.summary)
                    )
                } else {
                    format!(
                        "Session record ({}): {}.",
                        label,
                        compact_text(&session_record.summary)
                    )
                }
            }
            Self::DecisionAppended { entry, .. } => {
                format!("Decision: {}.", compact_text(&entry.summary))
            }
            Self::RedirectCommitted { to_direction, .. } => format!(
                "Redirected case. New direction: {}.",
                compact_text(&to_direction.summary)
            ),
            Self::RedirectRecovered { to_direction, .. } => format!(
                "Recovered redirect. Active direction: {}.",
                compact_text(&to_direction.summary)
            ),
            Self::StepAdded { step, .. } => {
                format!("Step added: {}.", compact_step_title(&step.title))
            }
            Self::StepStarted { step, .. } => {
                format!("Step started: {}.", compact_step_title(&step.title))
            }
            Self::StepDone { step, .. } => {
                format!("Step done: {}.", compact_step_title(&step.title))
            }
            Self::StepBlocked { step, .. } => {
                format!("Step blocked: {}.", compact_step_title(&step.title))
            }
            Self::StepsReordered {
                moved_step_id,
                before_step_id,
                ..
            } => format!(
                "Reordered steps. {} before {}.",
                compact_text(moved_step_id),
                compact_text(before_step_id)
            ),
            Self::CaseClosed { summary, .. } => {
                format!("Closed case: {}.", compact_text(summary))
            }
            Self::CaseAbandoned { summary, .. } => {
                format!("Abandoned case: {}.", compact_text(summary))
            }
        }
    }
}

fn compact_step_title(text: &str) -> String {
    let primary = text.split(';').next().unwrap_or(text);
    compact_text(primary)
}

fn compact_text(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    normalized
        .trim()
        .trim_end_matches(['.', '!', '?', ';', ':'])
        .trim()
        .to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseEventEnvelope {
    pub event_id: String,
    pub case_id: String,
    pub associated_case_id: Option<String>,
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
        let associated_case_id = event.case_id().map(ToOwned::to_owned);
        let session_id = associated_case_id
            .clone()
            .unwrap_or_else(|| format!("session-{}-{}", client.repo_id(), client.worktree_id()));
        let discriminator = metadata
            .get("entry_seq")
            .or_else(|| metadata.get("session_record_seq"))
            .or_else(|| metadata.get("step_id"))
            .or_else(|| metadata.get("to_direction_seq"))
            .or_else(|| metadata.get("summary"))
            .map(Value::to_string)
            .unwrap_or_else(|| "na".to_string());
        Self {
            event_id: format!(
                "{}:{}:{}:{}",
                session_id,
                event.event_type(),
                discriminator,
                occurred_at
            ),
            case_id: session_id,
            associated_case_id,
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

#[cfg(test)]
mod tests {
    use super::CaseDomainEvent;
    use crate::types::{Case, CaseStatus, Constraint, Direction, Step, StepStatus};

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

    fn sample_step(title: &str) -> Step {
        Step {
            id: "C-1/S-1".to_string(),
            case_id: "C-1".to_string(),
            direction_seq: 2,
            order_index: 1,
            title: title.to_string(),
            status: StepStatus::Pending,
            reason: None,
            created_at: "2026-03-25T08:00:00Z".to_string(),
            updated_at: "2026-03-25T08:00:00Z".to_string(),
        }
    }

    #[test]
    fn honcho_content_trims_step_boilerplate() {
        let event = CaseDomainEvent::StepAdded {
            case: sample_case(),
            step: sample_step(
                "Record concrete evidence for the Honcho configuration error; can preserve symptoms but cannot assert root cause; done when both outcomes are stored as case facts.",
            ),
        };

        assert_eq!(
            event.honcho_content(),
            "Step added: Record concrete evidence for the Honcho configuration error."
        );
    }

    #[test]
    fn honcho_content_uses_compact_redirect_summary() {
        let event = CaseDomainEvent::RedirectCommitted {
            case: sample_case(),
            from_direction: Direction {
                case_id: "C-1".to_string(),
                seq: 1,
                summary: "Demo smoke direction.".to_string(),
                constraints: Vec::new(),
                success_condition: "done".to_string(),
                abort_condition: "stop".to_string(),
                reason: None,
                context: None,
                created_at: "2026-03-25T08:00:00Z".to_string(),
            },
            to_direction: Direction {
                case_id: "C-1".to_string(),
                seq: 2,
                summary: "Investigate intermittent Honcho-backed context recall failures."
                    .to_string(),
                constraints: Vec::new(),
                success_condition: "done".to_string(),
                abort_condition: "stop".to_string(),
                reason: None,
                context: None,
                created_at: "2026-03-25T08:00:00Z".to_string(),
            },
            entry: crate::types::Entry {
                case_id: "C-1".to_string(),
                seq: 3,
                entry_type: crate::types::EntryType::Redirect,
                kind: None,
                step_id: None,
                summary: "redirected".to_string(),
                reason: None,
                context: None,
                files: Vec::new(),
                artifacts: Vec::new(),
                created_at: "2026-03-25T08:00:00Z".to_string(),
            },
        };

        assert_eq!(
            event.honcho_content(),
            "Redirected case. New direction: Investigate intermittent Honcho-backed context recall failures."
        );
    }
}
