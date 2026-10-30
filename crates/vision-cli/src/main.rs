//! Command-line interface for Vision.

mod cli;
mod commands;
mod output;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    if let Err(error) = commands::run(cli.command) {
        eprintln!("vision: {error}");
        std::process::exit(1);
    }
}
