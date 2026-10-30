mod common;

#[test]
fn list_filters_by_kind_favorite_tag_collection_and_search() {
    let dir = tempfile::tempdir().unwrap();
    let bib = dir.path().join("refs.bib");
    std::fs::write(
        &bib,
        "@book{a, title={The Atlas}, author={Lovelace, Ada}}\n",
    )
    .unwrap();
    let run = |args: &[&str]| {
        common::ibex()
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let imported = run(&[
        "import",
        bib.to_str().unwrap(),
        "--tag",
        "review",
        "--collection",
        "Research",
    ]);
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    let resource = run(&["resource", "list"]);
    let id = String::from_utf8(resource.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    assert!(
        run(&["resource", "update", &id, "--favorite"])
            .status
            .success()
    );
    let tag = run(&["tag", "list"]);
    let tag_id = String::from_utf8(tag.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let collection = run(&["collection", "list"]);
    let collection_id = String::from_utf8(collection.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let matched = run(&[
        "resource",
        "list",
        "--kind",
        "book",
        "--favorite",
        "--tag",
        &tag_id,
        "--collection",
        &collection_id,
        "--search",
        "Atlas",
    ]);
    assert!(
        matched.status.success(),
        "{}",
        String::from_utf8_lossy(&matched.stderr)
    );
    assert!(
        String::from_utf8(matched.stdout)
            .unwrap()
            .contains("The Atlas")
    );
    for flags in [vec!["--kind", "webpage"], vec!["--search", "unknown"]] {
        let missing = run(&["resource", "list", flags[0], flags[1]]);
        assert!(missing.status.success());
        assert_eq!(String::from_utf8(missing.stdout).unwrap().trim(), "(none)");
    }
}
