mod db;
mod kind;
mod library;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use oeuvre_adapter_surreal::SurrealStore;
use oeuvre_app::{AppState, Surface};

use gpui_kit::component::Theme;
use gpui_kit::component::table::TableState;
use gpui_kit::{AppContext as _, *};

use crate::db::DbWorker;
use crate::kind::AppAssets;
use crate::library::{LibraryApp, LibraryDelegate, PAGE_SIZE};

#[derive(Debug, Parser)]
#[command(name = "ovr-gui", version, about = "Oeuvre library GUI")]
struct Args {
    /// Library data directory (SurrealKV path).
    #[arg(long, global = true, env = "OEUVRE_DATA")]
    data: Option<PathBuf>,

    /// Use an in-memory store seeded with a demo library (overrides --data).
    #[arg(long, global = true)]
    memory: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let state = setup_state(&args)?;
    let worker = DbWorker::spawn(state);

    // Fetch the first page before the window opens so the grid renders full
    // immediately; scrolling drives every later page.
    let first = worker
        .list_page_blocking(None, PAGE_SIZE)
        .map_err(|e| anyhow::anyhow!("load first page: {e}"))?;

    gpui_kit::application().with_assets(AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        // Root sets the window rem size from this, so every rem-based size
        // in the kit shrinks with it.
        Theme::global_mut(cx).font_size = px(12.);
        if let Err(err) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(1100.), px(700.)),
                })),
                titlebar: Some(TitlebarOptions {
                    title: Some("Oeuvre".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let delegate = LibraryDelegate::new(worker, first);
                let table = cx.new(|cx| {
                    TableState::new(delegate, window, cx)
                        .col_movable(false)
                        .sortable(false)
                });
                cx.new(|_| LibraryApp::new(table))
            },
        ) {
            eprintln!("error: failed to open window: {err}");
            std::process::exit(1);
        }
    });
    Ok(())
}

fn setup_state(args: &Args) -> Result<AppState> {
    // The runtime is deliberately leaked: Surreal keeps engine tasks on it,
    // and it has to outlive the window. The DbWorker thread runs requests on
    // its own runtime, not this one.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("start runtime")?;
    let state = rt.block_on(async {
        if args.memory {
            let store = SurrealStore::open_memory()
                .await
                .context("open in-memory library")?;
            oeuvre_app::seed::seed_demo_library(&store, oeuvre_app::seed::DEFAULT_DEMO_LIBRARY_SIZE)
                .await
                .context("seed demo library")?;
            return Ok::<AppState, anyhow::Error>(
                AppState::builder(store).surface(Surface::Gui).build(),
            );
        }

        let data = args.data.clone().unwrap_or_else(default_data_dir);
        std::fs::create_dir_all(&data)
            .with_context(|| format!("create data directory {}", data.display()))?;
        let store = SurrealStore::open(&data)
            .await
            .with_context(|| format!("open library at {}", data.display()))?;
        Ok(AppState::builder(store).surface(Surface::Gui).build())
    })?;
    std::mem::forget(rt);
    Ok(state)
}

fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("oeuvre")
}
