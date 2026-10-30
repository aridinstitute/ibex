use std::process::Command;

#[test]
fn update_favorite_without_changing_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let bib = dir.path().join("refs.bib");
    std::fs::write(
        &bib,
        "@book{a, title={Keep Title}, author={Lovelace, Ada}}\n",
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ibex"))
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&["import", bib.to_str().unwrap()]).status.success());
    let listed = run(&["resource", "list"]);
    let output = String::from_utf8(listed.stdout).unwrap();
    let id = output.split_whitespace().next().unwrap();

    let favorited = run(&["resource", "update", id, "--favorite"]);
    assert!(
        favorited.status.success(),
        "{}",
        String::from_utf8_lossy(&favorited.stderr)
    );
    let shown = run(&["resource", "show", id]);
    let text = String::from_utf8(shown.stdout).unwrap();
    assert!(
        text.contains("favorite: true") && text.contains("Keep Title"),
        "{text}"
    );

    let unfavorited = run(&["resource", "update", id, "--unfavorite"]);
    assert!(
        unfavorited.status.success(),
        "{}",
        String::from_utf8_lossy(&unfavorited.stderr)
    );
    let shown = run(&["resource", "show", id]);
    assert!(
        String::from_utf8(shown.stdout)
            .unwrap()
            .contains("favorite: false")
    );
    assert!(
        !run(&["resource", "update", id, "--favorite", "--unfavorite"])
            .status
            .success()
    );
}
