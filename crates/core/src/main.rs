//! Small diagnostic entrypoint for the `tellur-core` crate.
//!
//! End-user commands live in the `tellur` binary (`crates/cli`). This binary is
//! intentionally narrow so packaging never ships a placeholder executable.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "tellur-core",
    version,
    about = "Internal Tellur core diagnostics",
    long_about = "Internal diagnostics for the Tellur core library.\n\nUse the `tellur` binary for normal CLI workflows."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Print a minimal health marker for packaging smoke tests.
    Doctor,
}

fn main() {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Doctor) {
        Command::Doctor => println!("tellur-core ok"),
    }
}
