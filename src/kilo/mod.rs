mod cli;
mod config;
mod slug;
mod template;
mod plugin;
mod commands;
mod git;
mod error;

pub use cli::KiloArgs;


use anyhow::Result;

pub fn run(args: KiloArgs) -> Result<()> {
    commands::execute(args)
}
