mod name;
mod person;
mod resource;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use oeuvre_adapter_surreal::SurrealStore;
use oeuvre_app::{AppState, CommandError, ErrorKind, Surface};

use crate::person::PersonCmd;
use crate::resource::ResourceCmd;

#[derive(Debug, Parser)]
#[command(name = "ovr", version, about = "Oeuvre citation manager")]
struct Cli {
    /// Library data directory (SurrealKV path).
    #[arg(long, global = true, env = "OEUVRE_DATA")]
    data: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Manage people in the library.
    Person(PersonCmd),
    /// Manage resources in the library.
    Resource(ResourceCmd),
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if let Some(cmd) = err.downcast_ref::<CommandError>() {
                eprintln!("error: {cmd}");
                return exit_for(cmd.kind());
            }
            eprintln!("error: {err:#}");
            ExitCode::from(1)
        }
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let data = cli.data.unwrap_or_else(default_data_dir);
    std::fs::create_dir_all(&data)
        .with_context(|| format!("create data directory {}", data.display()))?;

    let store = SurrealStore::open(&data)
        .await
        .with_context(|| format!("open library at {}", data.display()))?;
    let state = AppState::builder(store).surface(Surface::Cli).build();

    match cli.command {
        Commands::Person(cmd) => person::run(&state, cmd).await?,
        Commands::Resource(cmd) => resource::run(&state, cmd).await?,
    }
    Ok(())
}

fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("oeuvre")
}

fn exit_for(kind: ErrorKind) -> ExitCode {
    ExitCode::from(match kind {
        ErrorKind::Invalid => 2,
        ErrorKind::NotFound => 3,
        ErrorKind::Store => 4,
        ErrorKind::Internal => 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::ResourceAction;

    #[test]
    fn parses_resource_list() {
        let cli = Cli::try_parse_from(["ovr", "resource", "list"]).expect("valid args");
        assert!(matches!(
            cli.command,
            Commands::Resource(ResourceCmd {
                action: ResourceAction::List
            })
        ));
    }
}
