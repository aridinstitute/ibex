use std::process::Command;

#[test]
fn add_and_list_tags_and_reject_duplicates() {
    let dir = tempfile::tempdir().expect("temporary library");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ibex"))
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .expect("run ibex")
    };

    let created = run(&["tag", "add", "review"]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let text = String::from_utf8(created.stdout).unwrap();
    assert!(text.contains("review"), "{text}");
    let listed = run(&["tag", "list"]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    assert!(String::from_utf8(listed.stdout).unwrap().contains("review"));

    let duplicate = run(&["tag", "add", "review"]);
    assert!(!duplicate.status.success(), "duplicate tag must fail");
    let blank = run(&["tag", "add", "  "]);
    assert!(!blank.status.success(), "blank tag must fail");
}
