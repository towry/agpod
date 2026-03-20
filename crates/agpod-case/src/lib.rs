mod cli;
mod client;
mod commands;
mod config;
mod error;
mod output;
mod repo_id;
mod types;

pub use cli::CaseArgs;

use anyhow::Result;

pub async fn run(args: CaseArgs) -> Result<()> {
    commands::execute(args).await
}
