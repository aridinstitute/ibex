use std::path::Path;

use oeuvre_core::{Name, Person, PersonId, Store, StoreError, StoreHealth};
use serde::{Deserialize, Serialize};
use surrealdb::engine::local::{Db, Mem, SurrealKv};
use surrealdb::sql::Thing;
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
    aliases: Vec<Name>,
    name_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PersonIdRow {
    id: Thing,
}

impl SurrealStore {
    pub async fn open(path: &Path) -> Result<Self, StoreError> {
        let db = Surreal::new::<SurrealKv>(path)
            .await
            .map_err(map_err)?;
        db.use_ns(NS).use_db(DB).await.map_err(map_err)?;
        let store = Self { db };
        store.ensure_schema().await?;
        Ok(store)
    }

    pub async fn open_memory() -> Result<Self, StoreError> {
        let db = Surreal::new::<Mem>(()).await.map_err(map_err)?;
        db.use_ns(NS).use_db(DB).await.map_err(map_err)?;
        let store = Self { db };
        store.ensure_schema().await?;
        Ok(store)
    }

    async fn ensure_schema(&self) -> Result<(), StoreError> {
        // Best-effort NONUNIQUE index. Mem engines may reject DEFINE; find still
        // uses CONTAINS over name_keys.
        let _ = self
            .db
            .query(
                "DEFINE INDEX IF NOT EXISTS person_name_keys ON TABLE person FIELDS name_keys",
            )
            .await;
        Ok(())
    }

    fn person_key(id: PersonId) -> (String, String) {
        (PERSON_TABLE.to_owned(), id.to_string())
    }

    fn doc_from_person(person: &Person) -> PersonDoc {
        PersonDoc {
            name: person.name.clone(),
            aliases: person.aliases.clone(),
            name_keys: person
                .name_keys()
                .map(|k| k.as_str().to_owned())
                .collect(),
        }
    }

    fn from_doc(id: PersonId, doc: PersonDoc) -> Result<Person, StoreError> {
        Person::try_new(id, doc.name, doc.aliases).map_err(StoreError::from)
    }

    fn thing_to_person_id(thing: &Thing) -> Result<PersonId, StoreError> {
        PersonId::parse(&thing.id.to_string())
            .map_err(|e| StoreError::new(format!("invalid person id in store: {e}")))
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

    async fn create_person(
        &self,
        name: Name,
        aliases: Vec<Name>,
    ) -> Result<Person, StoreError> {
        let id = PersonId::generate();
        let person = Person::try_new(id, name, aliases)?;
        let key = Self::person_key(id);
        let created: Option<PersonDoc> = self
            .db
            .create(key)
            .content(Self::doc_from_person(&person))
            .await
            .map_err(map_err)?;
        if created.is_none() {
            return Err(StoreError::new(format!(
                "create_person returned empty for {id}"
            )));
        }
        Ok(person)
    }

    async fn get_person(&self, id: PersonId) -> Result<Person, StoreError> {
        let row: Option<PersonDoc> = self
            .db
            .select(Self::person_key(id))
            .await
            .map_err(map_err)?;
        match row {
            Some(doc) => Self::from_doc(id, doc),
            None => Err(StoreError::not_found(format!("person {id}"))),
        }
    }

    async fn update_person(
        &self,
        id: PersonId,
        name: Name,
        aliases: Vec<Name>,
    ) -> Result<Person, StoreError> {
        let _existing = self.get_person(id).await?;
        let person = Person::try_new(id, name, aliases)?;
        let updated: Option<PersonDoc> = self
            .db
            .update(Self::person_key(id))
            .content(Self::doc_from_person(&person))
            .await
            .map_err(map_err)?;
        match updated {
            Some(_) => Ok(person),
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

    async fn find_persons_by_name(&self, name: &Name) -> Result<Vec<Person>, StoreError> {
        let key = name.key().as_str().to_owned();
        let mut response = self
            .db
            .query("SELECT id FROM person WHERE name_keys CONTAINS $key")
            .bind(("key", key))
            .await
            .map_err(map_err)?;
        let rows: Vec<PersonIdRow> = response.take(0).map_err(map_err)?;
        let mut people = Vec::with_capacity(rows.len());
        for row in rows {
            let id = Self::thing_to_person_id(&row.id)?;
            people.push(self.get_person(id).await?);
        }
        Ok(people)
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
        let created = store.create_person(name.clone(), vec![]).await.unwrap();
        assert_eq!(created.name.display(), "Augustine");

        let got = store.get_person(created.id).await.unwrap();
        assert_eq!(got.id, created.id);
        assert_eq!(got.name, created.name);

        let updated_name =
            Name::try_new(Some("Aurelius"), Some("Augustine"), None::<&str>).unwrap();
        let updated = store
            .update_person(created.id, updated_name.clone(), vec![])
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
        let created = store.create_person(name, vec![]).await.unwrap();
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
                Name::try_new(None::<&str>, Some("X"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap_err()
            .is_not_found());
        assert!(store.delete_person(id).await.unwrap_err().is_not_found());
    }

    #[tokio::test]
    async fn create_with_aliases_finds_by_alias_and_primary() {
        let store = SurrealStore::open_memory().await.unwrap();
        let primary = Name::try_new(Some("Mark"), Some("Twain"), None::<&str>).unwrap();
        let alias = Name::try_new(Some("Samuel"), Some("Clemens"), None::<&str>).unwrap();

        let person = store
            .create_person(primary.clone(), vec![alias.clone()])
            .await
            .unwrap();
        assert_eq!(person.aliases, vec![alias.clone()]);

        let by_primary = store.find_persons_by_name(&primary).await.unwrap();
        assert_eq!(by_primary.len(), 1);
        assert_eq!(by_primary[0].id, person.id);

        let by_alias = store.find_persons_by_name(&alias).await.unwrap();
        assert_eq!(by_alias.len(), 1);
        assert_eq!(by_alias[0].id, person.id);
        assert_eq!(by_alias[0].aliases, vec![alias]);
    }

    #[tokio::test]
    async fn find_returns_two_people_with_same_name() {
        let store = SurrealStore::open_memory().await.unwrap();
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
}
