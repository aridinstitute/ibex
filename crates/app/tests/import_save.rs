#![cfg(feature = "testing")]

//! Shared orchestration tests: any draft feeds the same persistence path.

use ibex_app::command::{GetResource, ListPersons};
use ibex_app::import::save_draft;
use ibex_app::testing::{MemorySink, MemoryStore};
use ibex_app::{AppState, Command, Outcome, Surface};
use ibex_core::{
    CreatorRole, EntryProvenance, IMPORT_PARSER_VERSION, ImportDraft, ImportEntryDraft,
    ImportFormat, ImportTempId, Name, NewResource, NonEmptyStr, ResourceKind, SourceProvenance,
    UnresolvedCreator,
};

fn book_draft(key: &str, title: &str, author: Name) -> ImportDraft {
    ImportDraft {
        provenance: SourceProvenance {
            format: ImportFormat::Test,
            parser_version: IMPORT_PARSER_VERSION,
        },
        source_fingerprint: Some(ibex_core::source_fingerprint(
            ImportFormat::Test,
            &format!("{key}\0{title}"),
        )),
        entries: vec![ImportEntryDraft {
            temp_id: ImportTempId::new(1),
            resource: NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new(title).unwrap(),
                creators: vec![],
                publisher: Some("Test Press".into()),
                published: None,
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
                tag_ids: vec![],
                collection_ids: vec![],
                metadata: Default::default(),
                subtitle: None,
                title_addon: None,
                short_title: None,
                place: None,
                volume: None,
                issue: None,
                pages: None,
                total_pages: None,
                article_number: None,
                edition: None,
                doi: None,
                url: None,
                eprint: None,
                isbns: vec![],
                issns: vec![],
                isrns: vec![],
                original_date: None,
                event_date: None,
                language: None,
                document_type: None,
                publication_method: None,
                institution: None,
                organization: None,
                abstract_text: None,
                keywords: vec![],
            },
            unresolved_creators: vec![UnresolvedCreator {
                role: CreatorRole::Author,
                name: author,
            }],
            series_title: Some("Test Series".into()),
            series_number: Some(NonEmptyStr::try_new("3").unwrap()),
            provenance: EntryProvenance {
                source_key: key.into(),
                entry_type: Some("book".into()),
                original_fields: Default::default(),
                expanded_fields: Default::default(),
                inherited_origins: Default::default(),
                note: None,
                annotation: None,
            },
            had_reference_fields: false,
            publication_ref_keys: vec![],
            had_xdata: false,
        }],
        diagnostics: vec![],
    }
}

#[tokio::test]
async fn test_format_draft_saves_through_shared_orchestration() {
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    let draft = book_draft(
        "test-key",
        "From a test parser",
        Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
    );
    let report = save_draft(&state, draft).await.expect("save draft");
    assert_eq!(report.imported.len(), 1);
    assert_eq!(report.imported[0].entry_key, "test-key");
    assert!(report.skipped.is_empty());

    let Outcome::GetResource(book) = state
        .execute(Command::GetResource(GetResource {
            id: report.imported[0].resource_id,
        }))
        .await
        .unwrap()
    else {
        panic!("resource");
    };
    assert_eq!(book.title.as_str(), "From a test parser");
    assert_eq!(book.creators.len(), 1);
    assert!(book.series_id.is_some());
    assert_eq!(book.series_number.as_ref().map(|s| s.as_str()), Some("3"));

    let Outcome::ListPersons(people) = state
        .execute(Command::ListPersons(ListPersons))
        .await
        .unwrap()
    else {
        panic!("people");
    };
    assert_eq!(people.len(), 1);
    assert_eq!(people[0].name.display(), "Ada Lovelace");
}
