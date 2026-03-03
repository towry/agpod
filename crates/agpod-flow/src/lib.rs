mod cli;
mod commands;
mod config;
mod error;
mod frontmatter;
mod graph;
mod recent;
mod repo_id;
mod scanner;
mod session;
mod storage;

pub use cli::FlowArgs;

use anyhow::Result;

pub fn run(args: FlowArgs) -> Result<()> {
    commands::execute(args)
}
