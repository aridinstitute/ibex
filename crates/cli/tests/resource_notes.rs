mod common;

#[test]
fn note_crud_on_a_resource() {
    let dir = tempfile::tempdir().expect("temporary library");
    let run = |args: &[&str]| {
        common::ibex()
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .expect("run ibex")
    };

    let created = run(&["resource", "add", "--title", "Source"]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let resource_id = String::from_utf8(created.stdout)
        .expect("UTF-8")
        .split_whitespace()
        .next()
        .expect("resource id")
        .to_owned();

    let empty = run(&["resource", "note", "list", &resource_id]);
    assert!(
        empty.status.success(),
        "{}",
        String::from_utf8_lossy(&empty.stderr)
    );
    assert_eq!(String::from_utf8(empty.stdout).unwrap().trim(), "(none)");

    let added = run(&[
        "resource",
        "note",
        "add",
        &resource_id,
        "--kind",
        "quote",
        "--body",
        "    indented\n\n**A thought**",
        "--location",
        "page=12–15",
        "--location",
        "section=2.1",
    ]);
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let added_out = String::from_utf8(added.stdout).unwrap();
    assert!(added_out.contains("quote"), "{added_out}");
    assert!(added_out.contains("page=12–15"), "{added_out}");
    assert!(added_out.contains("indented"), "{added_out}");
    let note_id = added_out
        .split_whitespace()
        .next()
        .expect("note id")
        .to_owned();

    let listed = run(&["resource", "note", "list", &resource_id]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let listed_out = String::from_utf8(listed.stdout).unwrap();
    assert!(listed_out.contains(&note_id), "{listed_out}");

    let updated = run(&[
        "resource",
        "note",
        "update",
        &note_id,
        "--kind",
        "summary",
        "--body",
        "Revised body",
        "--clear-locations",
    ]);
    assert!(
        updated.status.success(),
        "{}",
        String::from_utf8_lossy(&updated.stderr)
    );
    let updated_out = String::from_utf8(updated.stdout).unwrap();
    assert!(updated_out.contains("summary"), "{updated_out}");
    assert!(updated_out.contains("Revised body"), "{updated_out}");
    assert!(!updated_out.contains("page="), "{updated_out}");

    let blank = run(&["resource", "note", "add", &resource_id, "--body", "   "]);
    assert!(!blank.status.success(), "blank note body must fail");

    let shown = run(&["resource", "show", &resource_id]);
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let detail = String::from_utf8(shown.stdout).unwrap();
    assert!(detail.contains("notes:"), "{detail}");
    assert!(detail.contains(&note_id), "{detail}");
    assert!(detail.contains("Revised body"), "{detail}");

    let deleted = run(&["resource", "note", "delete", &note_id]);
    assert!(
        deleted.status.success(),
        "{}",
        String::from_utf8_lossy(&deleted.stderr)
    );
    let after = run(&["resource", "note", "list", &resource_id]);
    assert!(
        after.status.success(),
        "{}",
        String::from_utf8_lossy(&after.stderr)
    );
    assert_eq!(String::from_utf8(after.stdout).unwrap().trim(), "(none)");
}

#[test]
fn note_update_requires_a_change_flag() {
    let dir = tempfile::tempdir().expect("temporary library");
    let run = |args: &[&str]| {
        common::ibex()
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .expect("run ibex")
    };
    let created = run(&["resource", "add", "--title", "Source"]);
    let resource_id = String::from_utf8(created.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let added = run(&["resource", "note", "add", &resource_id, "--body", "Keep me"]);
    let note_id = String::from_utf8(added.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let no_flags = run(&["resource", "note", "update", &note_id]);
    assert!(!no_flags.status.success(), "update with no flags must fail");
}
