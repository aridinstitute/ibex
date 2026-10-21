#![cfg(feature = "testing")]

use oeuvre_app::command::{
    CreatePerson, CreateResource, GetResource, UpdateResource,
};
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

#[tokio::test]
async fn update_title_round_trip() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let author_id = seed_author(&state).await;

    let Outcome::CreateResource(created) = state
        .execute(Command::CreateResource(CreateResource {
            new: book_new(author_id, "Confessions"),
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };

    // Full-replace edit: the write-side twin of the stored record with the
    // title swapped, the merge every shell performs.
    let mut edit = created.to_new();
    edit.title = NonEmptyStr::try_new("Retitled").unwrap();

    let Outcome::UpdateResource(updated) = state
        .execute(Command::UpdateResource(UpdateResource {
            id: created.id,
            new: edit,
        }))
        .await
        .unwrap()
    else {
        panic!("expected UpdateResource");
    };
    assert_eq!(updated.id, created.id);
    assert_eq!(updated.title.as_str(), "Retitled");
    assert_eq!(updated.creators.len(), 1);
    assert_eq!(updated.creators[0].person_id, author_id);
    assert_eq!(updated.creators[0].role, CreatorRole::Author);
    assert_eq!(updated.creators[0].order, 0);

    let Outcome::GetResource(got) = state
        .execute(Command::GetResource(GetResource { id: created.id }))
        .await
        .unwrap()
    else {
        panic!("expected GetResource");
    };
    assert_eq!(got.title.as_str(), "Retitled");
    assert_eq!(got.creators.len(), 1);
    assert_eq!(got.creators[0].person_id, author_id);
}

#[tokio::test]
async fn update_missing_resource_is_not_found() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let author_id = seed_author(&state).await;
    let _ = sink.take();

    let id = ResourceId::generate();
    let err = state
        .execute(Command::UpdateResource(UpdateResource {
            id,
            new: book_new(author_id, "Ghost"),
        }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::NotFound(_)));

    let events = sink.take();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status(), Status::Error);
    assert_eq!(events[0].command(), "update_resource");
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
async fn update_kind_change_without_required_parent_is_invalid() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let author_id = seed_author(&state).await;
    let Outcome::CreateResource(created) = state
        .execute(Command::CreateResource(CreateResource {
            new: book_new(author_id, "Confessions"),
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };
    let _ = sink.take();

    // podcastEpisode requires a podcast parent, so the re-typed record
    // fails shape validation inside the store write.
    let mut edit = created.to_new();
    edit.kind = ResourceKind::PodcastEpisode;
    let err = state
        .execute(Command::UpdateResource(UpdateResource {
            id: created.id,
            new: edit,
        }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Invalid(_)));

    let events = sink.take();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].get("error.kind"),
        Some(&FieldValue::from("invalid"))
    );
    // The diff lands even though the write never happened.
    assert_eq!(
        events[0].get("resource.changed_fields"),
        Some(&FieldValue::from("kind"))
    );
}

#[tokio::test]
async fn update_emits_exactly_one_ok_event_with_changed_fields() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let author_id = seed_author(&state).await;
    let Outcome::CreateResource(created) = state
        .execute(Command::CreateResource(CreateResource {
            new: book_new(author_id, "Confessions"),
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };
    let _ = sink.take();

    let mut edit = created.to_new();
    edit.title = NonEmptyStr::try_new("Retitled").unwrap();
    let Outcome::UpdateResource(updated) = state
        .execute(Command::UpdateResource(UpdateResource {
            id: created.id,
            new: edit,
        }))
        .await
        .unwrap()
    else {
        panic!("expected UpdateResource");
    };
    assert_eq!(updated.title.as_str(), "Retitled");

    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.status(), Status::Ok);
    assert_eq!(ev.command(), "update_resource");
    assert_eq!(
        ev.get("resource.id"),
        Some(&FieldValue::from(created.id.to_string()))
    );
    assert_eq!(ev.get("resource.kind"), Some(&FieldValue::from("book")));
    assert_eq!(ev.get("resource.title"), Some(&FieldValue::from("Retitled")));
    assert_eq!(
        ev.get("resource.changed_fields"),
        Some(&FieldValue::from("title"))
    );
}

#[tokio::test]
async fn update_creators_replaces_list_and_renumbers_order() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let author_id = seed_author(&state).await;
    let name = Name::try_new(None::<&str>, Some("Chadwick"), None::<&str>).unwrap();
    let Outcome::CreatePerson(editor) = state
        .execute(Command::CreatePerson(CreatePerson {
            name,
            aliases: vec![],
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreatePerson");
    };

    let Outcome::CreateResource(created) = state
        .execute(Command::CreateResource(CreateResource {
            new: book_new(author_id, "Confessions"),
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };
    let _ = sink.take();

    let mut edit = created.to_new();
    edit.creators = vec![
        CreatorSpec {
            person_id: editor.id,
            role: CreatorRole::Editor,
            credited_as: None,
        },
        CreatorSpec {
            person_id: author_id,
            role: CreatorRole::Author,
            credited_as: None,
        },
    ];
    let Outcome::UpdateResource(updated) = state
        .execute(Command::UpdateResource(UpdateResource {
            id: created.id,
            new: edit,
        }))
        .await
        .unwrap()
    else {
        panic!("expected UpdateResource");
    };
    assert_eq!(updated.creators.len(), 2);
    assert_eq!(updated.creators[0].person_id, editor.id);
    assert_eq!(updated.creators[0].order, 0);
    assert_eq!(updated.creators[1].person_id, author_id);
    assert_eq!(updated.creators[1].order, 1);

    let Outcome::GetResource(got) = state
        .execute(Command::GetResource(GetResource { id: created.id }))
        .await
        .unwrap()
    else {
        panic!("expected GetResource");
    };
    assert_eq!(got.creators.len(), 2);
    assert_eq!(got.creators[0].person_id, editor.id);

    let events = sink.take();
    assert_eq!(
        events[0].get("resource.changed_fields"),
        Some(&FieldValue::from("creators"))
    );
}
