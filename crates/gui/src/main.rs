mod classification_rules;
mod collections;
mod command_palette;
mod db;
mod detail;
mod documentation;
mod edit;
mod icons;
mod import_dialog;
mod inline_edit;
mod jev;
mod kind;
mod library;
mod metadata;
mod notes;
mod settings;
mod tags;
mod theme;
mod theme_editor;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use ibex_adapter_surreal::SurrealStore;
use ibex_app::paths::{AppPaths, CONFIG_DIR_ENV, Paths};
use ibex_app::{AppState, Surface};
use ibex_core::{ResourceGridParent, ResourceQuery};

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
#[command(name = "ibex-gui", version, about = "Ibex library GUI")]
struct Args {
    /// Library database directory (SurrealKV). Defaults to the platform
    /// data directory; backups then go to a sibling `<dir>-backups`.
    #[arg(long, global = true, env = "IBEX_DATA")]
    data: Option<PathBuf>,

    /// Use a throwaway in-memory library seeded with demo data; nothing is
    /// written to disk (overrides --data).
    #[arg(long, global = true)]
    memory: bool,
}

/// SurrealDB's embedded engine recommends 10 MiB worker stacks; the 2 MiB
/// default overflows in debug builds.
pub(crate) const RUNTIME_STACK_BYTES: usize = 10 * 1024 * 1024;

/// Paths for this launch, from the same resolver the CLI uses, so both
/// clients share one library and one config file.
fn resolve_paths(
    args: &Args,
    bases: &AppPaths,
    config_dir_override: Option<PathBuf>,
) -> Result<Paths> {
    Ok(bases.resolve(args.data.clone(), config_dir_override)?)
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
    let started = (|| {
        let paths = resolve_paths(
            &args,
            &AppPaths::system(),
            std::env::var_os(CONFIG_DIR_ENV).map(PathBuf::from),
        )?;
        let state = setup_state(&args, &paths)?;
        let worker = DbWorker::spawn(state);
        // Fetch the first page before the window opens so the grid renders
        // full immediately; scrolling drives every later page.
        let first = worker
            .list_grid_page_blocking(
                ResourceGridParent::Roots,
                ResourceQuery::newest_first(),
                0,
                PAGE_SIZE,
            )
            .map_err(|e| anyhow!("load first page: {e}"))?;
        Ok::<_, anyhow::Error>((paths, worker, first))
    })();
    let (paths, worker, first) = match started {
        Ok(started) => started,
        Err(err) => {
            // Never fall back to an empty or in-memory library: the user
            // must see that their library did not open, and why.
            eprintln!("error: {err:#}");
            show_startup_error(format!("{err:#}"));
            return Err(err);
        }
    };

    // Settings share the config file with the CLI, which never moves
    // with --data.
    let settings_dir = paths.config_dir().to_path_buf();
    let settings = Settings::load(&settings_dir);

    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            apply_app_icon();
            crate::theme::apply(cx.window_appearance().into(), &settings.theme, cx);
            bind_library_keys(cx);
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
                        title: Some("Ibex".into()),
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
                            .select_on_modifier_click(false)
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
        });
    Ok(())
}

/// The ibex logo, embedded so the Dock shows it even though the app ships
/// as a bare binary without a bundle.
#[cfg(target_os = "macos")]
const APP_ICON_PNG: &[u8] = include_bytes!("../assets/app-icon.png");

/// Set the macOS app (Dock) icon from the embedded logo. Other platforms
/// take their icon from bundle metadata, so there is nothing to do.
/// Call only after GPUI creates its shared GPUIApplication: calling
/// NSApplication::sharedApplication first permanently installs a plain instance
/// without the `platform` ivar GPUI needs at launch.
#[cfg(target_os = "macos")]
fn apply_app_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(APP_ICON_PNG);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        eprintln!("warning: could not decode the embedded app icon");
        return;
    };
    // SAFETY: the icon setter requires the main thread, which this is.
    unsafe {
        NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image));
    }
}

/// Show why the library could not open in a window of its own; closing it
/// exits with a failure status.
fn show_startup_error(message: String) {
    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            apply_app_icon();
            cx.on_window_closed(|_, _| std::process::exit(1)).detach();
            let opened = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(560.), px(220.)),
                    })),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Ibex could not open the library".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                move |_, cx| cx.new(|_| StartupError { message }),
            );
            if let Err(err) = opened {
                eprintln!("error: failed to open window: {err}");
                std::process::exit(1);
            }
        });
}

