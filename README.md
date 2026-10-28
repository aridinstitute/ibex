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

## GUI memory benchmark

Build the real desktop app in release mode, then run the process-level benchmark on a desktop session:

```sh
cargo build -p oeuvre-gui --release --locked
python3 scripts/benchmark_gui_memory.py --sizes 0,1000,10000,100000
```

Results go to `target/gui-memory.csv` (override with `--output`). Each size launches a fresh GUI with a deterministic in-memory SurrealDB library, waits until its window has opened and for a 3-second settling period, then terminates it. Sizes specify demo items; the seed also creates a few supporting resources even at size zero. The CSV records the sampled **resident set size (RSS)** in KiB at the end of that period, the highest RSS observed while starting and settling, and seconds to window-open. RSS includes the whole GUI process (GPUI, database, and runtime); the sampled peak can miss spikes between samples. The marker indicates window creation, not a confirmed rendered frame. Run on the same machine with similar system load for comparisons. `--interval`, `--settle`, and `--timeout` tune sampling; a failure or timeout aborts without writing a partial CSV. The benchmark needs macOS or Linux `ps` and an active graphical session. For a smaller smoke test, use `--sizes 0,1000`.
