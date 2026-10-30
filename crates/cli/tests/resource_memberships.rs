use std::process::Command;

#[test]
fn update_adds_and_removes_memberships_without_replacing_other_fields() {
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
    let created = run(&[
        "resource",
        "add",
        "--title",
        "Keep Book",
        "--creator",
        &format!("{person_id}:author"),
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
    let updated = run(&[
        "resource",
        "update",
        &id,
        "--tag",
        &tag_id,
        "--collection",
        &collection_id,
    ]);
    assert!(
        updated.status.success(),
        "{}",
        String::from_utf8_lossy(&updated.stderr)
    );
    let detail = String::from_utf8(run(&["resource", "show", &id]).stdout).unwrap();
    assert!(
        detail.contains("read") && detail.contains("Research") && detail.contains("Keep Book"),
        "{detail}"
    );
    assert!(
        run(&["resource", "update", &id, "--remove-tag", &tag_id])
            .status
            .success()
    );
    let detail = String::from_utf8(run(&["resource", "show", &id]).stdout).unwrap();
    assert!(
        !detail.contains("read") && detail.contains("Research") && detail.contains("Keep Book"),
        "{detail}"
    );
    assert!(
        run(&[
            "resource",
            "update",
            &id,
            "--remove-collection",
            &collection_id
        ])
        .status
        .success()
    );
    let detail = String::from_utf8(run(&["resource", "show", &id]).stdout).unwrap();
    assert!(!detail.contains("Research"), "{detail}");
}
