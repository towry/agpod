use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct KiroArgs {
    #[command(subcommand)]
    pub command: Option<KiroCommand>,

    /// Specify configuration file path
    #[arg(long, env = "AGPOD_CONFIG")]
    pub config: Option<String>,

    /// Override base directory
    #[arg(long, env = "AGPOD_BASE_DIR")]
    pub base_dir: Option<String>,

    /// Override templates directory
    #[arg(long, env = "AGPOD_TEMPLATES_DIR")]
    pub templates_dir: Option<String>,

    /// Override plugins directory
    #[arg(long, env = "AGPOD_PLUGINS_DIR")]
    pub plugins_dir: Option<String>,

    /// Log level
    #[arg(long, env = "AGPOD_LOG_LEVEL", default_value = "info")]
    pub log_level: String,

    /// Dry run (don't write files)
    #[arg(long)]
    pub dry_run: bool,

    /// Output format for list commands
    #[arg(long)]
    pub json: bool,

    // Shortcut flags for backward compatibility
    /// Create new PR draft (equivalent to pr-new)
    #[arg(long, conflicts_with = "command")]
    pub pr_new: Option<String>,

    /// List PR drafts (equivalent to pr-list)
    #[arg(long, conflicts_with = "command")]
    pub pr_list: bool,

    /// Interactive PR selection (equivalent to pr)
    #[arg(long, conflicts_with = "command")]
    pub pr: bool,
}

#[derive(Debug, Subcommand)]
pub enum KiroCommand {
    /// Create a new PR draft
    PrNew {
        /// Description for the PR draft
        #[arg(short, long)]
        desc: String,

        /// Template name to use
        #[arg(short, long)]
        template: Option<String>,

        /// Force creation even if directory exists
        #[arg(short, long)]
        force: bool,

        /// Create and checkout git branch
        #[arg(long)]
        git_branch: bool,

        /// Open in editor after creation
        #[arg(short, long)]
        open: bool,
    },

    /// List PR drafts
    PrList {
        /// Number of summary lines to extract
        #[arg(long, default_value = "3")]
        summary_lines: usize,
    },

    /// Interactive PR draft selection
    Pr {
        /// Use fzf for selection if available
        #[arg(long)]
        fzf: bool,

        /// Output format: name, rel, abs
        #[arg(long, default_value = "rel")]
        output: String,
    },

    /// Initialize agpod kiro configuration and templates
    Init {
        /// Force re-initialization even if config exists
        #[arg(short, long)]
        force: bool,
    },
}
