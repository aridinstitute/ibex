use std::process::Command;

#[test]
fn import_duplicate_policy_is_explicit_and_default_skips() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.bib");
    let second = dir.path().join("second.bib");
    std::fs::write(
        &first,
        "@book{one, title={Notes}, author={Lovelace, Ada}, doi={10.1000/notes}}\n",
    )
    .unwrap();
    std::fs::write(
        &second,
        "@book{other, title={Notes copy}, author={Lovelace, Ada}, doi={10.1000/notes}}\n",
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ovr"))
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&["import", first.to_str().unwrap()]).status.success());
    let skipped = run(&["import", second.to_str().unwrap()]);
    assert!(
        skipped.status.success(),
        "{}",
        String::from_utf8_lossy(&skipped.stderr)
    );
    let rows = run(&["resource", "list"]);
    assert_eq!(String::from_utf8(rows.stdout).unwrap().lines().count(), 1);
    let duplicate = run(&[
        "import",
        second.to_str().unwrap(),
        "--on-conflict",
        "duplicate",
    ]);
    assert!(
        duplicate.status.success(),
        "{}",
        String::from_utf8_lossy(&duplicate.stderr)
    );
    let rows = run(&["resource", "list"]);
    assert_eq!(String::from_utf8(rows.stdout).unwrap().lines().count(), 2);
}
