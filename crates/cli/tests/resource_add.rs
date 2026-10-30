use std::process::Command;

#[test]
fn add_resource_with_creator_and_labels_then_show_it() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ovr"))
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let person = run(&["person", "add", "--first", "Ada", "--last", "Lovelace"]);
    assert!(person.status.success());
    let person_id = String::from_utf8(person.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let tag = run(&["tag", "add", "read"]);
    let tag_id = String::from_utf8(tag.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let collection = run(&["collection", "add", "Research"]);
    let collection_id = String::from_utf8(collection.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let created = run(&[
        "resource",
        "add",
        "--title",
        "New Book",
        "--kind",
        "book",
        "--creator",
        &format!("{person_id}:author"),
        "--tag",
        &tag_id,
        "--collection",
        &collection_id,
        "--publisher",
        "Press",
        "--published-year",
        "2022",
    ]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let id = String::from_utf8(created.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let shown = run(&["resource", "show", &id]);
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let detail = String::from_utf8(shown.stdout).unwrap();
    for expected in [
        "New Book",
        "Ada Lovelace",
        "Press",
        "2022",
        "Research",
        "read",
    ] {
        assert!(detail.contains(expected), "missing {expected}: {detail}");
    }
    assert!(!run(&["resource", "add", "--title", " "]).status.success());
}
