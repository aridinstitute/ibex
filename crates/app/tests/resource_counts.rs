#![cfg(feature = "testing")]

use oeuvre_app::command::{CountResources, CreateResource};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, Outcome, Status, Surface};
use oeuvre_core::{NewResource, NonEmptyStr, ResourceKind};

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
