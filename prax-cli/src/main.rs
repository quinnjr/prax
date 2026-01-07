//! Prax CLI - Command-line interface for the Prax ORM.

use clap::Parser;

use prax_cli::cli::{Cli, Command};
use prax_cli::commands;
use prax_cli::error::CliResult;
use prax_cli::output;

#[tokio::main]
async fn main() {
    // Run the CLI and handle errors
    if let Err(e) = run().await {
        output::newline();
        output::error(&e.to_string());
        std::process::exit(1);
    }
}

async fn run() -> CliResult<()> {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Run the appropriate command
    match cli.command {
        Command::Init(args) => commands::init::run(args).await,
        Command::Generate(args) => commands::generate::run(args).await,
        Command::Validate(args) => commands::validate::run(args).await,
        Command::Format(args) => commands::format::run(args).await,
        Command::Migrate(args) => commands::migrate::run(args).await,
        Command::Db(args) => commands::db::run(args).await,
        Command::Import(args) => commands::import::run(args).await,
        Command::Version => commands::version::run().await,
    }
}
