# Oeuvre

Citation management, built as a Rust monorepo.

## Stack

- **Core** — shared domain logic (`oeuvre-core`)
- **SurrealDB** — persistence via `oeuvre-adapter-surreal`
- **CLI** — Clap (derive) *(planned)*
- **MCP** — Model Context Protocol server *(planned)*
- **Desktop** — GPUI + GPUI Kit *(planned)*

## Crates

| Crate | Path | Role |
| --- | --- | --- |
| `oeuvre-core` | `crates/core` | Domain types and traits |
| `oeuvre-adapter-surreal` | `crates/adapter-surreal` | SurrealDB storage adapter |

## Development

```bash
cargo check
cargo test
```

## Docs

- [Zotero feature parity](./docs/zotero-feature-parity.md) — target feature checklist against Zotero
