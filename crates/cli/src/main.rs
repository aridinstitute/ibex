mod collection;
mod config;
mod import;
mod name;
mod person;
mod resource;
mod tag;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use oeuvre_adapter_surreal::SurrealStore;
use oeuvre_app::{AppState, CommandError, ErrorKind, Surface};

use crate::collection::CollectionCmd;
use crate::config::ConfigCmd;
use crate::import::ImportArgs;
use crate::person::PersonCmd;
use crate::resource::ResourceCmd;
use crate::tag::TagCmd;

#[derive(Debug, Parser)]
#[command(name = "ovr", version, about = "Oeuvre citation manager")]
struct Cli {
    /// Library data directory (SurrealKV path).
    #[arg(long, global = true, env = "OEUVRE_DATA")]
    data: Option<PathBuf>,

    /// Use an in-memory store seeded with a demo library (overrides --data).
    #[arg(long, global = true)]
    memory: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Manage collections in the library.
    Collection(CollectionCmd),
    /// Manage classification settings shared with the desktop app.
    Config(ConfigCmd),
    /// Import a BibLaTeX or CSL-JSON bibliography, optionally adding every
    /// entry to collections and tags.
    Import(ImportArgs),
    /// Manage people in the library.
    Person(PersonCmd),
    /// Manage resources in the library.
    Resource(ResourceCmd),
    /// Manage tags in the library.
    Tag(TagCmd),
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
    if matches!(cli.command, Commands::Config(_)) {
        let Commands::Config(cmd) = cli.command else {
            unreachable!()
        };
        return config::run(cmd);
    }
    let state = open_state(&cli).await?;

    match cli.command {
        Commands::Config(_) => unreachable!("handled before opening the library"),
        Commands::Collection(cmd) => collection::run(&state, cmd).await?,
        Commands::Import(args) => import::run(&state, args).await?,
        Commands::Person(cmd) => person::run(&state, cmd).await?,
        Commands::Resource(cmd) => resource::run(&state, cmd).await?,
        Commands::Tag(cmd) => tag::run(&state, cmd).await?,
    }
    Ok(())
}

async fn open_state(cli: &Cli) -> Result<AppState> {
    if cli.memory {
        let store = SurrealStore::open_memory()
            .await
            .context("open in-memory library")?;
        oeuvre_app::seed::seed_demo_library(&store, oeuvre_app::seed::DEFAULT_DEMO_LIBRARY_SIZE)
            .await
            .context("seed demo library")?;
        return Ok(AppState::builder(store).surface(Surface::Cli).build());
    }

    let data = cli.data.clone().unwrap_or_else(default_data_dir);
    std::fs::create_dir_all(&data)
        .with_context(|| format!("create data directory {}", data.display()))?;

    let store = SurrealStore::open(&data)
        .await
        .with_context(|| format!("open library at {}", data.display()))?;
    Ok(AppState::builder(store).surface(Surface::Cli).build())
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
                action: ResourceAction::List(_)
            })
        ));
    }

    #[test]
    fn parses_memory_flag() {
        let cli = Cli::try_parse_from(["ovr", "--memory", "resource", "list"]).expect("valid args");
        assert!(cli.memory);
    }

    #[test]
    fn parses_resource_update() {
        let cli = Cli::try_parse_from([
            "ovr",
            "resource",
            "update",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "--title",
            "Retitled",
        ])
        .expect("valid args");
        let Commands::Resource(ResourceCmd {
            action: ResourceAction::Update(args),
        }) = cli.command
        else {
            panic!("expected update action");
        };
        assert_eq!(args.id, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(args.title.as_deref(), Some("Retitled"));
    }

    #[test]
    fn parses_import_with_repeatable_assignment_flags() {
        let cli = Cli::try_parse_from([
            "ovr",
            "import",
            "references.json",
            "--format",
            "csl-json",
            "--collection",
            "Research",
            "--collection-id",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "--tag",
            "review",
            "--tag",
            "2026",
            "--tag-id",
            "01ARZ3NDEKTSV4RRFFQ69G5FAW",
        ])
        .expect("valid args");
        let Commands::Import(args) = cli.command else {
            panic!("expected import");
        };
        assert_eq!(args.path, PathBuf::from("references.json"));
        assert_eq!(args.format, Some(crate::import::ImportFormatArg::CslJson));
        assert_eq!(args.collections, vec!["Research"]);
        assert_eq!(args.collection_ids.len(), 1);
        assert_eq!(args.tags, vec!["review", "2026"]);
        assert_eq!(args.tag_ids.len(), 1);
    }
}
