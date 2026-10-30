# AGENTS.md

Guidance for AI agents and contributors working in this repository.

## Project

Ibex is a citation manager built for speed and agentic research (CLI + MCP). It is a Rust workspace:

| Crate | Path | Role |
|-------|------|------|
| `ibex-core` | `crates/core` | Domain types and storage traits |
| `ibex-app` | `crates/app` | AppState, commands, and wide-event logging |
| `ibex-adapter-surreal` | `crates/adapter-surreal` | SurrealDB storage adapter |
| `ibex-cli` | `crates/cli` | `ibex` CLI (clap) |

See `README.md` and `docs/` for product context.

## Data, config, and tests

- Paths come from `ibex_app::paths` (library, backups, `~/.ibex/config.toml`);
  the shared config is read and written through `ibex_app::config`. Do not add
  other path or config logic to the clients.
- Tests must never touch the real home: pass `--data` and `IBEX_CONFIG_DIR`
  temp dirs (CLI tests use `tests/common::ibex()`), or `--memory`.
- Never point Ibex at an existing 3.x database: SurrealDB 4.0 upgrades it in
  place, irreversibly.

## Git: branches

Use a type prefix, then a short kebab-case slug:

- `feat/` — new capability
- `fix/` — bug fix
- `docs/` — documentation only
- `refactor/` — internal restructuring
- `chore/` — tooling, deps, maintenance
- `release/` — release preparation

Examples: `feat/library-search`, `fix/duplicate-detection`, `docs/agents-md`.

Do not commit directly to `main` unless explicitly asked.

## Git: commits

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(optional-scope): <summary>
```

- **Types:** `feat`, `fix`, `docs`, `refactor`, `chore`, `test`, `perf`, `ci`, `build`, `revert`
- **Summary:** imperative, lowercase, no trailing period; focus on *why* when it is not obvious
- **Body:** optional; use when the change needs rationale or migration notes

Examples:

```
feat(core): add item identity traits
fix(adapter-surreal): handle empty library queries
docs: add AGENTS.md with git conventions
```

Only create commits when the user asks. Never amend pushed history or skip hooks unless requested.

## Working style

- Prefer small, reviewable changes scoped to the request.
- Match existing Rust workspace layout and naming (`ibex-*` crates under `crates/`).
- Do not expand scope into unrelated refactors or drive-by docs.
- Scope formatting to touched files; check the diff afterward and remove unrelated formatting changes (avoid `cargo fmt --all` for local UI edits).
- Ask before destructive git operations (force push, hard reset).
