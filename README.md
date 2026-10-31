<a id="readme-top"></a>

# Ibex

Ibex is a citation manager built to handle massive libraries. It's inspired by Zotero and works as a CLI or desktop app.

## Overview
First, just to be clear: Ibex is currently (and somewhat apologetically) AI slop. I haven't looked at the code, though
I've done a fair amount of the architecture design (though I haven't been as involved as I wish I could be). I needed
something that worked and I needed it soon. I'd like to turn this into a quality product when I have time, but it bears
mentioning now that you'll likely encounter bad code, bugs, and quality issues.

### Built with

- Rust
- [SurrealDB](https://surrealdb.com/)
- [GPUI](https://www.gpui.rs/) and [gpui-kit](https://github.com/longbridge/gpui-kit)

### Comparison to Zotero
Ibex is inspired by Zotero, but it's not intended to be a clone. You'll notice some things that are similar (collections,
portions of the UI, etc.), but we diverge in a few ways:

- **Hierarchical Resources**: Resources like journal articles, podcast episodes, and book sections can have parent resources.
- **Built-in AI Features**: Ibex is designed for AI-assisted research and includes a CLI and (soon) an MCP.

## Getting started

You need a Rust toolchain with Cargo. Run either client against a demo-seeded in-memory library:

```sh
cargo run -p ibex-cli -- --memory resource list
cargo run -p ibex-gui -- --memory
```

## Usage

The CLI executable is `ibex`. The GUI includes an offline user guide: choose the **?** button in the toolbar or press **F1**. For now, its source is [`docs/user-guide.md`](docs/user-guide.md).

### Importing bibliographies

`ibex import` accepts BibLaTeX (`.bib`) and CSL-JSON (`.json`) files:

```sh
ibex import references.bib --collection "Research" --tag "review"
ibex import references.json --format csl-json --collection-id <id>
```

## Data and configuration

Without `--memory`, the CLI and GUI use the same persistent library and config:

| What    | Default                                                                                                                                                                 | Override                             |
|---------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------------------------------|
| Library | macOS `~/Library/Application Support/Ibex/database`; Linux `$XDG_DATA_HOME/ibex/database` (usually `~/.local/share/ibex/database`); Windows `%APPDATA%\\Ibex\\database` | `--data <dir>` or `IBEX_DATA`        |
| Backups | `backups` beside the default library                                                                                                                                    | With `--data <dir>`: `<dir>-backups` |
| Config  | `~/.ibex/config.toml` on every platform                                                                                                                                 | `IBEX_CONFIG_DIR=<dir>`              |

Only one Ibex process can use a library at a time.

## Crates

| Crate                  | Path                     | Role                                        |
|------------------------|--------------------------|---------------------------------------------|
| `ibex-core`            | `crates/core`            | Domain types and storage traits             |
| `ibex-app`             | `crates/app`             | App state, commands, and wide-event logging |
| `ibex-adapter-surreal` | `crates/adapter-surreal` | SurrealDB storage adapter                   |
| `ibex-cli`             | `crates/cli`             | `ibex` command-line interface               |
| `ibex-gui`             | `crates/gui`             | GPUI desktop application                    |

## Contributing

We’re not accepting pull requests right now, but you’re welcome to [open an issue](https://github.com/oeuvre/ibex/issues) with questions or suggestions.

## License

Ibex is licensed under the MIT License. See [`LICENSE`](LICENSE) for details.

<p align="right"><a href="#readme-top">Back to top</a></p>
