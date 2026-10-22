#![cfg(feature = "testing")]

use oeuvre_app::command::{CreatePerson, CreateResource, ListResourcesPage};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, CommandError, FieldValue, Outcome, Status, Surface};
use oeuvre_core::{
    CreatorRole, CreatorSpec, Name, NewResource, NonEmptyStr, PersonId, ResourceId, ResourceKind,
};

async fn seed_author(state: &AppState) -> PersonId {
    let name = Name::try_new(None::<&str>, Some("Augustine"), None::<&str>).unwrap();
    let Outcome::CreatePerson(author) = state
        .execute(Command::CreatePerson(CreatePerson {
            name,
            aliases: vec![],
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreatePerson");
    };
    author.id
}

fn book_new(author_id: PersonId, title: &str) -> NewResource {
    NewResource {
        kind: ResourceKind::Book,
        title: NonEmptyStr::try_new(title).unwrap(),
        creators: vec![CreatorSpec {
            person_id: author_id,
            role: CreatorRole::Author,
            credited_as: None,
        }],
        publisher: None,
        published: None,
        accessed: None,
        parent_id: None,
        series_id: None,
        series_number: None,
    }
}

fn state_with_sink(sink: &MemorySink) -> AppState {
    AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build()
}

fn page_call(before: Option<ResourceId>, limit: u32) -> Command {
    Command::ListResourcesPage(ListResourcesPage { before, limit })
}

#[tokio::test]
async fn pages_walk_the_library_newest_first() {
    let sink = MemorySink::new();
    let state = state_with_sink(&sink);

    let author_id = seed_author(&state).await;
    let mut created = Vec::new();
    for n in 1..=5 {
        let Outcome::CreateResource(book) = state
            .execute(Command::CreateResource(CreateResource {
                new: book_new(author_id, &format!("Study {n}")),
            }))
            .await
            .unwrap()
        else {
            panic!("expected CreateResource");
        };
        created.push(book.id);
    }

    // Creation order can tie within one ULID millisecond; the cursor contract
    // is id ordering, so derive expectations from the ids themselves.
    let mut expected = created;
    expected.sort_by(|a, b| b.as_ulid().cmp(&a.as_ulid()));

    let mut walked = Vec::new();
    let mut before = None;
    loop {
        let Outcome::ListResourcesPage(page) = state.execute(page_call(before, 2)).await.unwrap()
        else {
            panic!("expected ListResourcesPage");
        };
        assert!(
            !page.rows.is_empty() || page.next_before.is_none(),
            "cursor must not yield an empty page with rows remaining"
        );
        assert!(page.rows.len() <= 2);
        assert!(
            page.rows
                .iter()
                .all(|row| before.is_none_or(|b| row.id.as_ulid() < b.as_ulid()))
        );
        if let Some(next) = page.next_before {
            assert_eq!(Some(next), page.rows.last().map(|row| row.id));
        }
        assert!(
            page.rows
                .iter()
                .all(|row| row.creators_label == "Augustine")
        );
        walked.extend(page.rows.iter().map(|row| row.id));
        match page.next_before {
            Some(next) => before = Some(next),
            None => break,
        }
    }
    assert_eq!(walked, expected);
}

#[tokio::test]
async fn zero_limit_is_invalid() {
    let sink = MemorySink::new();
    let state = state_with_sink(&sink);

    let err = state.execute(page_call(None, 0)).await.unwrap_err();
    assert!(matches!(err, CommandError::Invalid(_)));

    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.status(), Status::Error);
    assert_eq!(ev.get("error.kind"), Some(&FieldValue::from("invalid")));
}

#[tokio::test]
async fn empty_library_first_page_is_empty() {
    let sink = MemorySink::new();
    let state = state_with_sink(&sink);

    let Outcome::ListResourcesPage(page) = state.execute(page_call(None, 5)).await.unwrap() else {
        panic!("expected ListResourcesPage");
    };
    assert!(page.rows.is_empty());
    assert!(page.next_before.is_none());
}

#[tokio::test]
async fn list_resources_page_emits_wide_event_fields() {
    let sink = MemorySink::new();
    let state = state_with_sink(&sink);

    let author_id = seed_author(&state).await;
    for n in 1..=3 {
        let Outcome::CreateResource(_) = state
            .execute(Command::CreateResource(CreateResource {
                new: book_new(author_id, &format!("Study {n}")),
            }))
            .await
            .unwrap()
        else {
            panic!("expected CreateResource");
        };
    }
    let _ = sink.take();

    let Outcome::ListResourcesPage(first) = state.execute(page_call(None, 2)).await.unwrap() else {
        panic!("expected ListResourcesPage");
    };
    assert_eq!(first.rows.len(), 2);
    let next = first.next_before.expect("three books leave one more page");

    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.command(), "list_resources_page");
    assert_eq!(ev.status(), Status::Ok);
    assert_eq!(ev.get("resource.page_limit"), Some(&FieldValue::from(2u32)));
    assert!(
        ev.get("resource.id").is_none(),
        "no cursor means no resource.id field"
    );

    let _ = state.execute(page_call(Some(next), 2)).await.unwrap();
    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(
        ev.get("resource.id"),
        Some(&FieldValue::from(next.to_string()))
    );
}
