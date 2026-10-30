//! Persistent SurrealKV libraries: survival across reopen, directory
//! creation, and single-process ownership.

use ibex_adapter_surreal::SurrealStore;
use ibex_core::{Name, Store};

fn ada() -> Name {
    Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap()
}

#[tokio::test]
async fn data_survives_close_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("database");
    let created = {
        let store = SurrealStore::open(&path).await.unwrap();
        let person = store.create_person(ada(), vec![]).await.unwrap();
        store.close().await.unwrap();
        person
    };
    let reopened = SurrealStore::open(&path).await.unwrap();
    let loaded = reopened.get_person(created.id).await.unwrap();
    assert_eq!(loaded.name.display(), "Ada Lovelace");
}

#[tokio::test]
async fn open_creates_a_missing_database_directory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("database");
    let store = SurrealStore::open(&path).await.unwrap();
    assert!(path.is_dir());
    store.close().await.unwrap();
}

#[tokio::test]
async fn a_second_open_of_a_held_library_fails_clearly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("database");
    let held = SurrealStore::open(&path).await.unwrap();
    let err = match SurrealStore::open(&path).await {
        Ok(_) => panic!("a held library must not open twice"),
        Err(err) => err,
    };
    assert!(err.is_in_use(), "{err}");
    assert_eq!(
        err.message,
        format!(
            "The Ibex library at {} is in use by another Ibex process. \
             Close the other Ibex app or CLI and try again.",
            path.display()
        )
    );
    // The held store keeps working: the refused open must not disturb it.
    held.create_person(ada(), vec![]).await.unwrap();
}
