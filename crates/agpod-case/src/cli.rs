//! CLI argument definitions for `agpod case`.
//!
//! Keywords: cli, clap, subcommand, case args

use clap::{Args, Subcommand, ValueEnum};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalDriftFlag {
    Yes,
    No,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaseStatusArg {
    Open,
    Closed,
    Abandoned,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    ValueEnum,
    serde::Serialize,
    serde::Deserialize,
    JsonSchema,
    Default,
)]
#[serde(rename_all = "lowercase")]
pub enum OpenModeArg {
    #[default]
    New,
    Reopen,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    ValueEnum,
    serde::Serialize,
    serde::Deserialize,
    JsonSchema,
    Default,
)]
#[serde(rename_all = "lowercase")]
pub enum ContextScopeArg {
    #[default]
    Case,
    Repo,
}

#[derive(Debug, Args)]
pub struct CaseArgs {
    /// SurrealDB data directory (default: $XDG_DATA_HOME/agpod/case.db)
    #[arg(long, env = "AGPOD_CASE_DATA_DIR", global = true)]
    pub data_dir: Option<String>,

    /// Case server address (default: 127.0.0.1:6142)
    #[arg(long, env = "AGPOD_CASE_SERVER_ADDR", global = true)]
    pub server_addr: Option<String>,

    /// Override repo root for identity resolution (default: current directory)
    #[arg(long, global = true)]
    pub repo_root: Option<String>,

    /// Output as JSON
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: CaseCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, Args)]
pub struct NeededContextQueryArg {
    #[serde(default)]
    #[arg(long = "how-to")]
    pub how_to: Vec<String>,
    #[serde(default)]
    #[arg(long = "doc-about")]
    pub doc_about: Vec<String>,
    #[serde(default)]
    #[arg(long = "pitfalls-about")]
    pub pitfalls_about: Vec<String>,
    #[serde(default)]
    #[arg(long = "known-patterns-for")]
    pub known_patterns_for: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Subcommand)]
pub enum CaseCommand {
    /// Open a new exploration case
    Open {
        /// Open mode: create a new case or reopen a previously closed/abandoned one
        #[arg(long, default_value = "new")]
        mode: OpenModeArg,

        /// Existing case ID to reopen (required when --mode reopen)
        #[arg(long = "case-id")]
        case_id: Option<String>,

        /// The goal (immutable once set; required when --mode new)
        #[arg(long)]
        goal: Option<String>,

        /// Initial direction summary (required when --mode new)
        #[arg(long)]
        direction: Option<String>,

        /// Goal-level constraint (JSON: {"rule":"...","reason":"..."})
        #[arg(long = "goal-constraint")]
        goal_constraints: Vec<String>,

        /// Direction-level constraint (JSON: {"rule":"...","reason":"..."})
        #[arg(long = "constraint")]
        constraints: Vec<String>,

        /// Success condition for the initial direction
        #[arg(long = "success-condition")]
        success_condition: Option<String>,

        /// Abort condition for the initial direction
        #[arg(long = "abort-condition")]
        abort_condition: Option<String>,

        /// Startup memory query: how-to topics
        #[serde(default)]
        #[arg(long = "how-to")]
        how_to: Vec<String>,

        /// Startup memory query: document topics
        #[serde(default)]
        #[arg(long = "doc-about")]
        doc_about: Vec<String>,

        /// Startup memory query: pitfall topics
        #[serde(default)]
        #[arg(long = "pitfalls-about")]
        pitfalls_about: Vec<String>,

        /// Startup memory query: known pattern topics
        #[serde(default)]
        #[arg(long = "known-patterns-for")]
        known_patterns_for: Vec<String>,

        /// Initial step spec. Repeatable; each value may be plain text or JSON like {"title":"...","reason":"...","start":true}; at most one step may set start=true
        #[serde(default)]
        #[arg(long = "step")]
        steps: Vec<String>,
    },

    /// Show current case navigation panel
    Current {
        /// Return only the current case state for fast CI checks
        #[arg(long)]
        state: bool,
    },

    /// Record a session fact; may optionally associate with a case
    #[command(name = "session_record")]
    SessionRecord {
        /// Case ID to associate. If omitted, uses the current open case when available.
        #[arg(long)]
        id: Option<String>,

        /// Summary of the record
        #[arg(long)]
        summary: String,

        /// Kind of record to append
        #[arg(long, default_value = "note")]
        kind: String,

        /// Goal-level constraint update payload (JSON: {"rule":"...","reason":"..."})
        #[arg(long = "goal-constraint")]
        goal_constraints: Vec<String>,

        /// Related file paths (comma-separated)
        #[arg(long)]
        files: Option<String>,

        /// Additional context
        #[arg(long)]
        context: Option<String>,
    },

    /// Record a decision
    Decide {
        /// Case ID (defaults to the open case)
        #[arg(long)]
        id: Option<String>,

        /// Decision summary
        #[arg(long)]
        summary: String,

        /// Reason for the decision
        #[arg(long)]
        reason: String,
    },

