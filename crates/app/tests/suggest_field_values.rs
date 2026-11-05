#![cfg(feature = "testing")]
use ibex_app::command::SuggestFieldValues;
use ibex_app::testing::{MemorySink, MemoryStore};
use ibex_app::{AppState, Command, Outcome, Status, Surface};
use ibex_core::{NewResource, NonEmptyStr, ResourceKind, ResourceTextField, Store};
fn bare_book(title: &str) -> NewResource {
    NewResource {
        kind: ResourceKind::Book,
        title: NonEmptyStr::try_new(title).unwrap(),
        creators: vec![],
        publisher: None,
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
        school: None,
        abstract_text: None,
        keywords: vec![],
    }
}

#[tokio::test]
async fn suggestions_match_only_selected_field_and_emit_one_event() {
    let store = MemoryStore::new();
    for value in [
        "zeta Press",
        "Alpha Press",
        "alpha press",
        "",
        "   ",
        "Beta Custom",
    ] {
        let mut new = bare_book("Not a suggestion");
        new.publisher = Some(value.into());
        new.institution = Some("Other field".into());
        store.create_resource(new).await.unwrap();
    }
    let sink = MemorySink::new();
    let state = AppState::builder(store)
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();
    let Outcome::SuggestFieldValues(values) = state
        .execute(Command::SuggestFieldValues(SuggestFieldValues {
            field: ResourceTextField::Publisher,
            query: "PrEs".into(),
            limit: 10,
        }))
        .await
        .unwrap()
    else {
        panic!("suggestions outcome")
    };
    assert_eq!(values, ["Alpha Press", "zeta Press"]);
    let events = sink.take();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].command(), "suggest_field_values");
    assert_eq!(events[0].status(), Status::Ok);
    assert_eq!(
        events[0].get("resource.suggestion_field"),
        Some(&ibex_app::FieldValue::from("publisher"))
    );
    assert_eq!(
        events[0].get("resource.suggestion_query_length"),
        Some(&ibex_app::FieldValue::from(4usize))
    );
    assert_eq!(
        events[0].get("resource.suggestion_count"),
        Some(&ibex_app::FieldValue::from(2usize))
    );
}

#[tokio::test]
async fn app_suggestions_preserve_custom_values_for_every_supported_field() {
    use ResourceTextField::*;
    let store = MemoryStore::new();
    for value in [
        Some("zeta Custom"),
        Some("Alpha Custom"),
        Some("alpha custom"),
        Some(""),
        Some("  "),
        None,
    ] {
        let mut new = bare_book("Unrelated title");
        let value = value.map(str::to_owned);
        new.publisher = value.clone();
        new.institution = value.clone();
        new.organization = value.clone();
        new.school = value.clone();
        new.document_type = value.clone();
        new.publication_method = value.clone();
        new.language = value;
        store.create_resource(new).await.unwrap();
    }
    let state = AppState::builder(store).surface(Surface::Test).build();
    for field in [
        Publisher,
        Institution,
        Organization,
        School,
        DocumentType,
        PublicationMethod,
        Language,
    ] {
        for (query, limit, expected) in [
            ("UsTo", 10, vec!["Alpha Custom", "zeta Custom"]),
            ("", 10, vec!["Alpha Custom", "zeta Custom"]),
            ("", 1, vec!["Alpha Custom"]),
            ("", 0, vec![]),
            ("Unrelated", 10, vec![]),
        ] {
            let Outcome::SuggestFieldValues(values) = state
                .execute(Command::SuggestFieldValues(SuggestFieldValues {
                    field,
                    query: query.into(),
                    limit,
                }))
                .await
                .unwrap()
            else {
                panic!("suggestions outcome")
            };
            assert_eq!(values, expected, "{field:?}");
        }
    }
}
