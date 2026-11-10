mod common;

#[test]
fn update_attach_file_shows_on_resource_detail() {
    let dir = tempfile::tempdir().expect("temporary library");
    let pdf = dir.path().join("paper.pdf");
    std::fs::write(&pdf, b"%PDF-1.4 demo").expect("attachment file");
    let run = |args: &[&str]| {
        common::ibex()
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .expect("run ibex")
    };

    let created = run(&["resource", "add", "--title", "With PDF"]);
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

    let attached = run(&[
        "resource",
        "update",
        &resource_id,
        "--attach",
        pdf.to_str().expect("utf-8 path"),
    ]);
    assert!(
        attached.status.success(),
        "{}",
        String::from_utf8_lossy(&attached.stderr)
    );
    let attached_out = String::from_utf8(attached.stdout).unwrap();
    assert!(attached_out.contains("attachment"), "{attached_out}");
    assert!(attached_out.contains("pdf"), "{attached_out}");
    assert!(
        attached_out.contains("paper.pdf") || attached_out.contains(&*pdf.to_string_lossy()),
        "{attached_out}"
    );
    assert!(attached_out.contains(&resource_id), "{attached_out}");

    let shown = run(&["resource", "show", &resource_id]);
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let detail = String::from_utf8(shown.stdout).unwrap();
    assert!(detail.contains("attachments"), "{detail}");
    assert!(detail.contains("paper"), "{detail}");

    let missing = run(&[
        "resource",
        "update",
        &resource_id,
        "--attach",
        dir.path().join("missing.pdf").to_str().unwrap(),
    ]);
    assert!(!missing.status.success(), "missing file must fail");
}
