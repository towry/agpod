mod cli;
mod client;
mod commands;
mod config;
mod context;
mod error;
mod events;
mod honcho;
mod hooks;
mod output;
mod repo_id;
mod rpc;
mod search;
mod server;
mod server_client;
mod types;

use std::time::Duration;

pub use cli::{
    CaseArgs, CaseCommand, CaseStatusArg, ContextScopeArg, GoalDriftFlag, OpenModeArg,
    RecallModeArg, StepCommand,
};
pub use config::{CaseAccessMode, CaseConfig, CaseOverrides, DbConfig, DEFAULT_CASE_SERVER_ADDR};
pub use server::CaseServer;
pub use types::{CaseContextHit, CaseContextResult, RecordKind};

use anyhow::Result;
use serde_json::Value;

pub(crate) const CASE_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn run(args: CaseArgs) -> Result<()> {
    commands::execute(args).await
}

pub async fn run_json(args: CaseArgs) -> Value {
    commands::execute_json(args).await
}

pub async fn run_json_batch(
    data_dir: Option<String>,
    server_addr: Option<String>,
    repo_root: Option<String>,
    commands: Vec<CaseCommand>,
) -> Vec<Value> {
    commands::execute_json_batch(
        data_dir.as_deref(),
        server_addr.as_deref(),
        repo_root.as_deref(),
        commands,
    )
    .await
}
