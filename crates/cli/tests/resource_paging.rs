use std::process::Command;

use oeuvre_adapter_surreal::SurrealStore;
use oeuvre_core::{NonEmptyStr, ResourceId, ResourceKind, Store};

const SEED_DATA: &str = "OEUVRE_PAGING_SEED_DATA";
const SEED_PARENT: &str = "OEUVRE_PAGING_SEED_PARENT";
const SEED_OUT: &str = "OEUVRE_PAGING_SEED_OUT";

/// Seeds rows past the CLI's 500-row page in a child process: SurrealKV keeps
/// its directory lock for the life of the process that opened it, so the
/// `ovr` runs that follow could not open the library from this one.
#[test]
#[ignore = "helper run by list_and_tree_preserve_every_row_and_order_across_page_boundaries"]
fn seed_paging_fixture() {
    let (Ok(data), Ok(parent), Ok(out)) = (
        std::env::var(SEED_DATA),
        std::env::var(SEED_PARENT),
        std::env::var(SEED_OUT),
    ) else {
        return;
    };
    let parent_id = ResourceId::parse(&parent).unwrap();
    let mut lines = String::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let store = SurrealStore::open(std::path::Path::new(&data))
            .await
            .unwrap();
        let template = store.get_resource(parent_id).await.unwrap().to_new();
        for index in 1..502 {
            let mut new = template.clone();
            new.title = NonEmptyStr::try_new(format!("Resource {index:03}")).unwrap();
            let resource = store.create_resource(new).await.unwrap();
            lines.push_str(&format!(
                "root\t{}\t{}  book  Resource {index:03}  Lovelace\n",
                resource.id, resource.id
            ));
        }
        for index in 0..502 {
            let mut new = template.clone();
            new.title = NonEmptyStr::try_new(format!("Section {index:03}")).unwrap();
            new.kind = ResourceKind::BookSection;
            new.parent_id = Some(parent_id);
            let resource = store.create_resource(new).await.unwrap();
            lines.push_str(&format!(
                "child\t{}\t{}  bookSection  Section {index:03}  Lovelace  in Resource 000\n",
                resource.id, resource.id
            ));
        }
    });
    std::fs::write(out, lines).unwrap();
}

#[test]
fn list_and_tree_preserve_every_row_and_order_across_page_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("library");
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_ovr"))
            .arg("--data")
            .arg(&data)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    let person = run(&["person", "add", "--last", "Lovelace"]);
    let person_id = person.split_whitespace().next().unwrap();
    let parent = run(&[
        "resource",
        "add",
        "--title",
        "Resource 000",
        "--creator",
        &format!("{person_id}:author"),
    ]);
    let parent_id = parent.split_whitespace().next().unwrap().to_owned();

    let seed_out = dir.path().join("seed.tsv");
    let seeded = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "seed_paging_fixture", "--ignored", "--quiet"])
        .env(SEED_DATA, &data)
        .env(SEED_PARENT, &parent_id)
        .env(SEED_OUT, &seed_out)
        .output()
        .unwrap();
    assert!(
        seeded.status.success(),
        "{}",
        String::from_utf8_lossy(&seeded.stdout)
    );
    let mut roots = vec![(
        parent_id.clone(),
        format!("{parent_id}  book  Resource 000  Lovelace\n"),
    )];
    let mut children = Vec::new();
    for line in std::fs::read_to_string(&seed_out).unwrap().lines() {
        let mut fields = line.splitn(3, '\t');
        let (group, id, row) = (
            fields.next().unwrap(),
            fields.next().unwrap().to_owned(),
            format!("{}\n", fields.next().unwrap()),
        );
        match group {
            "root" => roots.push((id, row)),
            _ => children.push((id, row)),
        }
    }
    assert_eq!((roots.len(), children.len()), (502, 502));

    // ULID strings sort in creation order, so descending ids are the
    // newest-first listing.
    let mut all = roots.iter().chain(&children).cloned().collect::<Vec<_>>();
    all.sort_by(|(a, _), (b, _)| b.cmp(a));
    let expected = all
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<String>();
    assert_eq!(run(&["resource", "list"]), expected);

    let expected_roots = roots
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<String>();
    assert_eq!(
        run(&["resource", "list", "--kind", "book", "--sort", "title"]),
        expected_roots
    );
    let expected_tree = roots[..1]
        .iter()
        .map(|(_, line)| line.clone())
        .chain(children.iter().map(|(_, line)| format!("  {line}")))
        .chain(roots[1..].iter().map(|(_, line)| line.clone()))
        .collect::<String>();
    assert_eq!(
        run(&["resource", "list", "--tree", "--sort", "title"]),
        expected_tree
    );
    assert_eq!(
        run(&["resource", "list", "--tree", "--title", "missing"]),
        "(none)\n"
    );
}
