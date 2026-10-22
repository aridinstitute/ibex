#![cfg(feature = "testing")]

use oeuvre_app::command::{CreatePerson, CreateResource, CreateSeries, GetResourceDetail};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, CommandError, FieldValue, Outcome, Status, Surface};
use oeuvre_core::{
    BibliographicDate, CreatorRole, CreatorSpec, Name, NewResource, NewSeries, NonEmptyStr,
    PersonId, ResourceId, ResourceKind, UtcMillis,
};

fn non_empty(s: &str) -> NonEmptyStr {
    NonEmptyStr::try_new(s).unwrap()
}

fn state_with_sink(sink: &MemorySink) -> AppState {
    AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build()
}

async fn create_person(state: &AppState, first: &str, last: &str) -> PersonId {
    let name = Name::try_new(Some(first), Some(last), None::<&str>).unwrap();
    let Outcome::CreatePerson(person) = state
        .execute(Command::CreatePerson(CreatePerson {
            name,
            aliases: vec![],
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreatePerson");
    };
    person.id
}

/// Container rows carry no creators; validation allows that for containers.
fn container_new(kind: ResourceKind, title: &str) -> NewResource {
    NewResource {
        kind,
        title: non_empty(title),
        creators: vec![],
        publisher: None,
        published: None,
        accessed: None,
        parent_id: None,
        series_id: None,
        series_number: None,
    }
}

#[tokio::test]
async fn detail_resolves_names_roles_and_linked_titles() {
    let sink = MemorySink::new();
    let state = state_with_sink(&sink);

    let ada = create_person(&state, "Ada", "Lovelace").await;
    let grace = create_person(&state, "Grace", "Hopper").await;

    let Outcome::CreateSeries(series) = state
        .execute(Command::CreateSeries(CreateSeries {
            new: NewSeries {
                title: non_empty("Loeb Classical Library"),
            },
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateSeries");
    };

    let Outcome::CreateResource(journal) = state
        .execute(Command::CreateResource(CreateResource {
            new: container_new(ResourceKind::Journal, "Journal of Citation Studies"),
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };

    let published = BibliographicDate::try_new(
        UtcMillis::from_unix_ms(1_609_459_200_000),
        Some(non_empty("Summer 2021")),
    )
    .unwrap();
    let accessed =
        BibliographicDate::try_new(UtcMillis::from_unix_ms(1_610_000_000_000), None).unwrap();

    let Outcome::CreateResource(article) = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::JournalArticle,
                title: non_empty("Quantum bibliometrics"),
                creators: vec![
                    CreatorSpec {
                        person_id: ada,
                        role: CreatorRole::Author,
                        credited_as: None,
                    },
                    CreatorSpec {
                        person_id: grace,
                        role: CreatorRole::Author,
                        credited_as: Some(non_empty("G. Hopper")),
                    },
                    // Same person in a second role: the person lookup dedups
                    // while each slot keeps its own credited_as label.
                    CreatorSpec {
                        person_id: ada,
                        role: CreatorRole::Editor,
                        credited_as: None,
                    },
                ],
                publisher: Some("Citation Society Press".into()),
                published: Some(published.clone()),
                accessed: Some(accessed.clone()),
                parent_id: Some(journal.id),
                series_id: Some(series.id),
                series_number: Some(non_empty("42")),
            },
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };

    let Outcome::GetResourceDetail(detail) = state
        .execute(Command::GetResourceDetail(GetResourceDetail {
            id: article.id,
        }))
        .await
        .unwrap()
    else {
        panic!("expected GetResourceDetail");
    };

    assert_eq!(detail.id, article.id);
    assert_eq!(detail.kind, ResourceKind::JournalArticle);
    assert_eq!(detail.title.as_str(), "Quantum bibliometrics");
    assert_eq!(detail.creators.len(), 2);
    assert_eq!(detail.creators[0].role, CreatorRole::Author);
    assert_eq!(detail.creators[0].names, vec!["Ada Lovelace", "G. Hopper"]);
    assert_eq!(detail.creators[1].role, CreatorRole::Editor);
    assert_eq!(detail.creators[1].names, vec!["Ada Lovelace"]);
    assert_eq!(detail.publisher.as_deref(), Some("Citation Society Press"));
    assert_eq!(detail.published, Some(published));
    assert_eq!(detail.accessed, Some(accessed));
    assert_eq!(
        detail.container_title.as_ref().map(NonEmptyStr::as_str),
        Some("Journal of Citation Studies")
    );
    assert_eq!(
        detail.series_title.as_ref().map(NonEmptyStr::as_str),
        Some("Loeb Classical Library")
    );
    assert_eq!(
        detail.series_number.as_ref().map(NonEmptyStr::as_str),
        Some("42")
    );
    // The detail carries the record-keeping stamps the Added and Edited
    // rows read; nothing edited the record, so both hold the create stamp.
    assert_eq!(detail.date_added, article.date_added);
    assert_eq!(detail.date_edited, article.date_edited);
}

#[tokio::test]
async fn detail_not_found_on_missing_id() {
    let sink = MemorySink::new();
    let state = state_with_sink(&sink);

    let id = ResourceId::generate();
    let err = state
        .execute(Command::GetResourceDetail(GetResourceDetail { id }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::NotFound(_)));

    let events = sink.take();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].command(), "get_resource_detail");
    assert_eq!(events[0].status(), Status::Error);
    assert_eq!(
        events[0].get("error.kind"),
        Some(&FieldValue::from("not_found"))
    );
    assert_eq!(
        events[0].get("resource.id"),
        Some(&FieldValue::from(id.to_string()))
    );
}

#[tokio::test]
async fn detail_emits_exactly_one_ok_event() {
    let sink = MemorySink::new();
    let state = state_with_sink(&sink);

    let augustine = create_person(&state, "Augustine", "").await;
    let Outcome::CreateResource(book) = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::Book,
                title: non_empty("Confessions"),
                creators: vec![CreatorSpec {
                    person_id: augustine,
                    role: CreatorRole::Author,
                    credited_as: None,
                }],
                publisher: None,
                published: None,
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
            },
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };

    // Setup commands each log their own event; count only the detail call.
    sink.take();
    let Outcome::GetResourceDetail(detail) = state
        .execute(Command::GetResourceDetail(GetResourceDetail { id: book.id }))
        .await
        .unwrap()
    else {
        panic!("expected GetResourceDetail");
    };
    assert_eq!(detail.creators[0].names, vec!["Augustine"]);

    let events = sink.take();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].command(), "get_resource_detail");
    assert_eq!(events[0].status(), Status::Ok);
    assert_eq!(
        events[0].get("resource.id"),
        Some(&FieldValue::from(book.id.to_string()))
    );
    assert_eq!(
        events[0].get("resource.kind"),
        Some(&FieldValue::from("book"))
    );
    assert_eq!(
        events[0].get("resource.title"),
        Some(&FieldValue::from("Confessions"))
    );
}
