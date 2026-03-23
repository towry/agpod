//! CLI argument definitions for `agpod case`.
//!
//! Keywords: cli, clap, subcommand, case args

use clap::{Args, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalDriftFlag {
    Yes,
    No,
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Subcommand)]
pub enum CaseCommand {
    /// Open a new exploration case
    Open {
        /// The goal (immutable once set)
        #[arg(long)]
        goal: String,

        /// Initial direction summary
        #[arg(long)]
        direction: String,

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
    },

    /// Show current case navigation panel
    Current,

    /// Record a fact, finding, evidence, or blocker
    Record {
        /// Case ID (e.g., C-550e8400-e29b-41d4-a716-446655440000)
        #[arg(long)]
        id: String,

        /// Summary of the record
        #[arg(long)]
        summary: String,

        /// Kind: note, finding, evidence, blocker
        #[arg(long, default_value = "note")]
        kind: String,

        /// Related file paths (comma-separated)
        #[arg(long)]
        files: Option<String>,

        /// Additional context
        #[arg(long)]
        context: Option<String>,
    },

    /// Record a decision
    Decide {
        /// Case ID
        #[arg(long)]
        id: String,

        /// Decision summary
        #[arg(long)]
        summary: String,

        /// Reason for the decision
        #[arg(long)]
        reason: String,
    },

    /// Change direction
    Redirect {
        /// Case ID
        #[arg(long)]
        id: String,

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
        /// Case ID
        #[arg(long)]
        id: String,

        /// Close summary
        #[arg(long)]
        summary: String,
    },

    /// Abandon a case
    Abandon {
        /// Case ID
        #[arg(long)]
        id: String,

        /// Abandon summary
        #[arg(long)]
        summary: String,
    },

    /// Manage execution steps
    Step {
        #[command(subcommand)]
        command: StepCommand,
    },

    /// Search past cases
    Recall {
        /// Search query
        query: String,
    },

    /// List all cases for this repository
    List,

    /// Resume brief for handoff
    Resume {
        /// Case ID (defaults to open case)
        #[arg(long)]
        id: Option<String>,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Subcommand)]
pub enum StepCommand {
    /// Add a new step to the current direction
    Add {
        /// Case ID
        #[arg(long)]
        id: String,

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
        /// Case ID
        #[arg(long)]
        id: String,

        /// Step ID (e.g., S-001)
        #[arg(long)]
        step_id: String,
    },

    /// Mark a step as done
    Done {
        /// Case ID
        #[arg(long)]
        id: String,

        /// Step ID
        #[arg(long)]
        step_id: String,
    },

    /// Reorder a step
    Move {
        /// Case ID
        #[arg(long)]
        id: String,

        /// Step ID to move
        #[arg(long)]
        step_id: String,

        /// Place before this step ID
        #[arg(long)]
        before: String,
    },

    /// Mark a step as blocked
    Block {
        /// Case ID
        #[arg(long)]
        id: String,

        /// Step ID
        #[arg(long)]
        step_id: String,

        /// Reason for blocking
        #[arg(long)]
        reason: String,
    },
}
