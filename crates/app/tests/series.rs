#![cfg(feature = "testing")]

use ibex_app::command::{CreateSeries, ListSeries};
use ibex_app::testing::{MemorySink, MemoryStore};
use ibex_app::{AppState, Command, Outcome, Status, Surface};
use ibex_core::{NewSeries, NonEmptyStr};

#[tokio::test]
async fn lists_empty_and_created_series_through_commands() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();
    let Outcome::ListSeries(empty) = state
        .execute(Command::ListSeries(ListSeries))
        .await
        .unwrap()
    else {
        panic!("expected ListSeries");
    };
    assert!(empty.is_empty());
    let mut ids = Vec::new();
    for title in ["Zoology", "Classics"] {
        let Outcome::CreateSeries(series) = state
            .execute(Command::CreateSeries(CreateSeries {
                new: NewSeries {
                    title: NonEmptyStr::try_new(title).unwrap(),
                },
            }))
            .await
            .unwrap()
        else {
            panic!("expected CreateSeries");
        };
        ids.push(series.id);
    }
    let Outcome::ListSeries(series) = state
        .execute(Command::ListSeries(ListSeries))
        .await
        .unwrap()
    else {
        panic!("expected ListSeries");
    };
    assert_eq!(
        series.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
        ["Classics", "Zoology"]
    );
    assert_eq!(
        series.iter().map(|s| s.id).collect::<Vec<_>>(),
        [ids[1], ids[0]]
    );
    let events = sink.take();
    assert_eq!(events.len(), 4);
    assert_eq!(events[3].command(), "list_series");
    assert_eq!(events[3].status(), Status::Ok);
}
