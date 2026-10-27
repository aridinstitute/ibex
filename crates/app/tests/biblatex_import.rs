#![cfg(feature = "testing")]

use oeuvre_app::command::{GetResource, ImportBiblatex};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, Outcome, Surface};
use oeuvre_core::utc_year_from_millis;

#[tokio::test]
async fn imports_a_book_through_the_application_command() {
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    let source = r#"@book{lovelace1843,
        author = {Lovelace, Ada},
        title = {Notes on the Analytical Engine},
        publisher = {Scientific Memoirs},
        year = {1843},
        doi = {10.1000/notes},
        custom = {Retain me}
    }"#;

    let Outcome::ImportBiblatex(report) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: source.into(),
        }))
        .await
        .unwrap()
    else {
        panic!("expected import report");
    };
    assert_eq!(report.imported.len(), 1);
    assert!(report.skipped.is_empty());
    assert!(report.warnings.is_empty());
    assert_eq!(report.imported[0].entry_key, "lovelace1843");

    let Outcome::GetResource(book) = state
        .execute(Command::GetResource(GetResource {
            id: report.imported[0].resource_id,
        }))
        .await
        .unwrap()
    else {
        panic!("expected imported resource");
    };
    assert_eq!(book.title.as_str(), "Notes on the Analytical Engine");
    assert_eq!(book.publisher.as_deref(), Some("Scientific Memoirs"));
    assert_eq!(book.creators.len(), 1);
    assert_eq!(book.metadata.citekey.as_deref(), Some("lovelace1843"));
    assert_eq!(book.doi.as_deref(), Some("10.1000/notes"));
    assert_eq!(
        book.metadata
            .source_fields
            .get("custom")
            .map(String::as_str),
        Some("Retain me")
    );
    assert_eq!(
        book.published
            .as_ref()
            .map(|d| utc_year_from_millis(d.instant().as_unix_ms())),
        Some(1843)
    );
}