    /// Change direction
    Redirect {
        /// Case ID (defaults to the open case)
        #[arg(long)]
        id: Option<String>,

        /// New direction summary
        #[arg(long)]
        direction: String,

        /// Why we are redirecting
        #[arg(long)]
        reason: String,

        /// Context from prior work
        #[arg(long)]
        context: String,

        /// Explicitly acknowledge whether the proposed redirect has drifted away from the immutable case goal
        #[arg(long, value_enum)]
        is_drift_from_goal: GoalDriftFlag,

        /// Direction-level constraint (JSON: {"rule":"...","reason":"..."})
        #[arg(long = "constraint")]
        constraints: Vec<String>,

        /// Success condition for the new direction
        #[arg(long = "success-condition")]
        success_condition: String,

        /// Abort condition for the new direction
        #[arg(long = "abort-condition")]
        abort_condition: String,
    },

    /// Show full case details
    Show {
        /// Case ID (defaults to open case)
        #[arg(long)]
        id: Option<String>,
    },

    /// Close a case successfully
    Close {
        /// Case ID (defaults to the open case)
        #[arg(long)]
        id: Option<String>,

        /// Close summary
        #[arg(long)]
        summary: String,

        /// Confirmation token returned by a prior close attempt
        #[arg(long = "confirm-token")]
        confirm_token: Option<String>,
    },

    /// Abandon a case
    Abandon {
        /// Case ID (defaults to the open case)
        #[arg(long)]
        id: Option<String>,

        /// Abandon summary
        #[arg(long)]
        summary: String,

        /// Confirmation token returned by a prior abandon attempt
        #[arg(long = "confirm-token")]
        confirm_token: Option<String>,
    },

    /// Manage execution steps
    Step {
        #[command(subcommand)]
        command: StepCommand,
    },

    /// Recall historical records when you need raw matches (cases + session records)
    Recall {
        /// Search query used for case/session-record matching
        query: String,

        /// Filter case results by status. Note: when set, session_record hits are omitted.
        #[arg(long, value_enum)]
        status: Option<CaseStatusArg>,

        /// Limit result count
        #[arg(long)]
        limit: Option<usize>,

        /// Only include cases updated within the last N days
        #[arg(long = "recent-days")]
        recent_days: Option<u32>,
    },

    /// Build a context brief when you need a ready-to-use summary instead of raw match lists
    Context {
        /// Case ID (defaults to open case)
        #[arg(long)]
        id: Option<String>,

        /// Retrieval scope: current case or current repo across sessions
        #[arg(long, value_enum, default_value = "repo")]
        #[serde(default)]
        scope: ContextScopeArg,

        /// Optional query used to rank and include relevant hits in the brief
        #[arg(long)]
        query: Option<String>,

        /// Max number of hits to return
        #[arg(long)]
        limit: Option<usize>,

        /// Optional token budget for returned context
        #[arg(long = "token-limit")]
        token_limit: Option<u32>,
    },

    /// List all cases for this repository
    List {
        /// Filter by case status
        #[arg(long, value_enum)]
        status: Option<CaseStatusArg>,

        /// Limit result count
        #[arg(long)]
        limit: Option<usize>,

        /// Only include cases updated within the last N days
        #[arg(long = "recent-days")]
        recent_days: Option<u32>,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Subcommand)]
pub enum StepCommand {
    /// Add a new step to the current direction
    Add {
        /// Case ID (defaults to the open case)
        #[arg(long)]
        id: Option<String>,

        /// Step title
        #[arg(long)]
        title: String,

        /// Reason for adding this step
        #[arg(long)]
        reason: Option<String>,

        /// Start the step immediately after creating it
        #[arg(long)]
        start: bool,
    },

    /// Start (activate) a step
    Start {
        /// Case ID (defaults to the open case)
        #[arg(long)]
        id: Option<String>,

        /// Step ID (e.g., S-001)
        #[arg(long)]
        step_id: String,
    },

    /// Mark a step as done
    Done {
        /// Case ID (defaults to the open case)
        #[arg(long)]
        id: Option<String>,

        /// Step ID
        #[arg(long)]
        step_id: String,
    },

    /// Reorder a step
    Move {
        /// Case ID (defaults to the open case)
        #[arg(long)]
        id: Option<String>,

        /// Step ID to move
        #[arg(long)]
        step_id: String,

        /// Place before this step ID
        #[arg(long)]
        before: String,
    },

    /// Mark a step as blocked
    Block {
        /// Case ID (defaults to the open case)
        #[arg(long)]
        id: Option<String>,

        /// Step ID
        #[arg(long)]
        step_id: String,

        /// Reason for blocking
        #[arg(long)]
        reason: String,
    },

    /// Complete the current active step, optionally record a fact, and optionally start the next step
    Advance {
        /// Case ID (defaults to the open case)
        #[arg(long)]
        id: Option<String>,

        /// Step ID (defaults to the current active step)
        #[arg(long)]
        step_id: Option<String>,

        /// Record summary to append while advancing
        #[arg(long = "record-summary")]
        record_summary: Option<String>,

        /// Record kind
        #[arg(long = "record-kind")]
        record_kind: Option<String>,

        /// Related file path; repeatable
        #[arg(long = "record-file")]
        record_files: Vec<String>,

        /// Record context
        #[arg(long = "record-context")]
        record_context: Option<String>,

        /// Explicit next step to start after completion
        #[arg(long = "next-step-id")]
        next_step_id: Option<String>,

        /// Automatically start the next pending step by `order_index`
        #[arg(long = "next-step-auto")]
        next_step_auto: bool,
    },
}
