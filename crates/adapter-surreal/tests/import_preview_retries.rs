//! Task 06: retry idempotency through the real Surreal adapter.

use oeuvre_app::command::{ImportBiblatex, ListResources};
use oeuvre_app::{AppState, Command, Outcome, Surface};
use oeuvre_adapter_surreal::SurrealStore;

#[tokio::test]
async fn retry_same_source_reuses_committed_entries_through_surreal() {
    let state = AppState::builder(SurrealStore::open_memory().await.unwrap())
        .surface(Surface::Test)
        .build();
    let source = r#"
        @book{a, title={First}, author={Ada, Lovelace}, year={1843}, doi={10.1000/first}}
        @book{b, title={Second}, author={Hopper, Grace}, year={1952}, doi={10.1000/second}}
    "#;
    let Outcome::ImportBiblatex(first) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: source.into(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("first");
    };
    assert_eq!(first.imported.len(), 2);

    let Outcome::ImportBiblatex(second) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: source.into(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("retry");
    };
    assert!(second.imported.is_empty(), "{second:?}");
    assert_eq!(second.reused.len(), 2, "{second:?}");

    let Outcome::ListResources(rows) = state
        .execute(Command::ListResources(ListResources))
        .await
        .unwrap()
    else {
        panic!("list");
    };
    assert_eq!(rows.len(), 2);
}
