use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn config_updates_model_and_secret_without_leaking_or_dropping_theme() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    std::fs::write(&file, "[theme.light]\naccent = \"#0064FF\"\n").unwrap();
    let model = Command::new(env!("CARGO_BIN_EXE_ovr"))
        .env("OEUVRE_CONFIG_DIR", dir.path())
        .args(["config", "set", "classification-model", "typesafe/jev-1.14"])
        .output()
        .unwrap();
    assert!(
        model.status.success(),
        "{}",
        String::from_utf8_lossy(&model.stderr)
    );
    let mut secret = Command::new(env!("CARGO_BIN_EXE_ovr"))
        .env("OEUVRE_CONFIG_DIR", dir.path())
        .args(["config", "set", "openrouter-api-key", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    secret
        .stdin
        .take()
        .unwrap()
        .write_all(b"sk-test-secret\n")
        .unwrap();
    let result = secret.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains("sk-test-secret"));
    assert!(!String::from_utf8_lossy(&result.stderr).contains("sk-test-secret"));
    let raw = std::fs::read_to_string(&file).unwrap();
    assert!(
        raw.contains("#0064FF")
            && raw.contains("typesafe/jev-1.14")
            && raw.contains("sk-test-secret"),
        "{raw}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
