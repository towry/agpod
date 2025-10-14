mod cli;
mod commands;
mod config;
mod error;
mod git;
mod plugin;
mod slug;
mod template;

pub use cli::KiloArgs;

use anyhow::Result;

pub fn run(args: KiloArgs) -> Result<()> {
    commands::execute(args)
}
