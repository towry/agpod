mod cli;
mod client;
mod commands;
mod config;
mod error;
mod output;
mod repo_id;
mod types;

pub use cli::{CaseArgs, CaseCommand, GoalDriftFlag, StepCommand};

use anyhow::Result;
use serde_json::Value;

pub async fn run(args: CaseArgs) -> Result<()> {
    commands::execute(args).await
}

pub async fn run_json(args: CaseArgs) -> Value {
    commands::execute_json(args).await
}
