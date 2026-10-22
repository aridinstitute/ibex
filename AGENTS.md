# AGENTS.md

Guidance for AI agents and contributors working in this repository.

## Project

Oeuvre is a citation manager built for speed and agentic research (CLI + MCP). It is a Rust workspace:

| Crate | Path | Role |
|-------|------|------|
| `oeuvre-core` | `crates/core` | Domain types and storage traits |
| `oeuvre-app` | `crates/app` | AppState, commands, and wide-event logging |
| `oeuvre-adapter-surreal` | `crates/adapter-surreal` | SurrealDB storage adapter |
| `oeuvre-cli` | `crates/cli` | `ovr` CLI (clap) |

See `README.md` and `docs/` for product context.

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
- Match existing Rust workspace layout and naming (`oeuvre-*` crates under `crates/`).
- Do not expand scope into unrelated refactors or drive-by docs.
- Scope formatting to touched files; check the diff afterward and remove unrelated formatting changes (avoid `cargo fmt --all` for local UI edits).
- Ask before destructive git operations (force push, hard reset).
