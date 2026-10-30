mod common;

#[test]
fn show_prints_full_resource_detail_and_rejects_missing_id() {
    let dir = tempfile::tempdir().expect("temporary library");
    let bib = dir.path().join("refs.bib");
    std::fs::write(
        &bib,
        "@book{example, title={Complete Work}, author={Lovelace, Ada}, doi={10.1234/example}, abstract={A detailed summary}}\n",
    )
    .expect("bibliography");
    let run = |args: &[&str]| {
        common::ibex()
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .expect("run ibex")
    };

    let imported = run(&["import", bib.to_str().unwrap(), "--tag", "review"]);
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    let listed = run(&["resource", "list"]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let rows = String::from_utf8(listed.stdout).expect("UTF-8 rows");
    let id = rows.split_whitespace().next().expect("resource id");

    let shown = run(&["resource", "show", id]);
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let detail = String::from_utf8(shown.stdout).expect("UTF-8 detail");
    for expected in [
        "Complete Work",
        "10.1234/example",
        "A detailed summary",
        "review",
    ] {
        assert!(detail.contains(expected), "missing {expected}: {detail}");
    }

    let missing = run(&["resource", "show", "01ARZ3NDEKTSV4RRFFQ69G5FAV"]);
    assert!(!missing.status.success(), "missing resource should fail");
}
