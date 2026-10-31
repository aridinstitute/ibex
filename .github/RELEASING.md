# Releasing Ibex

## Branch flow

1. Branch features from `release/<VERSION>` (`feat/*`, `fix/*`) and open PRs into it.
2. When the release is ready, set `[workspace.package].version` in `Cargo.toml`
   and open a squash PR from `release/<VERSION>` into `main`.
3. After it merges, tag the squash commit on `main` and push the tag:

   ```sh
   git switch main && git pull
   git tag -a v0.1.0 -m "Ibex v0.1.0"
   git push origin v0.1.0
   ```

4. The Release workflow confirms CI already passed for the tagged source tree
   (it does not re-run CI), packages the Apple Silicon macOS app + CLI
   and the Linux x86_64 CLI, smoke-tests the extracted archives, and creates a
   **draft** GitHub Release. Review it, then publish it manually.

Release candidates are tagged on the staging branch instead: set the version to
`0.1.0-rc.1`, merge that into `release/0.1`, then tag and push `v0.1.0-rc.1`
from `release/0.1`. Prerelease tags must point at a commit on a `release/*`
branch; final tags must point at a commit on `main`.

## Workflows

| Workflow | Runs on | Blocking |
|---|---|---|
| `ci.yml` | PRs into `main` / `release/**`, pushes to `main`, manual; weekly dependency audit only | Yes, via the **CI gate** check |
| `performance.yml` | Manual only (pick a baseline and `small`/`large`) | No; compares timings on one runner and uploads reports |
| `release.yml` | `v*` tags | Creates a draft only |

Clippy and rustdoc warnings fail CI. To save Actions minutes:

- Shared crates are tested on Linux; macOS runs only the GUI crate.
- Benchmark compilation and optimized builds run only on pushes to `main`;
  release packaging builds and smoke-tests optimized binaries for every tag.
- Changes that touch only Markdown or `docs/` skip CI, except
  `docs/user-guide.md`, which is compiled into the GUI.
- Coverage runs only on manual dispatch and is informational.

If a release tag points at a commit whose source tree no PR run tested (for
example, a docs-only merge or a PR that was behind its base), the Release
workflow stops. Run `gh workflow run ci.yml --ref <branch>`, then re-run the
release once CI passes.

## GitHub settings

Branch protection and rulesets need GitHub Pro (or a public repository) for
private repositories. Until then, enforce these by convention:

- Protect `main` and `release/*`: require a PR and the **CI gate** status
  check, and block force pushes.
- Allow only squash merges into `main`.
- Optionally protect `v*` tags so only maintainers can create them.
- macOS jobs on private repositories consume 10x the Actions minutes of
  Linux jobs. A public repository gets standard runners for free.

## Unsigned builds

The Mac app is not Developer ID signed or notarized. After downloading, users
must clear the quarantine flag (`xattr -dr com.apple.quarantine Ibex.app`) or
use **Open** from the Finder context menu. Add signing and notarization to
`release.yml` once an Apple Developer certificate is available.

## Dependency policy

`deny.toml` blocks RustSec advisories, unapproved licenses, and unknown
sources. Its advisory ignores cover SurrealDB 3.3.5 internals with no
compatible fix; remove them when upgrading SurrealDB.
