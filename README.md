# Oeuvre

Oeuvre is a citation management program designed for speed. It is inspired by Zotero and tries to re-implement many of 
its features. It handles libraries with 100K+ items easily. It also is designed for agentic research, featuring both a
CLI and MCP for automation.

## Crates

| Crate                    | Path                     | Role                                         |
|--------------------------|--------------------------|----------------------------------------------|
| `oeuvre-core`            | `crates/core`            | Domain types and storage traits              |
| `oeuvre-app`             | `crates/app`             | AppState, commands, and wide-event logging   |
| `oeuvre-adapter-surreal` | `crates/adapter-surreal` | SurrealDB storage adapter                    |
| `oeuvre-cli`             | `crates/cli`             | `ovr` command-line interface                 |
| `oeuvre-gui`             | `crates/gui`             | `ovr-gui` desktop library (GPUI + gpui-kit)  |

Try either shell against a seeded in-memory library with `--memory`:

```sh
cargo run -p oeuvre-cli -- --memory resource list
cargo run -p oeuvre-gui -- --memory
```

## Importing bibliographies

`ovr import` reads BibLaTeX (`.bib`) or CSL-JSON (`.json`) files. The format is
inferred from the extension; pass `--format biblatex` or `--format csl-json` to
choose it explicitly.

```sh
ovr import references.bib --collection "Research" --tag "review" --tag "2026"
ovr import references.json --format csl-json --collection-id <id>
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

