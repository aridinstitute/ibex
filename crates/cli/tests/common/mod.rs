//! Shared setup for CLI integration tests.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A config directory under the target dir that the library tests never
/// write, so a run never reads or writes the real `~/.ibex`.
pub fn isolated_config_dir() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("ibex-cli-tests-config")
}

/// The `ibex` binary with its config isolated from the real home. Tests
/// still pass `--data` (or `--memory`) themselves; a later
/// `.env("IBEX_CONFIG_DIR", ..)` overrides this one.
#[allow(dead_code)]
pub fn ibex() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ibex"));
    cmd.env("IBEX_CONFIG_DIR", isolated_config_dir())
        .env_remove("IBEX_DATA");
    cmd
}
