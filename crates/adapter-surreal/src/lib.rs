use std::path::Path;

use oeuvre_core::{Name, Person, PersonId, Store, StoreError, StoreHealth};
use serde::{Deserialize, Serialize};
use surrealdb::engine::local::{Db, Mem, SurrealKv};
use surrealdb::Surreal;

const NS: &str = "oeuvre";
const DB: &str = "oeuvre";
const PERSON_TABLE: &str = "person";

pub struct SurrealStore {
    db: Surreal<Db>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersonDoc {
    name: Name,
}

impl SurrealStore {
    pub async fn open(path: &Path) -> Result<Self, StoreError> {
        let db = Surreal::new::<SurrealKv>(path)
            .await
            .map_err(map_err)?;
        db.use_ns(NS).use_db(DB).await.map_err(map_err)?;
        Ok(Self { db })
    }

    pub async fn open_memory() -> Result<Self, StoreError> {
        let db = Surreal::new::<Mem>(()).await.map_err(map_err)?;
        db.use_ns(NS).use_db(DB).await.map_err(map_err)?;
        Ok(Self { db })
    }

    fn person_key(id: PersonId) -> (String, String) {
        (PERSON_TABLE.to_owned(), id.to_string())
    }
}

fn map_err(err: surrealdb::Error) -> StoreError {
    StoreError::new(err.to_string())
}

#[async_trait::async_trait]
impl Store for SurrealStore {
    async fn ping(&self) -> Result<StoreHealth, StoreError> {
        self.db
            .query("RETURN true")
            .await
            .map_err(map_err)?;
        Ok(StoreHealth {
            backend: "surrealdb",
            ok: true,
        })
    }

    async fn create_person(&self, name: Name) -> Result<Person, StoreError> {
        let id = PersonId::generate();
        let key = Self::person_key(id);
        let created: Option<PersonDoc> = self
            .db
            .create(key)
            .content(PersonDoc { name: name.clone() })
            .await
            .map_err(map_err)?;
        if created.is_none() {
            return Err(StoreError::new(format!(
                "create_person returned empty for {id}"
            )));
        }
        Ok(Person { id, name })
    }

    async fn get_person(&self, id: PersonId) -> Result<Person, StoreError> {
        let row: Option<PersonDoc> = self
            .db
            .select(Self::person_key(id))
            .await
            .map_err(map_err)?;
        match row {
            Some(doc) => Ok(Person {
                id,
                name: doc.name,
            }),
            None => Err(StoreError::not_found(format!("person {id}"))),
        }
    }

    async fn update_person(&self, id: PersonId, name: Name) -> Result<Person, StoreError> {
        let updated: Option<PersonDoc> = self
            .db
            .update(Self::person_key(id))
            .content(PersonDoc { name: name.clone() })
            .await
            .map_err(map_err)?;
        match updated {
            Some(_) => Ok(Person { id, name }),
            None => Err(StoreError::not_found(format!("person {id}"))),
        }
    }

    async fn delete_person(&self, id: PersonId) -> Result<(), StoreError> {
        let deleted: Option<PersonDoc> = self
            .db
            .delete(Self::person_key(id))
            .await
            .map_err(map_err)?;
        match deleted {
            Some(_) => Ok(()),
            None => Err(StoreError::not_found(format!("person {id}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oeuvre_core::Store;

    #[tokio::test]
    async fn person_crud_round_trip_memory() {
        let store = SurrealStore::open_memory().await.unwrap();

        let name = Name::try_new(None::<&str>, Some("Augustine"), None::<&str>).unwrap();
        let created = store.create_person(name.clone()).await.unwrap();
        assert_eq!(created.name.display(), "Augustine");

        let got = store.get_person(created.id).await.unwrap();
        assert_eq!(got.id, created.id);
        assert_eq!(got.name, created.name);

        let updated_name =
            Name::try_new(Some("Aurelius"), Some("Augustine"), None::<&str>).unwrap();
        let updated = store
            .update_person(created.id, updated_name.clone())
            .await
            .unwrap();
        assert_eq!(updated.name, updated_name);

        store.delete_person(created.id).await.unwrap();
        let err = store.get_person(created.id).await.unwrap_err();
        assert!(err.is_not_found());
    }

    #[tokio::test]
    async fn person_crud_round_trip_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = SurrealStore::open(dir.path()).await.unwrap();

        let name = Name::try_new(Some("Jane"), Some("Doe"), Some("Jr.")).unwrap();
        let created = store.create_person(name).await.unwrap();
        assert_eq!(created.name.display(), "Jane Doe, Jr.");

        let got = store.get_person(created.id).await.unwrap();
        assert_eq!(got.id, created.id);

        drop(store);

        let reopened = SurrealStore::open(dir.path()).await.unwrap();
        let got = reopened.get_person(created.id).await.unwrap();
        assert_eq!(got.name.display(), "Jane Doe, Jr.");
    }

    #[tokio::test]
    async fn ping_ok() {
        let store = SurrealStore::open_memory().await.unwrap();
        let health = store.ping().await.unwrap();
        assert!(health.ok);
        assert_eq!(health.backend, "surrealdb");
    }

    #[tokio::test]
    async fn missing_person_is_not_found() {
        let store = SurrealStore::open_memory().await.unwrap();
        let id = PersonId::generate();
        assert!(store.get_person(id).await.unwrap_err().is_not_found());
        assert!(store
            .update_person(
                id,
                Name::try_new(None::<&str>, Some("X"), None::<&str>).unwrap()
            )
            .await
            .unwrap_err()
            .is_not_found());
        assert!(store.delete_person(id).await.unwrap_err().is_not_found());
    }
}
