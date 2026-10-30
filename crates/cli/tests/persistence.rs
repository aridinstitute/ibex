//! The library and config persist where the shared path resolver says,
//! across separate CLI processes, and `--memory` writes nothing.

mod common;

use std::path::Path;
use std::process::{Command, Output};

fn ok(output: &Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// `ibex` with a fake home and no Ibex overrides, so defaults resolve
/// inside `home` instead of the real one.
fn ibex_at_home(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ibex"));
    cmd.env("HOME", home)
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env_remove("IBEX_CONFIG_DIR")
        .env_remove("IBEX_DATA");
    cmd
}

fn entries(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            found.push(path.display().to_string());
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    found
}

#[test]
fn a_resource_added_in_one_process_is_listed_by_the_next() {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    let config = dir.path().join("config");
    let run = |args: &[&str]| {
        common::ibex()
            .env("IBEX_CONFIG_DIR", &config)
            .arg("--data")
            .arg(&library)
            .args(args)
            .output()
            .unwrap()
    };
    ok(&run(&["resource", "add", "--title", "Persisted Book"]));
    let listed = ok(&run(&["resource", "list"]));
    assert!(listed.contains("Persisted Book"), "{listed}");
    assert!(library.is_dir());
}

#[test]
fn default_library_lives_in_the_platform_data_dir() {
    let home = tempfile::tempdir().unwrap();
    ok(&ibex_at_home(home.path())
        .args(["resource", "add", "--title", "Home Book"])
        .output()
        .unwrap());
    #[cfg(target_os = "macos")]
    let database = home
        .path()
        .join("Library/Application Support/Ibex/database");
    #[cfg(all(unix, not(target_os = "macos")))]
    let database = home.path().join(".local/share/ibex/database");
    #[cfg(unix)]
    assert!(database.is_dir(), "{:?}", entries(home.path()));
    let listed = ok(&ibex_at_home(home.path())
        .args(["resource", "list"])
        .output()
        .unwrap());
    assert!(listed.contains("Home Book"), "{listed}");
}

#[cfg(unix)]
#[test]
fn config_lives_in_dot_ibex_whatever_data_says() {
    use std::os::unix::fs::PermissionsExt as _;

    let home = tempfile::tempdir().unwrap();
    for (data, model) in [("first", "typesafe/jev-a"), ("second", "typesafe/jev-b")] {
        ok(&ibex_at_home(home.path())
            .arg("--data")
            .arg(home.path().join(data))
            .args(["config", "set", "classification-model", model])
            .output()
            .unwrap());
        // Setting config never opens or creates a library.
        assert!(!home.path().join(data).exists());
    }
    let file = home.path().join(".ibex/config.toml");
    let raw = std::fs::read_to_string(&file).unwrap();
    assert!(raw.contains("typesafe/jev-b"), "{raw}");
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&file), 0o600);
    assert_eq!(mode(&home.path().join(".ibex")), 0o700);
}

#[test]
fn memory_mode_writes_nothing_to_disk() {
    let home = tempfile::tempdir().unwrap();
    let listed = ok(&ibex_at_home(home.path())
        .args(["--memory", "resource", "list"])
        .output()
        .unwrap());
    assert!(!listed.trim().is_empty(), "the demo library is seeded");
    assert_eq!(entries(home.path()), Vec::<String>::new());
}

#[test]
fn a_library_held_by_another_process_fails_with_a_clear_message() {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(10 * 1024 * 1024)
        .build()
        .unwrap();
    let held = runtime
        .block_on(ibex_adapter_surreal::SurrealStore::open(&library))
        .unwrap();
    let output = common::ibex()
        .arg("--data")
        .arg(&library)
        .args(["resource", "list"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!(
            "The Ibex library at {} is in use by another Ibex process. \
             Close the other Ibex app or CLI and try again.",
            library.display()
        )),
        "{stderr}"
    );
    runtime.block_on(held.close()).unwrap();
}
