use std::process::Command;

#[test]
fn show_prints_the_person_and_get_is_not_a_command() {
    let dir = tempfile::tempdir().expect("temporary library");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ibex"))
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .expect("run ibex")
    };

    let added = run(&["person", "add", "--first", "Ada", "--last", "Lovelace"]);
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let output = String::from_utf8(added.stdout).expect("UTF-8 output");
    let id = output.split_whitespace().next().expect("created person ID");

    let shown = run(&["person", "show", id]);
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let output = String::from_utf8(shown.stdout).expect("UTF-8 output");
    assert!(output.contains("Ada Lovelace"), "{output}");

    let old = run(&["person", "get", id]);
    assert!(!old.status.success(), "old verb should be removed");
}
