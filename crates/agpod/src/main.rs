use agpod_case as case;
use agpod_core::init_logging;
use agpod_diff as diff;
use agpod_vcs_path as vcs_path;
use clap::{Args, Parser, Subcommand};
use tracing::warn;

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
    Case(Box<case::CaseArgs>),
    /// Run the case server for shared database access.
    CaseServer(CaseServerArgs),
    /// Format paths with VCS (Git/Jujutsu) branch/bookmark information
    VcsPathInfo(vcs_path::VcsPathInfoArgs),
}

#[derive(Args)]
struct CaseServerArgs {
    /// SurrealDB data directory (default: shared case config)
    #[arg(long, env = "AGPOD_CASE_DATA_DIR")]
    data_dir: Option<String>,

    /// Case server address (default: shared case config)
    #[arg(long, env = "AGPOD_CASE_SERVER_ADDR")]
    server_addr: Option<String>,
}

#[tokio::main]
async fn main() {
    if let Err(error) = init_logging("agpod") {
        eprintln!("Warning: failed to initialize logging: {error}");
    }

    let cli = Cli::parse();
    warn!("agpod started");

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
            if let Err(e) = case::run(*args).await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Some(Commands::CaseServer(args)) => {
            let config = case::CaseConfig::load(case::CaseOverrides {
                data_dir: args.data_dir.as_deref(),
                server_addr: args.server_addr.as_deref(),
            });
            let server = match case::CaseServer::new(config).await {
                Ok(server) => server,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            };
            if let Err(e) = server.serve().await {
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