struct StartupError {
    message: String,
}

impl Render for StartupError {
    fn render(&mut self, _: &mut Window, _: &mut gpui_kit::Context<Self>) -> impl IntoElement {
        div()
            .id("startup-error")
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_lg().child("Ibex could not open the library"))
            .child(div().child(self.message.clone()))
    }
}

fn setup_state(args: &Args, paths: &Paths) -> Result<AppState> {
    // The runtime is deliberately leaked: Surreal keeps engine tasks on it,
    // and it has to outlive the window. The DbWorker thread runs requests on
    // its own runtime, not this one.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(RUNTIME_STACK_BYTES)
        .build()
        .context("start runtime")?;
    let state = rt.block_on(async {
        if args.memory {
            let store = SurrealStore::open_memory()
                .await
                .context("open in-memory library")?;
            ibex_app::seed::seed_demo_library(&store, ibex_app::seed::DEFAULT_DEMO_LIBRARY_SIZE)
                .await
                .context("seed demo library")?;
            return Ok::<AppState, anyhow::Error>(
                AppState::builder(store).surface(Surface::Gui).build(),
            );
        }

        // The adapter's message names the path and, for a library another
        // process holds, what to do about it.
        let store = SurrealStore::open(&paths.database)
            .await
            .map_err(|err| anyhow!(err.message))?;
        Ok(AppState::builder(store).surface(Surface::Gui).build())
    })?;
    std::mem::forget(rt);
    Ok(state)
}

fn bind_library_keys(cx: &mut App) {
    // Shared with keyboard interaction tests so they exercise production bindings.
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
        KeyBinding::new(
            "f1",
            crate::documentation::OpenDocumentation,
            Some("LibraryApp"),
        ),
        KeyBinding::new("cmd-shift-i", ImportBiblatexFile, Some("LibraryApp")),
        KeyBinding::new("cmd-j", crate::library::ClassifyWithJev, Some("LibraryApp")),
        KeyBinding::new("]", ToggleDetailPanel, Some("LibraryApp && !Input")),
        KeyBinding::new("[", ToggleLeftPanel, Some("LibraryApp && !Input")),
        // GPUI dispatches actions before Link's keyboard click handling.
        // Keep activation keys local to browser links without disabling table actions.
        KeyBinding::new(
            "space",
            CycleColumnSort,
            Some("LibraryApp && !Input && !BrowserLink"),
        ),
        KeyBinding::new("e", BeginInlineEdit, Some("LibraryApp && !Input")),
        KeyBinding::new(
            "enter",
            ToggleEditMode,
            Some("LibraryApp && !Input && !BrowserLink"),
        ),
        KeyBinding::new("n", BeginNewRow, Some("LibraryApp && !Input")),
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
    bind_favorite_keys(cx);
}

fn bind_favorite_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            "f",
            crate::library::ToggleFavorite,
            Some("LibraryApp && !Input && !InlineEdit"),
        ),
        KeyBinding::new(
            "shift-f",
            FocusColumnFilter,
            Some("LibraryApp && !Input && !InlineEdit"),
        ),
    ]);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::Parser as _;
    use ibex_app::paths::AppPaths;

    use super::{Args, resolve_paths};

    #[test]
    fn gui_config_follows_the_shared_resolver_and_ignores_data() {
        let bases = AppPaths::new(
            Some(PathBuf::from("/home/ada")),
            Some(PathBuf::from("/home/ada/data")),
        );
        let with_data = Args::try_parse_from(["ibex-gui", "--data", "/srv/thesis"]).unwrap();
        let default = Args::try_parse_from(["ibex-gui"]).unwrap();
        let gui = resolve_paths(&with_data, &bases, None).unwrap();
        // The CLI resolves through the same `AppPaths::resolve`.
        let shared = bases.resolve(None, None).unwrap();
        assert_eq!(gui.config_file, shared.config_file);
        assert_eq!(
            gui.config_file,
            PathBuf::from("/home/ada/.ibex/config.toml")
        );
        assert_eq!(gui.database, PathBuf::from("/srv/thesis"));
        assert_eq!(resolve_paths(&default, &bases, None).unwrap(), shared);
    }
}
