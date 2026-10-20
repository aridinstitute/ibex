#![cfg(feature = "testing")]

use oeuvre_app::command::{CreatePerson, CreateResource, CreateSeries, GetResource};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, CommandError, FieldValue, Outcome, Status, Surface};
use oeuvre_core::{
    CreatorRole, CreatorSpec, Name, NewResource, NewSeries, NonEmptyStr, PersonId, ResourceId,
    ResourceKind,
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

#[tokio::test]
async fn create_book_after_create_person() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let author_id = seed_author(&state).await;
    let _ = sink.take();

    let Outcome::CreateResource(resource) = state
        .execute(Command::CreateResource(CreateResource {
            new: book_new(author_id, "Confessions"),
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
    assert_eq!(resource.creators[0].order, 0);
}

#[tokio::test]
async fn get_resource_after_create_round_trip() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let author_id = seed_author(&state).await;

    let Outcome::CreateResource(created) = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new("Confessions").unwrap(),
                creators: vec![CreatorSpec {
                    person_id: author_id,
                    role: CreatorRole::Author,
                    credited_as: Some(NonEmptyStr::try_new("St. Augustine").unwrap()),
                }],
                publisher: Some("Harvard".into()),
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

    let Outcome::GetResource(loaded) = state
        .execute(Command::GetResource(GetResource { id: created.id }))
        .await
        .unwrap()
    else {
        panic!("expected GetResource");
    };

    assert_eq!(loaded.id, created.id);
    assert_eq!(loaded.creators.len(), 1);
    assert_eq!(loaded.creators[0].person_id, author_id);
    assert_eq!(loaded.creators[0].order, 0);
    assert_eq!(
        loaded.creators[0]
            .credited_as
            .as_ref()
            .map(NonEmptyStr::as_str),
        Some("St. Augustine")
    );
    assert_eq!(loaded.publisher.as_deref(), Some("Harvard"));
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
            new: book_new(missing, "Orphan"),
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
                publisher: None,
                published: None,
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
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
        panic!("expected CreateResource podcast");
    };

    let Outcome::CreateResource(episode) = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::PodcastEpisode,
                title: NonEmptyStr::try_new("Ep 1").unwrap(),
                creators: vec![CreatorSpec {
                    person_id: host_id,
                    role: CreatorRole::Host,
                    credited_as: Some(NonEmptyStr::try_new("as Host").unwrap()),
                }],
                publisher: None,
                published: None,
                accessed: None,
                parent_id: Some(show.id),
                series_id: None,
                series_number: None,
            },
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource episode");
    };

    assert_eq!(episode.parent_id, Some(show.id));
    assert_eq!(episode.kind, ResourceKind::PodcastEpisode);

    let Outcome::GetResource(loaded) = state
        .execute(Command::GetResource(GetResource { id: episode.id }))
        .await
        .unwrap()
    else {
        panic!("expected GetResource");
    };
    assert_eq!(loaded.parent_id, Some(show.id));
    assert_eq!(
        loaded.creators[0]
            .credited_as
            .as_ref()
            .map(NonEmptyStr::as_str),
        Some("as Host")
    );
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
                creators: vec![CreatorSpec {
                    person_id: author_id,
                    role: CreatorRole::Host,
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
        .unwrap_err();
    assert!(matches!(err, CommandError::Invalid(_)));
}

#[tokio::test]
async fn create_episode_missing_parent_is_not_found() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let author_id = seed_author(&state).await;
    let missing_parent = ResourceId::generate();

    let err = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::PodcastEpisode,
                title: NonEmptyStr::try_new("Ep 1").unwrap(),
                creators: vec![CreatorSpec {
                    person_id: author_id,
                    role: CreatorRole::Host,
                    credited_as: None,
                }],
                publisher: None,
                published: None,
                accessed: None,
                parent_id: Some(missing_parent),
                series_id: None,
                series_number: None,
            },
        }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::NotFound(_)));
}

#[tokio::test]
async fn create_episode_wrong_parent_kind_is_invalid() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let author_id = seed_author(&state).await;

    let Outcome::CreateResource(blog) = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::Blog,
                title: NonEmptyStr::try_new("Blog").unwrap(),
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
        .unwrap()
    else {
        panic!("expected CreateResource blog");
    };

    let err = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::PodcastEpisode,
                title: NonEmptyStr::try_new("Ep 1").unwrap(),
                creators: vec![CreatorSpec {
                    person_id: author_id,
                    role: CreatorRole::Host,
                    credited_as: None,
                }],
                publisher: None,
                published: None,
                accessed: None,
                parent_id: Some(blog.id),
                series_id: None,
                series_number: None,
            },
        }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Invalid(_)));
}

#[tokio::test]
async fn create_resource_series_number_without_series_is_invalid() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let author_id = seed_author(&state).await;

    let err = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new("Confessions").unwrap(),
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
                series_number: Some(NonEmptyStr::try_new("LCL 26").unwrap()),
            },
        }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Invalid(_)));
}

#[tokio::test]
async fn create_resource_with_series() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let author_id = seed_author(&state).await;

    let Outcome::CreateSeries(series) = state
        .execute(Command::CreateSeries(CreateSeries {
            new: NewSeries {
                title: NonEmptyStr::try_new("Loeb Classical Library").unwrap(),
            },
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateSeries");
    };

    let Outcome::CreateResource(book) = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new("Confessions").unwrap(),
                creators: vec![CreatorSpec {
                    person_id: author_id,
                    role: CreatorRole::Author,
                    credited_as: None,
                }],
                publisher: Some("Harvard University Press".into()),
                published: None,
                accessed: None,
                parent_id: None,
                series_id: Some(series.id),
                series_number: Some(NonEmptyStr::try_new("LCL 26").unwrap()),
            },
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };

    assert_eq!(book.series_id, Some(series.id));
    assert_eq!(
        book.series_number.as_ref().map(NonEmptyStr::as_str),
        Some("LCL 26")
    );
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
            new: book_new(author_id, "Confessions"),
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
    assert_eq!(ev.get("resource.kind"), Some(&FieldValue::from("book")));
    assert_eq!(
        ev.get("resource.title"),
        Some(&FieldValue::from("Confessions"))
    );
}
