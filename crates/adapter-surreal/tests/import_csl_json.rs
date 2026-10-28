//! Task 07: CSL-JSON import through the real Surreal adapter.

use oeuvre_adapter_surreal::SurrealStore;
use oeuvre_app::command::{GetResource, ImportCslJson, ListResources};
use oeuvre_app::{AppState, Command, Outcome, Surface};
use oeuvre_core::ResourceKind;

#[tokio::test]
async fn csl_json_import_and_retry_through_surreal() {
    let state = AppState::builder(SurrealStore::open_memory().await.unwrap())
        .surface(Surface::Test)
        .build();
    let source = r#"[
      {
        "id": "lovelace1843",
        "type": "book",
        "title": "Notes on the Analytical Engine",
        "author": [{"family": "Lovelace", "given": "Ada"}],
        "publisher": "Scientific Memoirs",
        "issued": {"date-parts": [[1843]]},
        "DOI": "10.1000/notes"
      },
      {
        "id": "hopper1952",
        "type": "article-journal",
        "title": "The Education of a Computer",
        "author": [{"family": "Hopper", "given": "Grace"}],
        "container-title": "Proceedings of the ACM",
        "volume": "1",
        "issue": "2",
        "issued": {"date-parts": [[1952]]},
        "DOI": "10.1145/609.123"
      }
    ]"#;

    let fingerprint =
        oeuvre_core::source_fingerprint(oeuvre_core::ImportFormat::CslJson, source);
    let Outcome::ImportCslJson(first) = state
        .execute(Command::ImportCslJson(ImportCslJson {
            source_text: source.to_owned(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("first");
    };
    assert_eq!(first.imported.len(), 2, "{first:?}");

    let Outcome::ListResources(rows) = state
        .execute(Command::ListResources(ListResources))
        .await
        .unwrap()
    else {
        panic!("list");
    };
    assert!(
        rows.iter().any(|r| r.kind == ResourceKind::Journal),
        "expected inferred journal: {rows:?}"
    );
    assert!(
        rows.iter().any(|r| r.kind == ResourceKind::Issue),
        "expected inferred issue: {rows:?}"
    );

    let book_id = first
        .imported
        .iter()
        .find(|e| e.entry_key == "lovelace1843")
        .unwrap()
        .resource_id;
    let Outcome::GetResource(book) = state
        .execute(Command::GetResource(GetResource { id: book_id }))
        .await
        .unwrap()
    else {
        panic!("book");
    };
    assert_eq!(book.doi.as_deref(), Some("10.1000/notes"));
    assert_eq!(
        book.metadata.provenance.source_fingerprint.as_deref(),
        Some(fingerprint.as_str())
    );

    let Outcome::ImportCslJson(retry) = state
        .execute(Command::ImportCslJson(ImportCslJson {
            source_text: source.to_owned(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("retry");
    };
    assert!(retry.imported.is_empty(), "{retry:?}");
    assert_eq!(retry.reused.len(), 2, "{retry:?}");
}
