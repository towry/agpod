use crate::types::RecordKind;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CaseError {
    #[error("not a git repository")]
    NotGitRepo,

    #[error(
        "no git remote found. Please configure a remote (e.g., `git remote add origin <url>`)"
    )]
    NoGitRemote,

    #[error("git error: {0}")]
    Git(String),

    #[error("database connection failed: {0}")]
    DbConnection(String),

    #[error("database query failed: {0}")]
    DbQuery(String),

    #[error("database init failed: {0}")]
    DbInit(String),

    #[error("repo already has an open case: {0}")]
    RepoHasOpenCase(String),

    #[error("case not found: {0}")]
    CaseNotFound(String),

    #[error("case is not open: {0}")]
    CaseNotOpen(String),

    #[error("redirect requires both success_condition and abort_condition")]
    MissingDirectionExitConditions,

    #[error("goal drift detected: close or archive the current case and open a new one instead of redirecting")]
    GoalDriftRequiresNewCase,

    #[error("step not found: {0}")]
    StepNotFound(String),

    #[error("no open case in this repository")]
    NoOpenCase,

    #[error("invalid constraint JSON: {0}")]
    InvalidConstraint(String),

    #[error("invalid query: {0}")]
    InvalidQuery(String),

    #[error("invalid list option: {0}")]
    InvalidListOption(String),

    #[error("cannot close case with unfinished steps")]
    UnfinishedSteps,

    #[error(
        "invalid record kind: {kind}; use one of {allowed}, or call `case_decide` for decisions"
    )]
    InvalidRecordKind { kind: String, allowed: String },

    #[error("record kind `goal_constraint_update` requires at least one `goal_constraint`")]
    GoalConstraintUpdateRequiresConstraints,

    #[error("`goal_constraint` is only allowed when record kind is `goal_constraint_update`")]
    GoalConstraintsOnlyAllowedForGoalConstraintUpdate,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

pub type CaseResult<T> = Result<T, CaseError>;

impl CaseError {
    pub fn invalid_record_kind(kind: impl Into<String>) -> Self {
        Self::InvalidRecordKind {
            kind: kind.into(),
            allowed: RecordKind::allowed_values_display(),
        }
    }
}
