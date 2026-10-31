// Requires a logged-in macOS desktop session; run with
// `cargo test -p ibex-gui --test macos_startup -- --ignored`.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires a graphical macOS session"]
fn opens_window_without_creating_plain_nsapplication_first() {
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    let temp = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ibex-gui"))
        .arg("--data")
        .arg(temp.path().join("library"))
        .env("IBEX_CONFIG_DIR", temp.path().join("config"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            panic!(
                "GUI exited before opening a window ({status}): {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
}
