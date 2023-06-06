#![deny(clippy::all)]

use elusive::cli;
use elusive::cli::Args;

use anyhow::Result;
use clap::Parser;
use env_logger::Env;

/// Entrypoint of the program
#[cfg(not(tarpaulin))]
fn main() -> Result<()> {
    let env = Env::default().filter_or("RUST_LOG", "info");
    env_logger::init_from_env(env);

    let args = Args::parse();
    cli::elusive(args)?;

    Ok(())
}
