//! Data structures for Case, Direction, Step, Entry.
//!
//! Keywords: case model, direction, step, entry, constraint

use serde::{Deserialize, Serialize};

/// A constraint rule with its rationale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constraint {
    pub rule: String,
    pub reason: String,
}

/// Case status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    Open,
    Closed,
    Abandoned,
}

impl CaseStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Abandoned => "abandoned",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "closed" => Some(Self::Closed),
            "abandoned" => Some(Self::Abandoned),
            _ => None,
        }
    }
}

impl std::fmt::Display for CaseStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Step status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Active,
    Done,
    Blocked,
    Skipped,
}

impl StepStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Skipped => "skipped",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "active" => Some(Self::Active),
            "done" => Some(Self::Done),
            "blocked" => Some(Self::Blocked),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }
}

impl std::fmt::Display for StepStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Entry type in the case log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryType {
    Record,
    Decision,
    Redirect,
}

impl EntryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Record => "record",
            Self::Decision => "decision",
            Self::Redirect => "redirect",
        }
    }
}

impl std::fmt::Display for EntryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Record kind (sub-type of EntryType::Record).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Note,
    Finding,
    Evidence,
    Blocker,
}

impl RecordKind {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Finding => "finding",
            Self::Evidence => "evidence",
            Self::Blocker => "blocker",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "note" => Some(Self::Note),
            "finding" => Some(Self::Finding),
            "evidence" => Some(Self::Evidence),
            "blocker" => Some(Self::Blocker),
            _ => None,
        }
    }
}

/// The Case node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Case {
    pub id: String,
    pub repo_id: String,
    pub goal: String,
    pub goal_constraints: Vec<Constraint>,
    pub status: CaseStatus,
    pub current_direction_seq: u32,
    pub current_step_id: Option<String>,
    pub opened_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub close_summary: Option<String>,
    pub abandoned_at: Option<String>,
    pub abandon_summary: Option<String>,
}

/// A direction within a case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Direction {
    pub case_id: String,
    pub seq: u32,
    pub summary: String,
    pub constraints: Vec<Constraint>,
    pub success_condition: String,
    pub abort_condition: String,
    pub reason: Option<String>,
    pub context: Option<String>,
    pub created_at: String,
}

/// A step within a direction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub case_id: String,
    pub direction_seq: u32,
    pub order_index: u32,
    pub title: String,
    pub status: StepStatus,
    pub reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// An entry (event log item) within a case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub case_id: String,
    pub seq: u32,
    pub entry_type: EntryType,
    pub kind: Option<String>,
    pub summary: String,
    pub reason: Option<String>,
    pub context: Option<String>,
    pub files: Vec<String>,
    pub artifacts: Vec<String>,
    pub created_at: String,
}

/// Suggested next action for the CLI output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextAction {
    pub suggested_command: String,
    pub why: String,
}

/// Health status for current command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    OnTrack,
    Looping,
    Blocked,
}

impl Health {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OnTrack => "on_track",
            Self::Looping => "looping",
            Self::Blocked => "blocked",
        }
    }
}

impl std::fmt::Display for Health {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
