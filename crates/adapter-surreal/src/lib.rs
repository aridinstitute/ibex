use std::path::Path;

use oeuvre_core::{
    BibliographicDate, Creator, CreatorRole, Name, NewResource, NewSeries, NonEmptyStr, Person,
    PersonId, Resource, ResourceId, ResourceKind, Series, SeriesId, Store, StoreError, StoreHealth,
};
use serde::{Deserialize, Serialize};
use surrealdb::engine::local::{Db, Mem, SurrealKv};
use surrealdb::sql::Thing;
use surrealdb::Surreal;

const NS: &str = "oeuvre";
const DB: &str = "oeuvre";
const PERSON_TABLE: &str = "person";
const SERIES_TABLE: &str = "series";
const RESOURCE_TABLE: &str = "resource";
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SeriesDoc {
    title: NonEmptyStr,
}

/// Persisted resource row — creators intentionally absent.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResourceDoc {
    kind: ResourceKind,
    title: NonEmptyStr,
    publisher: Option<String>,
    published: Option<BibliographicDate>,
    accessed: Option<BibliographicDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<Thing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    series: Option<Thing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    series_number: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreatedEdgeRow {
    #[serde(rename = "in")]
    person: Thing,
    role: CreatorRole,
    order: u32,
    credited_as: Option<String>,
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

        let _ = self
            .db
            .query("DEFINE TABLE IF NOT EXISTS series SCHEMALESS")
            .await;
        let _ = self
            .db
            .query("DEFINE TABLE IF NOT EXISTS resource SCHEMALESS")
            .await;
        // RELATION may be unsupported on some embedded builds; RELATE still works.
        let _ = self
            .db
            .query(
                "DEFINE TABLE IF NOT EXISTS created TYPE RELATION IN person OUT resource ENFORCED",
            )
            .await;

        Ok(())
    }

    fn person_key(id: PersonId) -> (String, String) {
        (PERSON_TABLE.to_owned(), id.to_string())
    }

    fn series_key(id: SeriesId) -> (String, String) {
        (SERIES_TABLE.to_owned(), id.to_string())
    }

    fn resource_key(id: ResourceId) -> (String, String) {
        (RESOURCE_TABLE.to_owned(), id.to_string())
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

    fn thing_to_resource_id(thing: &Thing) -> Result<ResourceId, StoreError> {
        ResourceId::parse(&thing.id.to_string())
            .map_err(|e| StoreError::new(format!("invalid resource id in store: {e}")))
    }

    fn thing_to_series_id(thing: &Thing) -> Result<SeriesId, StoreError> {
        SeriesId::parse(&thing.id.to_string())
            .map_err(|e| StoreError::new(format!("invalid series id in store: {e}")))
    }

    fn resource_doc_from(resource: &Resource) -> ResourceDoc {
        ResourceDoc {
            kind: resource.kind,
            title: resource.title.clone(),
            publisher: resource.publisher.clone(),
            published: resource.published.clone(),
            accessed: resource.accessed.clone(),
            parent: resource.parent_id.map(|id| {
                Thing::from((RESOURCE_TABLE.to_owned(), id.to_string()))
            }),
            series: resource.series_id.map(|id| {
                Thing::from((SERIES_TABLE.to_owned(), id.to_string()))
            }),
            series_number: resource
                .series_number
                .as_ref()
                .map(|s| s.as_str().to_owned()),
        }
    }

    async fn delete_resource_best_effort(&self, id: ResourceId) {
        let _: Result<Option<ResourceDoc>, _> = self.db.delete(Self::resource_key(id)).await;
        let resource_thing = Thing::from((RESOURCE_TABLE.to_owned(), id.to_string()));
        let _ = self
            .db
            .query("DELETE created WHERE out = $resource")
            .bind(("resource", resource_thing))
            .await;
    }

    async fn load_creators_from_edges(
        &self,
        id: ResourceId,
    ) -> Result<Vec<Creator>, StoreError> {
        let resource_thing = Thing::from((RESOURCE_TABLE.to_owned(), id.to_string()));
        let mut response = self
            .db
            .query(
                "SELECT in, role, order, credited_as FROM created \
                 WHERE out = $resource ORDER BY order",
            )
            .bind(("resource", resource_thing))
            .await
            .map_err(map_err)?;

        let rows: Vec<CreatedEdgeRow> = response.take(0).map_err(map_err)?;
        let mut creators = Vec::with_capacity(rows.len());
        for row in rows {
            let person_id = Self::thing_to_person_id(&row.person)?;
            let credited_as = match row.credited_as {
                Some(s) => Some(
                    NonEmptyStr::try_new(s)
                        .map_err(|e| StoreError::invalid(e.to_string()))?,
                ),
                None => None,
            };
            creators.push(Creator {
                person_id,
                role: row.role,
                order: row.order,
                credited_as,
            });
        }
        Ok(creators)
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

    async fn list_persons(&self) -> Result<Vec<Person>, StoreError> {
        let mut response = self
            .db
            .query("SELECT id FROM person")
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

    async fn create_series(&self, new: NewSeries) -> Result<Series, StoreError> {
        let id = SeriesId::generate();
        let series = Series::try_new(id, new)?;
        let created: Option<SeriesDoc> = self
            .db
            .create(Self::series_key(id))
            .content(SeriesDoc {
                title: series.title.clone(),
            })
            .await
            .map_err(map_err)?;
        if created.is_none() {
            return Err(StoreError::new(format!(
                "create_series returned empty for {id}"
            )));
        }
        Ok(series)
    }

    async fn get_series(&self, id: SeriesId) -> Result<Series, StoreError> {
        let row: Option<SeriesDoc> = self
            .db
            .select(Self::series_key(id))
            .await
            .map_err(map_err)?;
        match row {
            Some(doc) => Ok(Series {
                id,
                title: doc.title,
            }),
            None => Err(StoreError::not_found(format!("series {id}"))),
        }
    }

    async fn create_resource(&self, new: NewResource) -> Result<Resource, StoreError> {
        let id = ResourceId::generate();
        let resource = Resource::try_new(id, new)?;

        let created: Option<ResourceDoc> = self
            .db
            .create(Self::resource_key(id))
            .content(Self::resource_doc_from(&resource))
            .await
            .map_err(map_err)?;
        if created.is_none() {
            return Err(StoreError::new(format!(
                "create_resource returned empty for {id}"
            )));
        }

        for creator in &resource.creators {
            let credited = creator
                .credited_as
                .as_ref()
                .map(|s| s.as_str().to_owned());
            let from = Thing::from((PERSON_TABLE.to_owned(), creator.person_id.to_string()));
            let to = Thing::from((RESOURCE_TABLE.to_owned(), id.to_string()));
            let result = self
                .db
                .query(
                    "RELATE $from -> created -> $to \
                     SET role = $role, order = $order, credited_as = $credited",
                )
                .bind(("from", from))
                .bind(("to", to))
                .bind(("role", creator.role))
                .bind(("order", creator.order))
                .bind(("credited", credited))
                .await;

            if let Err(err) = result {
                self.delete_resource_best_effort(id).await;
                return Err(map_err(err));
            }
        }

        Ok(resource)
    }

    async fn get_resource(&self, id: ResourceId) -> Result<Resource, StoreError> {
        let doc: Option<ResourceDoc> = self
            .db
            .select(Self::resource_key(id))
            .await
            .map_err(map_err)?;
        let doc = doc.ok_or_else(|| StoreError::not_found(format!("resource {id}")))?;
        let creators = self.load_creators_from_edges(id).await?;

        let parent_id = match doc.parent {
            Some(ref thing) => Some(Self::thing_to_resource_id(thing)?),
            None => None,
        };
        let series_id = match doc.series {
            Some(ref thing) => Some(Self::thing_to_series_id(thing)?),
            None => None,
        };
        let series_number = match doc.series_number {
            Some(s) => Some(
                NonEmptyStr::try_new(s).map_err(|e| StoreError::invalid(e.to_string()))?,
            ),
            None => None,
        };

        Resource::from_stored(
            id,
            doc.kind,
            doc.title,
            creators,
            doc.publisher,
            doc.published,
            doc.accessed,
            parent_id,
            series_id,
            series_number,
        )
        .map_err(StoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oeuvre_core::{CreatorRole, CreatorSpec, NewResource, NewSeries, ResourceKind, Store};

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

    #[tokio::test]
    async fn resource_create_get_reconstructs_ordered_creators() {
        let store = SurrealStore::open_memory().await.unwrap();

        let author = store
            .create_person(
                Name::try_new(None::<&str>, Some("Augustine"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();
        let editor = store
            .create_person(
                Name::try_new(Some("Henry"), Some("Chadwick"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();
        let series = store
            .create_series(NewSeries {
                title: NonEmptyStr::try_new("Loeb Classical Library").unwrap(),
            })
            .await
            .unwrap();

        let created = store
            .create_resource(NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new("Confessions").unwrap(),
                creators: vec![
                    CreatorSpec {
                        person_id: author.id,
                        role: CreatorRole::Author,
                        credited_as: None,
                    },
                    CreatorSpec {
                        person_id: editor.id,
                        role: CreatorRole::Editor,
                        credited_as: Some(NonEmptyStr::try_new("ed.").unwrap()),
                    },
                ],
                publisher: Some("Harvard University Press".into()),
                published: None,
                accessed: None,
                parent_id: None,
                series_id: Some(series.id),
                series_number: Some(NonEmptyStr::try_new("LCL 26").unwrap()),
            })
            .await
            .unwrap();

        let loaded = store.get_resource(created.id).await.unwrap();
        assert_eq!(loaded.id, created.id);
        assert_eq!(loaded.title.as_str(), "Confessions");
        assert_eq!(
            loaded.publisher.as_deref(),
            Some("Harvard University Press")
        );
        assert_eq!(loaded.series_id, Some(series.id));
        assert_eq!(
            loaded.series_number.as_ref().map(NonEmptyStr::as_str),
            Some("LCL 26")
        );
        assert_eq!(loaded.creators.len(), 2);
        assert_eq!(loaded.creators[0].person_id, author.id);
        assert_eq!(loaded.creators[0].order, 0);
        assert_eq!(loaded.creators[0].role, CreatorRole::Author);
        assert_eq!(loaded.creators[1].person_id, editor.id);
        assert_eq!(loaded.creators[1].order, 1);
        assert_eq!(
            loaded.creators[1].credited_as.as_ref().map(NonEmptyStr::as_str),
            Some("ed.")
        );
    }

    #[tokio::test]
    async fn resource_doc_select_has_no_creators_field() {
        let store = SurrealStore::open_memory().await.unwrap();
        let author = store
            .create_person(
                Name::try_new(None::<&str>, Some("Augustine"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();
        let created = store
            .create_resource(NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new("Confessions").unwrap(),
                creators: vec![CreatorSpec {
                    person_id: author.id,
                    role: CreatorRole::Author,
                    credited_as: None,
                }],
                publisher: None,
                published: None,
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
            })
            .await
            .unwrap();

        let resource = Thing::from((RESOURCE_TABLE.to_owned(), created.id.to_string()));
        let mut response = store
            .db
            .query("RETURN object::keys((SELECT * FROM ONLY $r))")
            .bind(("r", resource))
            .await
            .unwrap();
        let keys: Vec<String> = response.take(0).unwrap();
        assert!(
            !keys.iter().any(|k| k == "creators"),
            "resource document must not embed creators; keys={keys:?}"
        );
    }
}
