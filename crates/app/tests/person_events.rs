#![cfg(feature = "testing")]

use oeuvre_app::command::{CreatePerson, DeletePerson, GetPerson, UpdatePerson};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, CommandError, FieldValue, Outcome, Status, Surface};
use oeuvre_core::{Name, PersonId};

#[tokio::test]
async fn person_crud_round_trip() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let name = Name::try_new(None::<&str>, Some("Augustine"), None::<&str>).unwrap();
    let Outcome::CreatePerson(created) = state
        .execute(Command::CreatePerson(CreatePerson { name }))
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
        .execute(Command::CreatePerson(CreatePerson { name }))
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
        .execute(Command::CreatePerson(CreatePerson { name }))
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
