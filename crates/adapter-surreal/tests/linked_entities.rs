//! Real-adapter coverage for Task 04 linked publication entities.

use oeuvre_adapter_surreal::SurrealStore;
use oeuvre_app::command::{ImportBiblatex, ListResources};
use oeuvre_app::{AppState, Command, Outcome, Surface};
use oeuvre_core::{ResourceKind, Store};

#[tokio::test]
async fn shared_issue_and_series_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library");
    let store = SurrealStore::open(&path).await.unwrap();
    let state = AppState::builder(store).surface(Surface::Test).build();
    let source = r#"
        @article{a1, title={First}, author={Ada, Lovelace},
          journaltitle={Nature}, issn={0028-0836}, volume={12}, number={3}}
        @article{a2, title={Second}, author={Hopper, Grace},
          journaltitle={Nature}, issn={0028-0836}, volume={12}, number={3}}
        @book{parent, title={Collected}, isbn={978-0-306-40615-7},
          series={Essay Series}, number={1}}
        @incollection{chapter, title={Chapter}, author={Turing, Alan},
          crossref={parent}}
    "#;

    let Outcome::ImportBiblatex(report) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: source.into(),
        }))
        .await
        .unwrap()
    else {
        panic!("import");
    };
    assert_eq!(report.imported.len(), 4);

    let Outcome::ListResources(rows) = state
        .execute(Command::ListResources(ListResources))
        .await
        .unwrap()
    else {
        panic!("list");
    };
    assert_eq!(
        rows.iter()
            .filter(|r| r.kind == ResourceKind::Journal)
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r.kind == ResourceKind::Issue)
            .count(),
        1
    );

    let issue_id = rows
        .iter()
        .find(|r| r.kind == ResourceKind::Issue)
        .unwrap()
        .id;
    let a1 = report
        .imported
        .iter()
        .find(|e| e.entry_key == "a1")
        .unwrap()
        .resource_id;
    let chapter_id = report
        .imported
        .iter()
        .find(|e| e.entry_key == "chapter")
        .unwrap()
        .resource_id;
    let book_id = report
        .imported
        .iter()
        .find(|e| e.entry_key == "parent")
        .unwrap()
        .resource_id;

    drop(state);

    let reopened = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match SurrealStore::open(&path).await {
                Ok(store) => break store,
                Err(err) if err.message.contains("already locked") => {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Err(err) => panic!("reopen failed: {err}"),
            }
        }
    })
    .await
    .expect("database lock released");

    let a1 = reopened.get_resource(a1).await.unwrap();
    assert_eq!(a1.parent_id, Some(issue_id));

    let chapter = reopened.get_resource(chapter_id).await.unwrap();
    assert_eq!(chapter.parent_id, Some(book_id));

    let book = reopened.get_resource(book_id).await.unwrap();
    assert!(book.series_id.is_some());
    assert_eq!(book.series_number.as_ref().map(|s| s.as_str()), Some("1"));
}
