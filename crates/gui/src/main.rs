mod collections;
mod command_palette;
mod db;
mod detail;
mod edit;
mod icons;
mod inline_edit;
mod kind;
mod library;
mod settings;
mod tags;
mod theme;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use oeuvre_adapter_surreal::SurrealStore;
use oeuvre_app::{AppState, Surface};

use gpui_kit::component::TitleBar;
use gpui_kit::component::table::TableState;
use gpui_kit::{AppContext as _, *};

use crate::db::DbWorker;
use crate::kind::AppAssets;
use crate::library::ImportBiblatexFile;
use crate::library::{
    BeginInlineEdit, BeginNewRow, CycleColumnSort, FocusColumnFilter, FocusSearch, InlineNextField,
    InlinePrevField, LibraryApp, LibraryDelegate, OpenCommandPalette, PAGE_SIZE, ToggleDetailPanel,
    ToggleEditMode, ToggleLeftPanel,
};
use crate::settings::Settings;

#[derive(Debug, Parser)]
#[command(name = "ovr-gui", version, about = "Oeuvre library GUI")]
struct Args {
    /// Library data directory (SurrealKV path).
    #[arg(long, global = true, env = "OEUVRE_DATA")]
    data: Option<PathBuf>,

    /// Use an in-memory store seeded with a demo library (overrides --data).
    #[arg(long, global = true)]
    memory: bool,

    /// Number of demo resources to seed in memory mode (default: 5000).
    #[arg(long, requires = "memory")]
    demo_resources: Option<usize>,

    /// Print a readiness marker after opening the benchmark window.
    #[arg(long, requires = "memory", hide = true)]
    benchmark_ready: bool,
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
    let benchmark_ready = args.benchmark_ready;
    let worker = DbWorker::spawn(state);

    // Settings are app-level, so they live in the default app data
    // directory even when the library itself opens from --data elsewhere.
    let settings_dir = default_data_dir();
    let settings = Settings::load(&settings_dir);

    // Fetch the first page before the window opens so the grid renders full
    // immediately; scrolling drives every later page.
    let first = worker
        .list_page_blocking(None, PAGE_SIZE)
        .map_err(|e| anyhow::anyhow!("load first page: {e}"))?;

    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            crate::theme::apply(cx.window_appearance().into(), cx);
            // `LibraryApp` is an ancestor of every control in the window, so these
            // fire whether the table, toolbar, or sidebar holds focus. Letter keys
            // skip `Input` so typing is undisturbed. Double-Esc to clear filters
            // is a keystroke observer (works while Input owns single Esc).
            // ⌘↩ save is handled on LibraryApp via the Input `Enter` action that
            // single-line fields propagate after their own binding.
            // E = inline row edit; Enter = sidebar form; N = new draft row.
            // Letter keys skip Input so typing in cells/filters is undisturbed.
            cx.bind_keys([
                KeyBinding::new("cmd-k", OpenCommandPalette, Some("LibraryApp")),
                KeyBinding::new("cmd-shift-i", ImportBiblatexFile, Some("LibraryApp")),
                KeyBinding::new("]", ToggleDetailPanel, Some("LibraryApp && !Input")),
                KeyBinding::new("[", ToggleLeftPanel, Some("LibraryApp && !Input")),
                KeyBinding::new("space", CycleColumnSort, Some("LibraryApp && !Input")),
                KeyBinding::new("e", BeginInlineEdit, Some("LibraryApp && !Input")),
                KeyBinding::new("enter", ToggleEditMode, Some("LibraryApp && !Input")),
                KeyBinding::new("n", BeginNewRow, Some("LibraryApp && !Input")),
                KeyBinding::new("f", FocusColumnFilter, Some("LibraryApp && !Input")),
                KeyBinding::new(
                    "/",
                    FocusSearch,
                    Some("LibraryApp && !Input && !InlineEdit"),
                ),
                // Tab must beat Input's IndentInline/OutdentInline (same depth as
                // `Input`, later registration wins) and DataTable's column select
                // (`InlineEdit > Combobox` is deeper than `DataTable`). Without
                // these, Tab no-ops on single-line fields and Shift+Tab loops the
                // table onto Added/Edited — leaving the edit row.
                KeyBinding::new("tab", InlineNextField, Some("InlineEdit > Input")),
                KeyBinding::new("shift-tab", InlinePrevField, Some("InlineEdit > Input")),
                KeyBinding::new("tab", InlineNextField, Some("InlineEdit > Combobox")),
                KeyBinding::new("shift-tab", InlinePrevField, Some("InlineEdit > Combobox")),
                KeyBinding::new("tab", InlineNextField, Some("InlineEdit > DataTable")),
                KeyBinding::new("shift-tab", InlinePrevField, Some("InlineEdit > DataTable")),
            ]);
            if let Err(err) = gpui_kit::open_window(
                // The window draws its own title bar (the toolbar inside
                // LibraryApp), so the native one is transparent with only the
                // traffic lights left in place.
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(1100.), px(700.)),
                    })),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Oeuvre".into()),
                        // Center the 14px native controls in the 40px toolbar.
                        traffic_light_position: Some(point(px(9.), px(13.))),
                        ..TitleBar::title_bar_options()
                    }),
                    // Required on macOS: otherwise the system also handles
                    // title bar double clicks and delays title bar clicks to
                    // disambiguate double clicks.
                    app_owns_titlebar_drag: true,
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let delegate = LibraryDelegate::new(worker, first, settings);
                    let table = cx.new(|cx| {
                        TableState::new(delegate, window, cx)
                            .col_movable(false)
                            .sortable(false)
                    });
                    // Key bindings dispatch along the focus path, so the table
                    // must hold focus from the first frame for `]` to work
                    // before any click.
                    let focus_handle = table.focus_handle(cx);
                    window.focus(&focus_handle, cx);
                    cx.new(|cx| LibraryApp::new(table, settings_dir, window, cx))
                },
            ) {
                eprintln!("error: failed to open window: {err}");
                std::process::exit(1);
            }
            if benchmark_ready {
                println!("OEUVRE_GUI_READY");
                std::io::stdout().flush().expect("flush readiness marker");
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
            // Memory-only storage is too costly for large libraries: the 100k-item
            // demo used ~1.37 GiB RSS. Keep this mode for demos and benchmarks.
            let store = SurrealStore::open_memory()
                .await
                .context("open in-memory library")?;
            oeuvre_app::seed::seed_demo_library(
                &store,
                args.demo_resources
                    .unwrap_or(oeuvre_app::seed::DEFAULT_DEMO_LIBRARY_SIZE),
            )
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

#[cfg(test)]
mod benchmark_args_tests {
    use super::*;

    #[tokio::test]
    async fn demo_resources_requires_memory_mode() {
        assert!(Args::try_parse_from(["ovr-gui", "--demo-resources", "1000"]).is_err());
        let args = Args::try_parse_from(["ovr-gui", "--memory", "--demo-resources", "1000"])
            .expect("benchmark seed count");
        assert_eq!(args.demo_resources, Some(1000));
    }

    #[tokio::test]
    async fn benchmark_ready_requires_memory_mode() {
        assert!(Args::try_parse_from(["ovr-gui", "--benchmark-ready"]).is_err());
    }
}
