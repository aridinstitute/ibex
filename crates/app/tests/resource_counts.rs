#![cfg(feature = "testing")]

use ibex_app::command::{CountResources, CreateResource};
use ibex_app::testing::{MemorySink, MemoryStore};
use ibex_app::{AppState, Command, Outcome, Status, Surface};
use ibex_core::{NewResource, NonEmptyStr, ResourceKind};

#[tokio::test]
async fn counts_report_the_whole_library_and_emit_one_event() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();
    for (kind, title) in [
        (ResourceKind::Book, "First"),
        (ResourceKind::Book, "Second"),
        (ResourceKind::Report, "Third"),
    ] {
        state
            .execute(Command::CreateResource(CreateResource {
                new: NewResource {
                    kind,
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
                    abstract_text: None,
                    keywords: vec![],
                },
            }))
            .await
            .unwrap();
    }
    let Outcome::CountResources(counts) = state
        .execute(Command::CountResources(CountResources))
        .await
        .unwrap()
    else {
        panic!("expected counts");
    };
    assert_eq!(counts.all_items, 3);
    assert_eq!(counts.favorites, 0);
    assert_eq!(counts.by_kind.get(&ResourceKind::Book), Some(&2));
    assert_eq!(counts.by_kind.get(&ResourceKind::Report), Some(&1));
    assert_eq!(counts.by_kind.get(&ResourceKind::Thesis), None);
    let events = sink.take();
    assert_eq!(events.len(), 4);
    assert_eq!(events[3].command(), "count_resources");
    assert_eq!(events[3].status(), Status::Ok);
}
