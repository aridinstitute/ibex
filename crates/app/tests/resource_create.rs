#![cfg(feature = "testing")]

use oeuvre_app::command::{CreatePerson, CreateResource};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, CommandError, FieldValue, Outcome, Status, Surface};
use oeuvre_core::{
    Creator, CreatorRole, Name, NewResource, NonEmptyStr, PersonId, ResourceKind,
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

#[tokio::test]
async fn create_book_after_create_person() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let author_id = seed_author(&state).await;
    let _ = sink.take();

    let title = NonEmptyStr::try_new("Confessions").unwrap();
    let Outcome::CreateResource(resource) = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::Book,
                title,
                creators: vec![Creator {
                    person_id: author_id,
                    role: CreatorRole::Author,
                }],
                published: None,
                accessed: None,
                parent_id: None,
            },
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };

    assert_eq!(resource.kind, ResourceKind::Book);
    assert_eq!(resource.title.as_str(), "Confessions");
    assert_eq!(resource.creators.len(), 1);
    assert_eq!(resource.creators[0].person_id, author_id);
    assert_eq!(resource.creators[0].role, CreatorRole::Author);
}

#[tokio::test]
async fn create_resource_missing_person_is_not_found() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let missing = PersonId::generate();
    let err = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new("Orphan").unwrap(),
                creators: vec![Creator {
                    person_id: missing,
                    role: CreatorRole::Author,
                }],
                published: None,
                accessed: None,
                parent_id: None,
            },
        }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::NotFound(_)));
}

#[tokio::test]
async fn create_resource_empty_creators_is_invalid() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let err = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new("Empty").unwrap(),
                creators: vec![],
                published: None,
                accessed: None,
                parent_id: None,
            },
        }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Invalid(_)));
}

#[tokio::test]
async fn create_podcast_episode_with_podcast_parent() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let host_id = seed_author(&state).await;

    let Outcome::CreateResource(show) = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::Podcast,
                title: NonEmptyStr::try_new("Show").unwrap(),
                creators: vec![],
                published: None,
                accessed: None,
                parent_id: None,
            },
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource podcast");
    };

    let Outcome::CreateResource(episode) = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::PodcastEpisode,
                title: NonEmptyStr::try_new("Ep 1").unwrap(),
                creators: vec![Creator {
                    person_id: host_id,
                    role: CreatorRole::Host,
                }],
                published: None,
                accessed: None,
                parent_id: Some(show.id),
            },
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource episode");
    };

    assert_eq!(episode.parent_id, Some(show.id));
    assert_eq!(episode.kind, ResourceKind::PodcastEpisode);
}

#[tokio::test]
async fn create_podcast_episode_without_parent_is_invalid() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let author_id = seed_author(&state).await;

    let err = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::PodcastEpisode,
                title: NonEmptyStr::try_new("Ep 1").unwrap(),
                creators: vec![Creator {
                    person_id: author_id,
                    role: CreatorRole::Host,
                }],
                published: None,
                accessed: None,
                parent_id: None,
            },
        }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Invalid(_)));
}

#[tokio::test]
async fn create_resource_emits_wide_event_fields() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let author_id = seed_author(&state).await;
    let _ = sink.take();

    let Outcome::CreateResource(resource) = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new("Confessions").unwrap(),
                creators: vec![Creator {
                    person_id: author_id,
                    role: CreatorRole::Author,
                }],
                published: None,
                accessed: None,
                parent_id: None,
            },
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };

    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.command(), "create_resource");
    assert_eq!(ev.status(), Status::Ok);
    assert_eq!(
        ev.get("resource.id"),
        Some(&FieldValue::from(resource.id.to_string()))
    );
    assert_eq!(
        ev.get("resource.kind"),
        Some(&FieldValue::from("book"))
    );
    assert_eq!(
        ev.get("resource.title"),
        Some(&FieldValue::from("Confessions"))
    );
}
