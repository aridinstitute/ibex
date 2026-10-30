use std::process::Command;

#[test]
fn tree_lists_container_and_child_with_indentation() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ibex"))
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let person = run(&["person", "add", "--last", "Lovelace"]);
    let person_id = String::from_utf8(person.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let parent = run(&[
        "resource",
        "add",
        "--title",
        "Container",
        "--kind",
        "book",
        "--creator",
        &format!("{person_id}:author"),
    ]);
    assert!(
        parent.status.success(),
        "{}",
        String::from_utf8_lossy(&parent.stderr)
    );
    let parent_id = String::from_utf8(parent.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let child = run(&[
        "resource",
        "add",
        "--title",
        "Chapter",
        "--kind",
        "bookSection",
        "--parent",
        &parent_id,
        "--creator",
        &format!("{person_id}:author"),
    ]);
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    let child_id = String::from_utf8(child.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let tree = run(&["resource", "list", "--tree"]);
    assert!(
        tree.status.success(),
        "{}",
        String::from_utf8_lossy(&tree.stderr)
    );
    let text = String::from_utf8(tree.stdout).unwrap();
    assert!(
        text.contains(&format!("{parent_id}  book  Container")),
        "{text}"
    );
    assert!(
        text.contains(&format!("  {child_id}  bookSection  Chapter")),
        "{text}"
    );
    assert_eq!(text.lines().count(), 2, "{text}");
}
