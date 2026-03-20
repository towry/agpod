use agpod_case as case;
use agpod_diff as diff;
use agpod_vcs_path as vcs_path;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "agpod")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"), long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Minimize git diff for LLM context (reads from stdin)
    Diff {
        /// Save diff chunks to separate files
        #[arg(long)]
        save: bool,

        /// Specify custom output directory
        #[arg(long)]
        save_path: Option<String>,

        /// Add context information to REVIEW.md (e.g., reference documentation)
        #[arg(long)]
        context: Option<String>,
    },
    /// Track exploration cases: open/close/redirect goals, record findings, manage steps. Use `--json` for machine output. All args are `--key value` (no positional).
    Case(case::CaseArgs),
    /// Format paths with VCS (Git/Jujutsu) branch/bookmark information
    VcsPathInfo(vcs_path::VcsPathInfoArgs),
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Diff {
            save,
            save_path,
            context,
        }) => {
            // Process git diff from stdin
            match diff::process_git_diff(save, save_path, context) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Case(args)) => {
            if let Err(e) = case::run(args).await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Some(Commands::VcsPathInfo(args)) => {
            if let Err(e) = vcs_path::run(args).await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        None => {
            // No command provided, print help
            use clap::CommandFactory;
            let _ = Cli::command().print_help();
            println!(); // Add a newline after help
        }
    }
}
