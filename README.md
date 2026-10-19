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

