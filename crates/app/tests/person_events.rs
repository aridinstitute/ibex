#![cfg(feature = "testing")]

use oeuvre_app::command::{
    CreatePerson, DeletePerson, FindPersonsByName, GetPerson, UpdatePerson,
};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, CommandError, FieldValue, Outcome, Status, Surface};
use oeuvre_core::{Name, PersonId, Store};

#[tokio::test]
async fn person_crud_round_trip() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let name = Name::try_new(None::<&str>, Some("Augustine"), None::<&str>).unwrap();
    let Outcome::CreatePerson(created) = state
        .execute(Command::CreatePerson(CreatePerson {
            name,
            aliases: vec![],
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreatePerson");
    };
    assert_eq!(created.name.display(), "Augustine");

    let Outcome::GetPerson(got) = state
        .execute(Command::GetPerson(GetPerson { id: created.id }))
        .await
        .unwrap()
    else {
        panic!("expected GetPerson");
    };
    assert_eq!(got.id, created.id);
    assert_eq!(got.name.display(), "Augustine");

    let updated_name =
        Name::try_new(Some("Aurelius"), Some("Augustine"), None::<&str>).unwrap();
    let Outcome::UpdatePerson(updated) = state
        .execute(Command::UpdatePerson(UpdatePerson {
            id: created.id,
            name: updated_name,
            aliases: vec![],
        }))
        .await
        .unwrap()
    else {
        panic!("expected UpdatePerson");
    };
    assert_eq!(updated.name.display(), "Aurelius Augustine");

    let Outcome::DeletePerson(deleted) = state
        .execute(Command::DeletePerson(DeletePerson { id: created.id }))
        .await
        .unwrap()
    else {
        panic!("expected DeletePerson");
    };
    assert_eq!(deleted.id, created.id);

    let err = state
        .execute(Command::GetPerson(GetPerson { id: created.id }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::NotFound(_)));
}

#[tokio::test]
async fn person_not_found_on_missing_id() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let id = PersonId::generate();
    let err = state
        .execute(Command::GetPerson(GetPerson { id }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::NotFound(_)));

    let events = sink.take();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status(), Status::Error);
    assert_eq!(
        events[0].get("error.kind"),
        Some(&FieldValue::from("not_found"))
    );
    assert_eq!(
        events[0].get("person.id"),
        Some(&FieldValue::from(id.to_string()))
    );
}

#[tokio::test]
async fn person_create_emits_exactly_one_ok_event() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let name = Name::try_new(Some("Jane"), Some("Doe"), Some("Jr.")).unwrap();
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
    assert_eq!(person.name.display(), "Jane Doe, Jr.");

    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.command(), "create_person");
    assert_eq!(ev.status(), Status::Ok);
    assert_eq!(
        ev.get("person.id"),
        Some(&FieldValue::from(person.id.to_string()))
    );
    assert_eq!(
        ev.get("person.display"),
        Some(&FieldValue::from("Jane Doe, Jr."))
    );
}

#[tokio::test]
async fn each_person_command_emits_exactly_one_event() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let name = Name::try_new(Some("Jane"), Some("Doe"), None::<&str>).unwrap();
    let Outcome::CreatePerson(created) = state
        .execute(Command::CreatePerson(CreatePerson {
            name,
            aliases: vec![],
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreatePerson");
    };
    assert_eq!(sink.take().len(), 1);

    state
        .execute(Command::GetPerson(GetPerson { id: created.id }))
        .await
        .unwrap();
    assert_eq!(sink.take().len(), 1);

    let updated = Name::try_new(Some("Janet"), Some("Doe"), None::<&str>).unwrap();
    state
        .execute(Command::UpdatePerson(UpdatePerson {
            id: created.id,
            name: updated,
            aliases: vec![],
        }))
        .await
        .unwrap();
    assert_eq!(sink.take().len(), 1);

    state
        .execute(Command::DeletePerson(DeletePerson { id: created.id }))
        .await
        .unwrap();
    assert_eq!(sink.take().len(), 1);
}

#[tokio::test]
async fn alias_round_trip_and_find_by_primary_or_alias() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let primary = Name::try_new(Some("Mark"), Some("Twain"), None::<&str>).unwrap();
    let birth = Name::try_new(Some("Samuel"), Some("Clemens"), None::<&str>).unwrap();

    let Outcome::CreatePerson(person) = state
        .execute(Command::CreatePerson(CreatePerson {
            name: primary.clone(),
            aliases: vec![birth.clone()],
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreatePerson");
    };
    assert_eq!(person.name, primary);
    assert_eq!(person.aliases, vec![birth.clone()]);

    let Outcome::FindPersonsByName(by_primary) = state
        .execute(Command::FindPersonsByName(FindPersonsByName {
            name: primary.clone(),
        }))
        .await
        .unwrap()
    else {
        panic!("expected FindPersonsByName");
    };
    assert_eq!(by_primary.len(), 1);
    assert_eq!(by_primary[0].id, person.id);

    let Outcome::FindPersonsByName(by_alias) = state
        .execute(Command::FindPersonsByName(FindPersonsByName {
            name: birth.clone(),
        }))
        .await
        .unwrap()
    else {
        panic!("expected FindPersonsByName");
    };
    assert_eq!(by_alias.len(), 1);
    assert_eq!(by_alias[0].id, person.id);

    let Outcome::UpdatePerson(promoted) = state
        .execute(Command::UpdatePerson(UpdatePerson {
            id: person.id,
            name: birth.clone(),
            aliases: vec![primary.clone()],
        }))
        .await
        .unwrap()
    else {
        panic!("expected UpdatePerson");
    };
    assert_eq!(promoted.name.display(), "Samuel Clemens");
    assert_eq!(promoted.aliases[0].display(), "Mark Twain");

    assert_eq!(sink.take().len(), 4);
}

#[tokio::test]
async fn find_returns_multiple_people_sharing_a_name() {
    let store = MemoryStore::new();
    let shared = Name::try_new(Some("John"), Some("Smith"), None::<&str>).unwrap();
    let a = store
        .create_person(shared.clone(), vec![])
        .await
        .unwrap();
    let b = store
        .create_person(shared.clone(), vec![])
        .await
        .unwrap();

    let hits = store.find_persons_by_name(&shared).await.unwrap();
    assert_eq!(hits.len(), 2);
    let ids: Vec<_> = hits.iter().map(|p| p.id).collect();
    assert!(ids.contains(&a.id));
    assert!(ids.contains(&b.id));
}

#[tokio::test]
async fn create_rejects_alias_equal_to_primary() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let name = Name::try_new(Some("Mark"), Some("Twain"), None::<&str>).unwrap();
    let err = state
        .execute(Command::CreatePerson(CreatePerson {
            name: name.clone(),
            aliases: vec![name],
        }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Invalid(_)));
    assert_eq!(
        sink.take()[0].get("error.kind"),
        Some(&FieldValue::from("invalid"))
    );
}
