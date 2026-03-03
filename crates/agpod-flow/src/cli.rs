//! CLI argument definitions for `agpod flow`.
//!
//! Keywords: flow cli, clap, subcommands, session flag

use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct FlowArgs {
    /// Session ID for stateful commands
    #[arg(short = 's', long, global = true)]
    pub session: Option<String>,

    /// Output as JSON
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: FlowCommand,
}

#[derive(Debug, Subcommand)]
pub enum FlowCommand {
    /// Initialize .agpod.flow.toml in repo root (idempotent)
    Init,

    /// Rebuild graph.json from documents
    Rebuild,

    /// List recent tasks (stateless)
    Recent {
        /// Number of results
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,

        /// Look-back window in days
        #[arg(long, default_value = "14")]
        days: u32,
    },

    /// Print task tree (stateless)
    Tree {
        /// Root task to start from
        #[arg(long)]
        root: Option<String>,
    },

    /// Session lifecycle commands
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },

    /// Show active task status (requires -s)
    Status,

    /// Set session focus to a task (requires -s)
    Focus {
        /// Task ID to focus on
        #[arg(long)]
        task: String,
    },

    /// Fork a sub-task (requires -s)
    Fork {
        /// New task ID
        #[arg(long)]
        to: String,

        /// Parent task ID (defaults to session active task)
        #[arg(long)]
        from: Option<String>,

        /// Don't switch focus to the new task
        #[arg(long)]
        no_switch: bool,
    },

    /// Navigate to parent task (requires -s)
    Parent,

    /// Document management commands
    Doc {
        #[command(subcommand)]
        command: DocCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Create a new session
    New,
    /// List sessions for this repo
    List,
    /// Close a session
    Close,
}

#[derive(Debug, Subcommand)]
pub enum DocCommand {
    /// Add/mount a document to a task (requires -s)
    Add {
        /// Relative path to document file (from repo root), absolute paths are not allowed
        #[arg(long)]
        path: String,

        /// Task ID (defaults to session active task)
        #[arg(long)]
        task: Option<String>,

        /// Document type
        #[arg(long, alias = "type")]
        doc_type: Option<String>,
    },

    /// Initialize frontmatter in a document
    Init {
        /// Relative path to document file (from repo root), absolute paths are not allowed
        #[arg(long)]
        path: String,

        /// Task ID (optional; auto-bootstraps T-001 if omitted)
        #[arg(long)]
        task: Option<String>,

        /// Document type
        #[arg(long, alias = "type")]
        doc_type: String,

        /// Overwrite existing frontmatter in target file
        #[arg(long)]
        force: bool,
    },

    /// Remove/unmount a document from flow graph by stripping frontmatter
    Remove {
        /// Relative path to document file (from repo root), absolute paths are not allowed
        #[arg(long)]
        path: String,
    },
}
