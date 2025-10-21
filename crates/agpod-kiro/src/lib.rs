mod cli;
mod commands;
mod config;
mod error;
mod git;
mod plugin;
mod slug;
mod template;

pub use cli::KiroArgs;

use anyhow::Result;

pub fn run(args: KiroArgs) -> Result<()> {
    commands::execute(args)
}
