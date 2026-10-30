use std::process::Command;

#[test]
fn fetch_validates_identifier_and_update_target_before_network() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ovr"))
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let invalid = run(&["resource", "fetch", "not-an-isbn-or-doi"]);
    assert!(!invalid.status.success());
    let message = String::from_utf8(invalid.stderr).unwrap();
    assert!(
        message.contains("identifier") || message.contains("ISBN"),
        "{message}"
    );
    let missing = run(&[
        "resource",
        "fetch",
        "9780306406157",
        "--update",
        "01ARZ3NDEKTSV4RRFFQ69G5FAV",
    ]);
    assert!(!missing.status.success());
    let message = String::from_utf8(missing.stderr).unwrap();
    assert!(message.contains("not found"), "{message}");
}
