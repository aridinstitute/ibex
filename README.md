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

Try either shell against a seeded in-memory library with `--memory`:

```sh
cargo run -p ibex-cli -- --memory resource list
cargo run -p ibex-gui -- --memory
```

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

