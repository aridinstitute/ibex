# Ibex

Ibex is a citation management program designed for speed. It is inspired by Zotero and tries to re-implement many of
its features. It handles libraries with 100K+ items easily. It also is designed for agentic research, featuring both a
CLI and MCP for automation.

## Crates

| Crate                  | Path                     | Role                                        |
|------------------------|--------------------------|---------------------------------------------|
| `ibex-core`            | `crates/core`            | Domain types and storage traits             |
| `ibex-app`             | `crates/app`             | AppState, commands, and wide-event logging  |
| `ibex-adapter-surreal` | `crates/adapter-surreal` | SurrealDB storage adapter                   |
| `ibex-cli`             | `crates/cli`             | `ibex` command-line interface               |
| `ibex-gui`             | `crates/gui`             | `ibex-gui` desktop library (GPUI + gpui-kit) |

Try either shell against a throwaway, demo-seeded in-memory library with
`--memory` (nothing is written to disk):

```sh
cargo run -p ibex-cli -- --memory resource list
cargo run -p ibex-gui -- --memory
```

## Where Ibex keeps data

Without `--memory`, both clients open the same persistent library (SurrealKV
on SurrealDB 4.0) and read the same config file:

| What     | Default                                                                 | Override                         |
|----------|-------------------------------------------------------------------------|----------------------------------|
| Library  | macOS `~/Library/Application Support/Ibex/database`; Linux `$XDG_DATA_HOME/ibex/database` (usually `~/.local/share/ibex/database`); Windows `%APPDATA%\Ibex\database` | `--data <dir>` or `IBEX_DATA`    |
| Backups  | `backups` beside the default library (e.g. `.../Ibex/backups`)         | with `--data <dir>`: `<dir>-backups` |
| Config   | `~/.ibex/config.toml` on every platform                                 | `IBEX_CONFIG_DIR=<dir>`          |

- The config location never follows `--data`. The file is written atomically
  with private permissions (`0600`, directory `0700`), and each client keeps
  the other's keys when it saves.
- The library directory is created on first open. Only one Ibex process can
  hold a library at a time; a second GUI or CLI on the same library fails with
  "The Ibex library at <path> is in use by another Ibex process".
- Ibex never opens or migrates data from older installs (such as an Oeuvre
  directory) on its own.

> **Upgrading an existing 3.x database:** opening a database written by
> SurrealDB 3.x with Ibex (SurrealDB 4.0) converts it to the 4.0 on-disk format
> in place. The conversion is irreversible and 3.x can no longer open it. Only
> point `--data` at such a directory after taking a verified export with your
> 3.x tooling and an offline copy of the directory.

## Importing bibliographies

`ibex import` reads BibLaTeX (`.bib`) or CSL-JSON (`.json`) files. The format is
inferred from the extension; pass `--format biblatex` or `--format csl-json` to
choose it explicitly.

```sh
ibex import references.bib --collection "Research" --tag "review" --tag "2026"
ibex import references.json --format csl-json --collection-id <id>
```

Every imported or reused entry receives all of the given collections and tags.
Existing memberships are kept, and containers created automatically (such as a
journal for an article) are not tagged. `--collection` and `--tag` match an
existing collection or tag by exact name and create it when none exists (new
collections are top-level). When several collections share a name, the import
is rejected; select one with `--collection-id`. `--collection-id` and `--tag-id`
select existing objects only. All four flags are repeatable. Re-running the
same import reuses committed entries and never duplicates memberships, tags,
or collections.

The report lists imported, reused, skipped, conflicted, failed, and
not-attempted entries. A storage failure keeps entries committed before it and
exits with status 4.

