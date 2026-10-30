use std::process::Command;

#[test]
fn list_and_show_imported_collection() {
    let dir = tempfile::tempdir().expect("temporary library");
    let bib = dir.path().join("refs.bib");
    std::fs::write(&bib, "@book{a, title={A Book}, author={Lovelace, Ada}}\n").unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ovr"))
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .expect("run ovr")
    };
    let imported = run(&["import", bib.to_str().unwrap(), "--collection", "Research"]);
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    let listed = run(&["collection", "list"]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let text = String::from_utf8(listed.stdout).unwrap();
    assert!(text.contains("Research"), "{text}");
    let id = text.split_whitespace().next().expect("collection ID");
    let shown = run(&["collection", "show", id]);
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let detail = String::from_utf8(shown.stdout).unwrap();
    assert!(
        detail.contains("Research") && detail.contains("jev_enabled: false"),
        "{detail}"
    );
    let flat = run(&["collection", "list", "--flat"]);
    assert!(
        flat.status.success(),
        "{}",
        String::from_utf8_lossy(&flat.stderr)
    );
    assert!(String::from_utf8(flat.stdout).unwrap().contains("Research"));
    let missing = run(&["collection", "show", "01ARZ3NDEKTSV4RRFFQ69G5FAV"]);
    assert!(!missing.status.success());
}

#[test]
fn add_root_and_child_and_reject_unknown_parent() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ovr"))
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let root = run(&["collection", "add", "Research"]);
    assert!(
        root.status.success(),
        "{}",
        String::from_utf8_lossy(&root.stderr)
    );
    let root_text = String::from_utf8(root.stdout).unwrap();
    let root_id = root_text.split_whitespace().next().unwrap();
    let child = run(&["collection", "add", "Methods", "--parent", root_id]);
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    let child_text = String::from_utf8(child.stdout).unwrap();
    let child_id = child_text.split_whitespace().next().unwrap();
    let shown = run(&["collection", "show", child_id]);
    let detail = String::from_utf8(shown.stdout).unwrap();
    assert!(detail.contains(&format!("parents: {root_id}")), "{detail}");
    let tree = run(&["collection", "list"]);
    let text = String::from_utf8(tree.stdout).unwrap();
    assert!(text.contains(&format!("  {child_id}  Methods")), "{text}");
    let missing = run(&[
        "collection",
        "add",
        "Orphan",
        "--parent",
        "01ARZ3NDEKTSV4RRFFQ69G5FAV",
    ]);
    assert!(!missing.status.success());
    let tree = run(&["collection", "list"]);
    assert!(!String::from_utf8(tree.stdout).unwrap().contains("Orphan"));
}

#[test]
fn delete_collection_cascades_exclusive_children() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ovr"))
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let root = run(&["collection", "add", "Root"]);
    let id = String::from_utf8(root.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    assert!(
        run(&["collection", "add", "Child", "--parent", &id])
            .status
            .success()
    );
    let deleted = run(&["collection", "delete", &id]);
    assert!(
        deleted.status.success(),
        "{}",
        String::from_utf8_lossy(&deleted.stderr)
    );
    let list = run(&["collection", "list"]);
    let text = String::from_utf8(list.stdout).unwrap();
    assert_eq!(text.trim(), "(none)", "{text}");
}

#[test]
fn update_replaces_classification_and_edits_all_parent_links_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ovr"))
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let add = |name: &str| {
        let out = run(&["collection", "add", name]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_owned()
    };
    let a = add("A");
    let b = add("B");
    let c = add("C");
    let child = add("Child");
    let updated = run(&[
        "collection",
        "update",
        &child,
        "--name",
        "New",
        "--add-parent",
        &a,
        "--add-parent",
        &b,
        "--keyword",
        "  History ",
        "--keyword",
        "history",
        "--jev",
    ]);
    assert!(
        updated.status.success(),
        "{}",
        String::from_utf8_lossy(&updated.stderr)
    );
    let show = |id: &str| String::from_utf8(run(&["collection", "show", id]).stdout).unwrap();
    let detail = show(&child);
    assert!(
        detail.contains("name: New")
            && detail.contains("keywords: History")
            && detail.contains("jev_enabled: true"),
        "{detail}"
    );
    assert!(detail.contains(&a) && detail.contains(&b), "{detail}");

    let failed = run(&[
        "collection",
        "update",
        &child,
        "--name",
        "Bad",
        "--remove-parent",
        &a,
        "--add-parent",
        &c,
        "--add-parent",
        "01ARZ3NDEKTSV4RRFFQ69G5FAV",
    ]);
    assert!(!failed.status.success());
    let detail = show(&child);
    assert!(
        detail.contains("name: New")
            && detail.contains(&a)
            && detail.contains(&b)
            && !detail.contains(&c),
        "{detail}"
    );

    let changed = run(&[
        "collection",
        "update",
        &child,
        "--remove-parent",
        &a,
        "--add-parent",
        &c,
        "--clear-keywords",
        "--no-jev",
    ]);
    assert!(
        changed.status.success(),
        "{}",
        String::from_utf8_lossy(&changed.stderr)
    );
    let detail = show(&child);
    assert!(
        detail.contains(&b) && detail.contains(&c) && !detail.contains(&a),
        "{detail}"
    );
    assert!(
        detail.contains("keywords: \n") && detail.contains("jev_enabled: false"),
        "{detail}"
    );
    let cycle = run(&[
        "collection",
        "update",
        &b,
        "--name",
        "ShouldNotPersist",
        "--add-parent",
        &child,
    ]);
    assert!(!cycle.status.success());
    assert!(show(&b).contains("name: B"));
    assert!(!show(&b).contains(&child));
}
