mod collection;
mod config;
mod import;
mod name;
mod note;
mod person;
mod resource;
mod tag;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use ibex_adapter_surreal::SurrealStore;
use ibex_app::config::ConfigFile;
use ibex_app::paths::{AppPaths, Paths};
use ibex_app::{AppState, CommandError, ErrorKind, Surface};

use crate::collection::CollectionCmd;
use crate::config::ConfigCmd;
use crate::import::ImportArgs;
use crate::person::PersonCmd;
use crate::resource::ResourceCmd;
use crate::tag::TagCmd;

#[derive(Debug, Parser)]
#[command(name = "ibex", version, about = "Ibex citation manager")]
struct Cli {
    /// Library database directory (SurrealKV). Defaults to the platform
    /// data directory; backups then go to a sibling `<dir>-backups`.
    #[arg(long, global = true, env = "IBEX_DATA")]
    data: Option<PathBuf>,

    /// Use a throwaway in-memory library seeded with demo data; nothing is
    /// written to disk (overrides --data).
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

/// SurrealDB's embedded engine recommends 10 MiB worker stacks; the 2 MiB
/// default overflows in debug builds.
const RUNTIME_STACK_BYTES: usize = 10 * 1024 * 1024;

fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(RUNTIME_STACK_BYTES)
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("error: start runtime: {err}");
            return ExitCode::from(1);
        }
    };
    match runtime.block_on(run()) {
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
    let paths = AppPaths::resolve_from_env(cli.data.clone())?;
    let config = ConfigFile::new(&paths.config_file);
    if matches!(cli.command, Commands::Config(_)) {
        let Commands::Config(cmd) = cli.command else {
            unreachable!()
        };
        return config::run(cmd, &config);
    }
    let state = open_state(&cli, &paths).await?;

    match cli.command {
        Commands::Config(_) => unreachable!("handled before opening the library"),
        Commands::Collection(cmd) => collection::run(&state, cmd).await?,
        Commands::Import(args) => import::run(&state, args).await?,
        Commands::Person(cmd) => person::run(&state, cmd).await?,
        Commands::Resource(cmd) => resource::run(&state, cmd, &config).await?,
        Commands::Tag(cmd) => tag::run(&state, cmd).await?,
    }
    Ok(())
}

async fn open_state(cli: &Cli, paths: &Paths) -> Result<AppState> {
    if cli.memory {
        let store = SurrealStore::open_memory()
            .await
            .context("open in-memory library")?;
        ibex_app::seed::seed_demo_library(&store, ibex_app::seed::DEFAULT_DEMO_LIBRARY_SIZE)
            .await
            .context("seed demo library")?;
        return Ok(AppState::builder(store).surface(Surface::Cli).build());
    }

    // The adapter's message already names the path and, for a library
    // another process holds, what to do about it.
    let store = SurrealStore::open(&paths.database)
        .await
        .map_err(|err| anyhow!(err.message))?;
    Ok(AppState::builder(store).surface(Surface::Cli).build())
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
        let cli = Cli::try_parse_from(["ibex", "resource", "list"]).expect("valid args");
        assert!(matches!(
            cli.command,
            Commands::Resource(ResourceCmd {
                action: ResourceAction::List(_)
            })
        ));
    }

    #[test]
    fn parses_memory_flag() {
        let cli =
            Cli::try_parse_from(["ibex", "--memory", "resource", "list"]).expect("valid args");
        assert!(cli.memory);
    }

    #[test]
    fn parses_resource_update() {
        let cli = Cli::try_parse_from([
            "ibex",
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
            "ibex",
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

    #[test]
    fn parses_resource_note_add() {
        let cli = Cli::try_parse_from([
            "ibex",
            "resource",
            "note",
            "add",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "--kind",
            "quote",
            "--body",
            "A line",
            "--location",
            "page=12",
        ])
        .expect("valid args");
        let Commands::Resource(ResourceCmd {
            action: ResourceAction::Note(note),
        }) = cli.command
        else {
            panic!("expected resource note");
        };
        assert!(matches!(
            note.action,
            crate::note::NoteAction::Add(args) if args.resource == "01ARZ3NDEKTSV4RRFFQ69G5FAV"
                && matches!(args.kind, crate::note::NoteKindArg::Quote)
                && args.body.as_deref() == Some("A line")
                && args.locations == ["page=12"]
        ));
    }

    #[test]
    fn parses_resource_update_attach_and_fetch() {
        let cli = Cli::try_parse_from([
            "ibex",
            "resource",
            "update",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "--attach",
            "/tmp/paper.pdf",
            "--fetch",
            "9780306406157",
        ])
        .expect("valid args");
        let Commands::Resource(ResourceCmd {
            action: ResourceAction::Update(args),
        }) = cli.command
        else {
            panic!("expected resource update");
        };
        assert_eq!(args.id, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(args.attachments, [PathBuf::from("/tmp/paper.pdf")]);
        assert_eq!(args.fetch.as_deref(), Some("9780306406157"));
    }
}
