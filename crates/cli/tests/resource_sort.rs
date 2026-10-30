mod common;

#[test]
fn sorts_title_ascending_and_descending() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        common::ibex()
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let person = run(&["person", "add", "--last", "Lovelace"]);
    let id = String::from_utf8(person.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    for title in ["Zebra", "Alpha"] {
        let created = run(&[
            "resource",
            "add",
            "--title",
            title,
            "--creator",
            &format!("{id}:author"),
        ]);
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
    }
    let sorted = run(&["resource", "list", "--sort", "title", "--asc"]);
    assert!(
        sorted.status.success(),
        "{}",
        String::from_utf8_lossy(&sorted.stderr)
    );
    let text = String::from_utf8(sorted.stdout).unwrap();
    assert!(
        text.find("Alpha").unwrap() < text.find("Zebra").unwrap(),
        "{text}"
    );
    let sorted = run(&["resource", "list", "--sort", "title", "--desc"]);
    assert!(
        sorted.status.success(),
        "{}",
        String::from_utf8_lossy(&sorted.stderr)
    );
    let text = String::from_utf8(sorted.stdout).unwrap();
    assert!(
        text.find("Zebra").unwrap() < text.find("Alpha").unwrap(),
        "{text}"
    );
    assert!(!run(&["resource", "list", "--asc"]).status.success());
}
