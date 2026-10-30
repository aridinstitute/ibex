use std::process::Command;

#[test]
fn dry_run_previews_biblatex_and_csl_json_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let bib = dir.path().join("refs.bib");
    std::fs::write(&bib, "@book{a, title={New Book}, author={Lovelace, Ada}}\n").unwrap();
    let csl = dir.path().join("refs.json");
    std::fs::write(&csl, r#"[{"id":"b","type":"book","title":"Second Book","author":[{"family":"Hopper","given":"Grace"}]}]"#).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ibex"))
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    for path in [&bib, &csl] {
        let preview = run(&["import", path.to_str().unwrap(), "--dry-run"]);
        assert!(
            preview.status.success(),
            "{}",
            String::from_utf8_lossy(&preview.stderr)
        );
        let text = String::from_utf8(preview.stdout).unwrap();
        assert!(text.contains("1") && text.contains("new"), "{text}");
        let rows = run(&["resource", "list"]);
        assert_eq!(String::from_utf8(rows.stdout).unwrap().trim(), "(none)");
    }
}
