mod common;

#[test]
fn keyword_classification_previews_then_applies_without_duplicates() {
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
    let person_id = String::from_utf8(person.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let collection = run(&["collection", "add", "Mathematics"]);
    let collection_id = String::from_utf8(collection.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    assert!(
        run(&[
            "collection",
            "update",
            &collection_id,
            "--keyword",
            "analytical engine"
        ])
        .status
        .success()
    );
    let created = run(&[
        "resource",
        "add",
        "--title",
        "Analytical Engine",
        "--creator",
        &format!("{person_id}:author"),
    ]);
    let id = String::from_utf8(created.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let preview = run(&["resource", "classify", &id]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(
        String::from_utf8(preview.stdout)
            .unwrap()
            .contains("Mathematics")
    );
    assert!(
        !String::from_utf8(run(&["resource", "show", &id]).stdout)
            .unwrap()
            .contains("Mathematics")
    );
    let apply = run(&["resource", "classify", &id, "--apply"]);
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    assert!(
        String::from_utf8(run(&["resource", "show", &id]).stdout)
            .unwrap()
            .contains("Mathematics")
    );
    assert_eq!(
        String::from_utf8(run(&["resource", "classify", &id]).stdout)
            .unwrap()
            .trim(),
        "(none)"
    );
}
