mod common;

use std::fs;

#[test]
fn cluster_preview_and_apply_groups_by_content() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        common::ibex()
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };

    let papers = [
        (
            "Neural nets",
            "deep learning neural networks representation learning backpropagation layers",
        ),
        (
            "CNN survey",
            "convolutional neural networks deep learning image recognition layers",
        ),
        (
            "Transformers",
            "transformer attention neural networks deep learning language models",
        ),
        (
            "Carbon sinks",
            "climate change carbon sequestration forests greenhouse emissions",
        ),
        (
            "Warming trends",
            "climate change global warming temperature greenhouse emissions",
        ),
        (
            "Ocean acid",
            "climate change ocean acidification carbon dioxide greenhouse emissions",
        ),
    ];

    for (i, (title, body)) in papers.iter().enumerate() {
        let path = dir.path().join(format!("paper-{i}.txt"));
        fs::write(&path, body).unwrap();
        let created = run(&["resource", "add", "--title", title]);
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
        let attached = run(&[
            "resource",
            "update",
            &id,
            "--attach",
            path.to_str().unwrap(),
        ]);
        assert!(
            attached.status.success(),
            "{}",
            String::from_utf8_lossy(&attached.stderr)
        );
    }

    let preview = run(&["collection", "cluster", "--min-size", "3"]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let preview_out = String::from_utf8(preview.stdout).unwrap();
    assert!(
        preview_out.contains("topic 1"),
        "preview missing topics: {preview_out}"
    );
    assert!(
        preview_out.contains("[attachment]"),
        "expected attachment provenance: {preview_out}"
    );
    assert!(
        String::from_utf8_lossy(&preview.stderr).contains("preview only"),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );

    // Preview must not create collections.
    let listed = run(&["collection", "list"]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    assert_eq!(String::from_utf8(listed.stdout).unwrap().trim(), "(none)");

    let apply = run(&["collection", "cluster", "--min-size", "3", "--apply"]);
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply_out = String::from_utf8(apply.stdout).unwrap();
    assert!(
        apply_out.contains("resources)"),
        "apply output: {apply_out}"
    );

    let listed = run(&["collection", "list", "--flat"]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let list_out = String::from_utf8(listed.stdout).unwrap();
    assert!(
        list_out.lines().filter(|l| !l.trim().is_empty()).count() >= 2,
        "expected topic collections, got: {list_out}"
    );
    assert_ne!(list_out.trim(), "(none)");
}

#[test]
fn cluster_falls_back_to_title_metadata_without_attachments() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        common::ibex()
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };

    let titles = [
        (
            "Deep Learning Foundations",
            "neural networks deep learning representation layers backpropagation",
        ),
        (
            "Convolutional Recognition",
            "convolutional neural networks deep learning image recognition layers",
        ),
        (
            "Attention Transformers",
            "transformer attention neural networks deep learning language models",
        ),
        (
            "Climate Carbon Forests",
            "climate change carbon sequestration forests greenhouse emissions",
        ),
        (
            "Global Warming Trends",
            "climate change global warming temperature greenhouse emissions",
        ),
        (
            "Ocean Carbon Dioxide",
            "climate change ocean acidification carbon dioxide greenhouse emissions",
        ),
    ];

    for (title, abstract_text) in titles {
        // CLI add has no abstract flag; encode topical words in the title.
        let created = run(&[
            "resource",
            "add",
            "--title",
            &format!("{title}: {abstract_text}"),
        ]);
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
    }

    let preview = run(&["collection", "cluster", "--min-size", "3"]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let out = String::from_utf8(preview.stdout).unwrap();
    assert!(out.contains("topic "), "{out}");
    assert!(out.contains("[metadata]"), "{out}");
}

#[test]
fn cluster_apply_twice_reuses_collections() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        common::ibex()
            .arg("--data")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };

    let papers = [
        (
            "Neural nets",
            "deep learning neural networks representation learning backpropagation layers",
        ),
        (
            "CNN survey",
            "convolutional neural networks deep learning image recognition layers",
        ),
        (
            "Transformers",
            "transformer attention neural networks deep learning language models",
        ),
        (
            "Carbon sinks",
            "climate change carbon sequestration forests greenhouse emissions",
        ),
        (
            "Warming trends",
            "climate change global warming temperature greenhouse emissions",
        ),
        (
            "Ocean acid",
            "climate change ocean acidification carbon dioxide greenhouse emissions",
        ),
    ];
    for (title, body) in papers {
        let path = dir.path().join(format!("{title}.txt"));
        fs::write(&path, body).unwrap();
        let created = run(&["resource", "add", "--title", title]);
        assert!(created.status.success());
        let id = String::from_utf8(created.stdout)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_owned();
        assert!(
            run(&[
                "resource",
                "update",
                &id,
                "--attach",
                path.to_str().unwrap()
            ])
            .status
            .success()
        );
    }

    let first = run(&["collection", "cluster", "--min-size", "3", "--apply"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_list = String::from_utf8(run(&["collection", "list", "--flat"]).stdout).unwrap();
    let first_count = first_list.lines().filter(|l| !l.trim().is_empty()).count();

    let second = run(&["collection", "cluster", "--min-size", "3", "--apply"]);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_out = String::from_utf8(second.stdout).unwrap();
    assert!(
        second_out.contains("reused"),
        "expected reuse marker: {second_out}"
    );
    let second_list = String::from_utf8(run(&["collection", "list", "--flat"]).stdout).unwrap();
    let second_count = second_list.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(
        first_count, second_count,
        "apply must not duplicate collections"
    );
}
