use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use oeuvre_core::{
    creator_label_name, format_creators_label, published_label_from, BibliographicDate, Creator,
    CreatorRole, Name, NewResource, NewSeries, NonEmptyStr, Person, PersonId, Resource, ResourceFilter,
    ResourceId, ResourceKind, ResourcePage, ResourceQuery, ResourceSort, ResourceSummary, Series,
    SeriesId, SortDirection, SortedResourcePage, Store, StoreError, StoreHealth,
};
use serde::{Deserialize, Serialize};
use surrealdb::engine::local::{Db, Mem, SurrealKv};
use surrealdb::opt::Config;
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

#[derive(Debug, Deserialize)]
struct ListResourceRow {
    id: Thing,
    kind: ResourceKind,
    title: NonEmptyStr,
    publisher: Option<String>,
    published: Option<BibliographicDate>,
    #[serde(default)]
    parent: Option<Thing>,
}

#[derive(Debug, Deserialize)]
struct ListCreatorEdgeRow {
    out: Thing,
    order: u32,
    credited_as: Option<String>,
    name: Name,
}

#[derive(Debug, Deserialize)]
struct ContainerTitleRow {
    id: Thing,
    title: NonEmptyStr,
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
        Self::open_memory_with_query_timeout(None).await
    }

    /// Open an in-memory store whose every query the datastore caps at
    /// `query_timeout`. `None` leaves queries unbounded, matching
    /// [`Self::open_memory`].
    ///
    /// The bound is enforced by SurrealDB itself rather than by a caller-side
    /// race, so an expensive predicate fails with a timeout error instead of
    /// running to completion. Benchmarks use it to keep a pathological query
    /// from stalling a run that also measures healthy ones.
    pub async fn open_memory_with_query_timeout(
        query_timeout: Option<Duration>,
    ) -> Result<Self, StoreError> {
        let db = Surreal::new::<Mem>(Config::new().query_timeout(query_timeout))
            .await
            .map_err(map_err)?;
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

        // Sorted pages resolve the first created edge per resource row; the
        // index turns that per-row lookup into a seek instead of a scan.
        let _ = self
            .db
            .query("DEFINE INDEX IF NOT EXISTS created_out ON TABLE created FIELDS out")
            .await;

        // The creator filter walks one person's created edges; the index
        // keeps that lookup a seek instead of a relation scan.
        let _ = self
            .db
            .query("DEFINE INDEX IF NOT EXISTS created_in ON TABLE created FIELDS in")
            .await;

        let _ = self
            .db
            .query("DEFINE TABLE IF NOT EXISTS series SCHEMALESS")
            .await;
        let _ = self
            .db
            .query("DEFINE TABLE IF NOT EXISTS resource SCHEMALESS")
            .await;
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

    /// Project page rows into summary rows: creator labels from the created
    /// edges, container titles from in-page parents plus one follow-up
    /// query for parents outside the page.
    async fn summaries_for_rows(
        &self,
        rows: Vec<ListResourceRow>,
    ) -> Result<Vec<ResourceSummary>, StoreError> {
        let ids: Vec<ResourceId> = rows
            .iter()
            .map(|row| Self::thing_to_resource_id(&row.id))
            .collect::<Result<_, _>>()?;

        // Creator edges for this page's rows only; per-resource order comes
        // from the `order` field, re-sorted after grouping.
        let edge_things: Vec<Thing> = ids
            .iter()
            .map(|id| Thing::from(Self::resource_key(*id)))
            .collect();
        let mut response = self
            .db
            .query(
                "SELECT out, order, credited_as, in.name AS name FROM created \
                 WHERE out IN $ids ORDER BY order",
            )
            .bind(("ids", edge_things))
            .await
            .map_err(map_err)?;
        let edge_rows: Vec<ListCreatorEdgeRow> = response.take(0).map_err(map_err)?;

        let mut names_by_resource: HashMap<ResourceId, Vec<(u32, String)>> = HashMap::new();
        for edge in edge_rows {
            let resource_id = Self::thing_to_resource_id(&edge.out)?;
            let credited_as = match edge.credited_as {
                Some(s) => Some(
                    NonEmptyStr::try_new(s)
                        .map_err(|e| StoreError::invalid(e.to_string()))?,
                ),
                None => None,
            };
            let label = creator_label_name(credited_as.as_ref(), &edge.name);
            names_by_resource
                .entry(resource_id)
                .or_default()
                .push((edge.order, label));
        }
        for names in names_by_resource.values_mut() {
            names.sort_by_key(|(order, _)| *order);
        }

        let mut title_by_id: HashMap<ResourceId, NonEmptyStr> = HashMap::with_capacity(rows.len());
        for (row, id) in rows.iter().zip(&ids) {
            title_by_id.insert(*id, row.title.clone());
        }

        // Parents usually live outside the page (they must exist before their
        // children), so resolve container titles the page itself missed with
        // one follow-up query.
        let mut missing: Vec<ResourceId> = Vec::new();
        for row in &rows {
            if let Some(parent) = &row.parent {
                let parent_id = Self::thing_to_resource_id(parent)?;
                if !title_by_id.contains_key(&parent_id) && !missing.contains(&parent_id) {
                    missing.push(parent_id);
                }
            }
        }
        if !missing.is_empty() {
            let parent_things: Vec<Thing> = missing
                .iter()
                .map(|id| Thing::from(Self::resource_key(*id)))
                .collect();
            let mut response = self
                .db
                .query("SELECT id, title FROM resource WHERE id IN $ids")
                .bind(("ids", parent_things))
                .await
                .map_err(map_err)?;
            let title_rows: Vec<ContainerTitleRow> = response.take(0).map_err(map_err)?;
            for title_row in title_rows {
                title_by_id.insert(
                    Self::thing_to_resource_id(&title_row.id)?,
                    title_row.title,
                );
            }
        }

        let mut summaries = Vec::with_capacity(rows.len());
        for (row, id) in rows.into_iter().zip(ids) {
            let names: Vec<String> = names_by_resource
                .remove(&id)
                .unwrap_or_default()
                .into_iter()
                .map(|(_, name)| name)
                .collect();
            let container_title = match row.parent {
                Some(ref thing) => {
                    let parent_id = Self::thing_to_resource_id(thing)?;
                    title_by_id.get(&parent_id).cloned()
                }
                None => None,
            };
            summaries.push(ResourceSummary {
                id,
                kind: row.kind,
                title: row.title,
                creators_label: format_creators_label(&names),
                published_label: published_label_from(row.published.as_ref()),
                publisher: row.publisher,
                container_title,
            });
        }
        Ok(summaries)
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
        let series = Series::try_new(id, new);
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

        self.db
            .query("BEGIN TRANSACTION")
            .await
            .map_err(map_err)?;

        let persist = async {
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
                let from =
                    Thing::from((PERSON_TABLE.to_owned(), creator.person_id.to_string()));
                let to = Thing::from((RESOURCE_TABLE.to_owned(), id.to_string()));
                self.db
                    .query(
                        "RELATE $from -> created -> $to \
                         SET role = $role, order = $order, credited_as = $credited",
                    )
                    .bind(("from", from))
                    .bind(("to", to))
                    .bind(("role", creator.role))
                    .bind(("order", creator.order))
                    .bind(("credited", credited))
                    .await
                    .map_err(map_err)?;
            }
            Ok(())
        }
        .await;

        match persist {
            Ok(()) => {
                self.db
                    .query("COMMIT TRANSACTION")
                    .await
                    .map_err(map_err)?;
                Ok(resource)
            }
            Err(err) => {
                let _ = self.db.query("CANCEL TRANSACTION").await;
                Err(err)
            }
        }
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

    async fn list_resources(&self) -> Result<Vec<ResourceSummary>, StoreError> {
        // The full list is the first page with an unbounded cursor, so one
        // projection path serves both reads.
        let page = self.list_resources_page(None, u32::MAX).await?;
        Ok(page.rows)
    }

    async fn list_resources_page(
        &self,
        before: Option<ResourceId>,
        limit: u32,
    ) -> Result<ResourcePage, StoreError> {
        // One extra row decides next_before exactly; Surreal LIMIT tops out
        // at u32::MAX, so an unbounded page asks for the maximum.
        let fetch_limit = (u64::from(limit) + 1).min(u64::from(u32::MAX)) as usize;
        let mut response = match before {
            Some(before) => self
                .db
                .query(
                    "SELECT id, kind, title, publisher, published, parent FROM resource \
                     WHERE id < $before ORDER BY id DESC LIMIT $limit",
                )
                .bind(("before", Thing::from(Self::resource_key(before))))
                .bind(("limit", fetch_limit))
                .await,
            None => self
                .db
                .query(
                    "SELECT id, kind, title, publisher, published, parent FROM resource \
                     ORDER BY id DESC LIMIT $limit",
                )
                .bind(("limit", fetch_limit))
                .await,
        }
        .map_err(map_err)?;

        let mut rows: Vec<ListResourceRow> = response.take(0).map_err(map_err)?;
        let has_more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let next_before = if has_more {
            rows.last()
                .map(|row| Self::thing_to_resource_id(&row.id))
                .transpose()?
        } else {
            None
        };

        let rows = self.summaries_for_rows(rows).await?;
        Ok(ResourcePage {
            rows,
            next_before,
        })
    }

    async fn list_resources_query_page(
        &self,
        query: &ResourceQuery,
        offset: u64,
        limit: u32,
    ) -> Result<SortedResourcePage, StoreError> {
        // ORDER BY takes field paths, not expressions, so computed keys are
        // aliased projections the clause can name; ASC/DESC cannot come from
        // a parameter, so the literal is chosen by the enum below, never
        // from input. Ties break newest-first (id descending) and rows
        // without the sort field land at the no-data end for the direction
        // via the boolean pre-key (`sort_missing`).
        let fetch_limit = (u64::from(limit) + 1).min(u64::from(u32::MAX)) as usize;
        // Filter predicates AND into one WHERE ahead of the order contract;
        // values are bound parameters, never interpolated text.
        let where_sql = filter_where(&query.filter);
        let sql = match query.sort {
            // The default listing order, paged by offset instead of cursor.
            None => format!(
                "SELECT id, kind, title, publisher, published, parent FROM resource{where_sql} \
                 ORDER BY id DESC LIMIT $limit START $offset"
            ),
            Some((ResourceSort::Title, direction)) => {
                let dir = dir_literal(direction);
                format!(
                    "SELECT id, kind, title, publisher, published, parent FROM resource{where_sql} \
                     ORDER BY title COLLATE {dir}, id DESC LIMIT $limit START $offset"
                )
            }
            Some((ResourceSort::Published, direction)) => {
                let dir = dir_literal(direction);
                format!(
                    "SELECT id, kind, title, publisher, published, parent, \
                            published.instant AS sort_key, \
                            (IF published THEN false ELSE true END) AS sort_missing \
                     FROM resource{where_sql} \
                     ORDER BY sort_missing {dir}, sort_key {dir}, id DESC \
                     LIMIT $limit START $offset"
                )
            }
            // The primary-creator key needs the first created edge per row:
            // a correlated subquery aliases it, the middle level computes
            // the family-name key over that alias, and the outer level adds
            // the no-data marker — one level per reference so the edge query
            // runs once per row.
            Some((ResourceSort::PrimaryCreator, direction)) => {
                let dir = dir_literal(direction);
                format!(
                    "SELECT id, kind, title, publisher, published, parent, sort_key, \
                            (IF sort_key THEN false ELSE true END) AS sort_missing \
                     FROM ( \
                         SELECT *, \
                             (IF first[0].credited_as THEN \
                                  array::last(string::split(first[0].credited_as, ' ')) \
                              ELSE IF first[0].name.last THEN first[0].name.last \
                              ELSE IF first[0].name.first THEN first[0].name.first \
                              ELSE NONE END) AS sort_key \
                         FROM ( \
                             SELECT *, (SELECT order, credited_as, in.name AS name FROM created \
                                  WHERE out = $parent.id ORDER BY order LIMIT 1) AS first \
                             FROM resource{where_sql} \
                         ) \
                     ) \
                     ORDER BY sort_missing {dir}, sort_key COLLATE {dir}, id DESC \
                     LIMIT $limit START $offset"
                )
            }
        };
        let mut statement = self
            .db
            .query(sql)
            .bind(("limit", fetch_limit))
            .bind(("offset", offset));
        // The needle binds only when its clause is present, and lowercases
        // once here instead of per row inside the query.
        if let Some(needle) = query.filter.title.as_ref() {
            statement = statement.bind(("title_needle", needle.as_str().to_lowercase()));
        }
        // The creator binds as its record id, matching the created edges'
        // `in` links without string building.
        if let Some(creator) = query.filter.creator {
            statement = statement.bind(("creator_id", Thing::from(Self::person_key(creator))));
        }
        if let Some(range) = query.filter.published {
            if let Some(from) = range.from {
                statement = statement.bind(("published_from", from.as_unix_ms()));
            }
            if let Some(to) = range.to {
                statement = statement.bind(("published_to", to.as_unix_ms()));
            }
        }
        let mut response = statement.await.map_err(map_err)?;
        let mut rows: Vec<ListResourceRow> = response.take(0).map_err(map_err)?;
        // One extra row decides next_offset exactly, mirroring the cursor
        // page; an offset past the end yields an empty page.
        let has_more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let next_offset = has_more.then(|| offset.saturating_add(u64::from(limit)));
        let rows = self.summaries_for_rows(rows).await?;
        Ok(SortedResourcePage { rows, next_offset })
    }
}

/// ORDER BY keywords cannot arrive as query parameters, so the direction
/// enum picks the literal; caller input never reaches the query text.
fn dir_literal(direction: SortDirection) -> &'static str {
    match direction {
        SortDirection::Ascending => "ASC",
        SortDirection::Descending => "DESC",
    }
}

/// WHERE fragment for the set filter predicates, empty when none are set.
/// Each fragment binds its own parameters in `list_resources_query_page`.
fn filter_where(filter: &ResourceFilter) -> String {
    let mut clauses: Vec<&str> = Vec::new();
    if filter.title.is_some() {
        clauses.push("string::lowercase(title) CONTAINS $title_needle");
    }
    // Membership rides the created relation: one person's edges name
    // exactly their works, so shared authorship counts.
    if filter.creator.is_some() {
        clauses.push("id IN (SELECT VALUE out FROM created WHERE in = $creator_id)");
    }
    // The window bounds the stored instant. Surreal orders NONE before
    // every value, so a bare `instant < to` would pass undated rows; the
    // published guard keeps the window about dated rows only, matching
    // MemoryStore's undated-never-matches contract. Bounds bind only when
    // their clause is present.
    if let Some(range) = filter.published {
        if range.from.is_some() {
            clauses.push(
                "(IF published THEN published.instant >= $published_from ELSE false END)",
            );
        }
        if range.to.is_some() {
            clauses.push("(IF published THEN published.instant < $published_to ELSE false END)");
        }
    }
    if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oeuvre_core::{
        BibliographicDate, CreatorRole, CreatorSpec, NewResource, NewSeries, ResourceKind,
        ResourceSort, SortDirection, Store, UtcMillis,
    };

    const UNIX_MS_2021_01_01_UTC: i64 = 1_609_459_200_000;

    async fn page_article(
        store: &SurrealStore,
        author: PersonId,
        parent: ResourceId,
        title: &str,
    ) -> Resource {
        store
            .create_resource(NewResource {
                kind: ResourceKind::JournalArticle,
                title: NonEmptyStr::try_new(title).unwrap(),
                creators: vec![CreatorSpec {
                    person_id: author,
                    role: CreatorRole::Author,
                    credited_as: Some(NonEmptyStr::try_new("St. Augustine").unwrap()),
                }],
                publisher: None,
                published: Some(
                    BibliographicDate::try_new(
                        UtcMillis::from_unix_ms(UNIX_MS_2021_01_01_UTC),
                        None,
                    )
                    .unwrap(),
                ),
                accessed: None,
                parent_id: Some(parent),
                series_id: None,
                series_number: None,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn list_resources_page_walks_cursor() {
        let store = SurrealStore::open_memory().await.unwrap();

        let author = store
            .create_person(
                Name::try_new(None::<&str>, Some("Augustine"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();

        // The journal is created early enough that every article ULID is
        // strictly newer, so the first page proves container titles resolve
        // across page boundaries.
        let journal = store
            .create_resource(NewResource {
                kind: ResourceKind::Journal,
                title: NonEmptyStr::try_new("Speculum").unwrap(),
                creators: vec![],
                publisher: None,
                published: None,
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
            })
            .await
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));

        let articles = [
            page_article(&store, author.id, journal.id, "Study 1").await,
            page_article(&store, author.id, journal.id, "Study 2").await,
            page_article(&store, author.id, journal.id, "Study 3").await,
            page_article(&store, author.id, journal.id, "Study 4").await,
        ];

        // Creation order can tie within one ULID millisecond; the cursor
        // contract is id ordering, so derive expectations from the ids.
        let mut expected: Vec<ResourceId> = std::iter::once(journal.id)
            .chain(articles.iter().map(|a| a.id))
            .collect();
        expected.sort_by(|a, b| b.as_ulid().cmp(&a.as_ulid()));

        let mut walked: Vec<ResourceId> = Vec::new();
        let mut before = None;
        loop {
            let page = store.list_resources_page(before, 2).await.unwrap();
            assert!(
                !page.rows.is_empty() || page.next_before.is_none(),
                "cursor must never yield an empty page with rows remaining"
            );
            assert!(page.rows.len() <= 2);
            assert!(page
                .rows
                .iter()
                .all(|row| before.is_none_or(|b| row.id.as_ulid() < b.as_ulid())));
            if let Some(next) = page.next_before {
                assert_eq!(Some(next), page.rows.last().map(|row| row.id));
            }
            walked.extend(page.rows.iter().map(|row| row.id));
            match page.next_before {
                Some(next) => before = Some(next),
                None => break,
            }
        }
        assert_eq!(walked, expected);

        // The first page holds the two newest articles; labels and container
        // titles must resolve even though the journal sits on a later page.
        let first = store.list_resources_page(None, 2).await.unwrap();
        assert_eq!(first.rows[0].id, expected[0]);
        assert_eq!(first.rows[1].id, expected[1]);
        for row in &first.rows {
            assert_eq!(row.kind, ResourceKind::JournalArticle);
            assert_eq!(row.creators_label, "St. Augustine");
            assert_eq!(
                row.container_title.as_ref().map(NonEmptyStr::as_str),
                Some("Speculum")
            );
        }
    }

    #[tokio::test]
    async fn list_resources_page_empty_library() {
        let store = SurrealStore::open_memory().await.unwrap();
        let page = store.list_resources_page(None, 3).await.unwrap();
        assert!(page.rows.is_empty());
        assert!(page.next_before.is_none());
    }

    #[tokio::test]
    async fn sorted_title_pages_walk_case_insensitive_order() {
        let store = SurrealStore::open_memory().await.unwrap();

        let author = store
            .create_person(
                Name::try_new(None::<&str>, Some("Augustine"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();
        let journal = store
            .create_resource(NewResource {
                kind: ResourceKind::Journal,
                title: NonEmptyStr::try_new("Speculum").unwrap(),
                creators: vec![],
                publisher: None,
                published: None,
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
            })
            .await
            .unwrap();

        // Mixed case pins the ordering intent: "Beta" must land between
        // "alpha" and "charlie", not before "alpha" the way a case-sensitive
        // scan would sort it.
        for title in ["alpha waves", "Beta modes", "charlie nodes"] {
            page_article(&store, author.id, journal.id, title).await;
        }

        let first = store
            .list_resources_query_page(
                &sorted_query(ResourceSort::Title, SortDirection::Ascending),
                0,
                2,
            )
            .await
            .unwrap();
        assert_eq!(
            first
                .rows
                .iter()
                .map(|row| row.title.as_str())
                .collect::<Vec<_>>(),
            ["alpha waves", "Beta modes"]
        );
        assert_eq!(first.next_offset, Some(2));
        for row in &first.rows {
            assert_eq!(row.kind, ResourceKind::JournalArticle);
            assert_eq!(row.creators_label, "St. Augustine");
            assert_eq!(
                row.container_title.as_ref().map(NonEmptyStr::as_str),
                Some("Speculum")
            );
        }

        let second = store
            .list_resources_query_page(
                &sorted_query(ResourceSort::Title, SortDirection::Ascending),
                2,
                2,
            )
            .await
            .unwrap();
        assert_eq!(
            second
                .rows
                .iter()
                .map(|row| row.title.as_str())
                .collect::<Vec<_>>(),
            ["charlie nodes", "Speculum"]
        );
        assert_eq!(second.next_offset, None);
    }
    /// A resource with explicit creators (credited strings allowed) and an
    /// explicit published instant, the two levers the sorted-order tests
    /// need; the label is a free-form display string.
    async fn sortable_resource(
        store: &SurrealStore,
        authors: Vec<(PersonId, Option<&str>)>,
        title: &str,
        published: Option<(i64, &str)>,
    ) -> Resource {
        store
            .create_resource(NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new(title).unwrap(),
                creators: authors
                    .into_iter()
                    .map(|(person_id, credited_as)| CreatorSpec {
                        person_id,
                        role: CreatorRole::Author,
                        credited_as: credited_as.map(|c| NonEmptyStr::try_new(c).unwrap()),
                    })
                    .collect(),
                publisher: None,
                published: published.map(|(ms, label)| {
                    BibliographicDate::try_new(
                        UtcMillis::from_unix_ms(ms),
                        Some(NonEmptyStr::try_new(label).unwrap()),
                    )
                    .unwrap()
                }),
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
            })
            .await
            .unwrap()
    }

    async fn sortable_container(store: &SurrealStore, title: &str) -> Resource {
        store
            .create_resource(NewResource {
                kind: ResourceKind::Journal,
                title: NonEmptyStr::try_new(title).unwrap(),
                creators: vec![],
                publisher: None,
                published: None,
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
            })
            .await
            .unwrap()
    }

    fn sorted_titles(page: &oeuvre_core::SortedResourcePage) -> Vec<String> {
        page.rows
            .iter()
            .map(|row| row.title.as_str().to_owned())
            .collect()
    }

    fn sorted_query(field: ResourceSort, direction: SortDirection) -> ResourceQuery {
        ResourceQuery::sorted(field, direction)
    }

    #[tokio::test]
    async fn sorted_primary_creator_keys_on_family_name_from_the_edges() {
        let store = SurrealStore::open_memory().await.unwrap();
        let hopper = store
            .create_person(
                Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();
        let lovelace = store
            .create_person(
                Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();
        let augustine = store
            .create_person(
                Name::try_new(None::<&str>, Some("Augustine"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();

        // Two authors pin "primary": the order key must come from the first
        // creator only (Lovelace, not Hopper).
        let coauthored = sortable_resource(
            &store,
            vec![(lovelace.id, None), (hopper.id, None)],
            "Citation basics",
            None,
        )
        .await;
        let by_hopper = sortable_resource(&store, vec![(hopper.id, None)], "Compiler origins", None).await;
        let by_augustine =
            sortable_resource(&store, vec![(augustine.id, None)], "Confessions", None).await;
        // A credited string keys on its last word, tying the coauthored book
        // on "lovelace" so the newest-first tiebreak shows.
        let credited = sortable_resource(
            &store,
            vec![(lovelace.id, Some("A. Lovelace"))],
            "Annotated algorithms",
            None,
        )
        .await;
        let container = sortable_container(&store, "Speculum").await;

        // Creation can tie within one ULID millisecond, so derive the
        // lovelace pair order from the ids the contract ties on.
        let mut lovelace_pair = [coauthored.id, credited.id];
        lovelace_pair.sort_by_key(|id| std::cmp::Reverse(id.as_ulid()));
        let title_of = |id: &ResourceId| -> String {
            [&coauthored, &by_hopper, &by_augustine, &credited, &container]
                .iter()
                .find(|r| r.id == *id)
                .map(|r| r.title.as_str().to_owned())
                .unwrap_or_default()
        };
        let expected: Vec<String> = [
            "Confessions".to_owned(),
            "Compiler origins".to_owned(),
            title_of(&lovelace_pair[0]),
            title_of(&lovelace_pair[1]),
            "Speculum".to_owned(),
        ]
        .to_vec();

        let first = store
            .list_resources_query_page(
                &sorted_query(ResourceSort::PrimaryCreator, SortDirection::Ascending),
                0,
                3,
            )
            .await
            .unwrap();
        assert_eq!(sorted_titles(&first), expected[..3]);
        assert_eq!(first.next_offset, Some(3));

        let second = store
            .list_resources_query_page(
                &sorted_query(ResourceSort::PrimaryCreator, SortDirection::Ascending),
                3,
                3,
            )
            .await
            .unwrap();
        assert_eq!(sorted_titles(&second), expected[3..]);
        assert_eq!(second.next_offset, None);

        // Labels still resolve in sorted pages, whichever page holds the row.
        let coauthored_row = first
            .rows
            .iter()
            .chain(second.rows.iter())
            .find(|row| row.id == coauthored.id)
            .expect("coauthored row appears on one of the pages");
        assert_eq!(coauthored_row.creators_label, "Ada Lovelace; Grace Hopper");
    }

    #[tokio::test]
    async fn sorted_published_orders_by_instant_not_label() {
        let store = SurrealStore::open_memory().await.unwrap();
        let grace = store
            .create_person(
                Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();

        // Labels are display strings, not chronological keys: "Zzz era"
        // carries the oldest instant and would sort last under string order.
        for (title, label, instant) in [
            ("Modern notes", "1999", 1_900_000_000_000),
            ("Middle era", "B-movie era", 1_500_000_000_000),
            ("Old chronicle", "Zzz era", 1_000_000_000_000),
        ] {
            sortable_resource(&store, vec![(grace.id, None)], title, Some((instant, label))).await;
        }
        // The container has no date, so it lands last in ascending order.
        sortable_container(&store, "Speculum").await;

        let page = store
            .list_resources_query_page(
                &sorted_query(ResourceSort::Published, SortDirection::Ascending),
                0,
                10,
            )
            .await
            .unwrap();
        assert_eq!(
            sorted_titles(&page),
            ["Old chronicle", "Middle era", "Modern notes", "Speculum"]
        );
        assert_eq!(page.next_offset, None);
    }

    #[tokio::test]
    async fn sorted_descending_reverses_keys_and_moves_no_data_rows_first() {
        let store = SurrealStore::open_memory().await.unwrap();
        let hopper = store
            .create_person(
                Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();
        let lovelace = store
            .create_person(
                Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();

        sortable_resource(&store, vec![(lovelace.id, None)], "Citation basics", None).await;
        sortable_resource(&store, vec![(hopper.id, None)], "Compiler origins", None).await;
        // No creators: last ascending, first descending.
        sortable_container(&store, "Speculum").await;

        let page = store
            .list_resources_query_page(
                &sorted_query(ResourceSort::PrimaryCreator, SortDirection::Descending),
                0,
                10,
            )
            .await
            .unwrap();
        assert_eq!(
            sorted_titles(&page),
            ["Speculum", "Citation basics", "Compiler origins"]
        );
        assert_eq!(page.next_offset, None);
    }

    #[tokio::test]
    async fn sorted_pages_at_or_past_the_end_yield_empty_pages() {
        let store = SurrealStore::open_memory().await.unwrap();
        let page = store
            .list_resources_query_page(
                &sorted_query(ResourceSort::Title, SortDirection::Ascending),
                0,
                3,
            )
            .await
            .unwrap();
        assert!(page.rows.is_empty());
        assert_eq!(page.next_offset, None);

        let grace = store
            .create_person(
                Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();
        sortable_resource(&store, vec![(grace.id, None)], "Only book", None).await;
        let past = store
            .list_resources_query_page(
                &sorted_query(ResourceSort::Title, SortDirection::Ascending),
                100,
                3,
            )
            .await
            .unwrap();
        assert!(past.rows.is_empty());
        assert_eq!(past.next_offset, None);
    }

    #[tokio::test]
    async fn unsorted_query_pages_walk_newest_first() {
        let store = SurrealStore::open_memory().await.unwrap();
        let grace = store
            .create_person(
                Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();

        let mut created: Vec<ResourceId> = Vec::new();
        for title in ["alpha waves", "Beta modes", "charlie nodes"] {
            let id = sortable_resource(&store, vec![(grace.id, None)], title, None)
                .await
                .id;
            created.push(id);
        }
        // `sort: None` is the default listing order — newest first by id —
        // and ids settle same-millisecond ties.
        created.sort_by(|a, b| b.as_ulid().cmp(&a.as_ulid()));

        let first = store
            .list_resources_query_page(&ResourceQuery::newest_first(), 0, 2)
            .await
            .unwrap();
        assert_eq!(
            first.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            &created[..2]
        );
        assert_eq!(first.next_offset, Some(2));

        let second = store
            .list_resources_query_page(&ResourceQuery::newest_first(), 2, 2)
            .await
            .unwrap();
        assert_eq!(
            second.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            &created[2..]
        );
        assert_eq!(second.next_offset, None);
    }

    fn titled_query(needle: &str) -> ResourceQuery {
        ResourceQuery {
            sort: None,
            filter: ResourceFilter {
                title: Some(NonEmptyStr::try_new(needle).unwrap()),
                creator: None,
                published: None,
            },
        }
    }

    #[tokio::test]
    async fn title_filter_matches_case_insensitive_substring() {
        let store = SurrealStore::open_memory().await.unwrap();
        let grace = store
            .create_person(
                Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();

        for title in ["alpha waves", "Beta modes", "GAMMA nodes"] {
            sortable_resource(&store, vec![(grace.id, None)], title, None).await;
        }
        sortable_container(&store, "Journal of Nothing").await;

        // "MOD" must reach inside "Beta modes" regardless of case.
        let page = store
            .list_resources_query_page(&titled_query("MOD"), 0, 10)
            .await
            .unwrap();
        assert_eq!(sorted_titles(&page), ["Beta modes"]);
        assert_eq!(page.next_offset, None);

        let none = store
            .list_resources_query_page(&titled_query("zzz-not-there"), 0, 10)
            .await
            .unwrap();
        assert!(none.rows.is_empty());
        assert_eq!(none.next_offset, None);
    }

    #[tokio::test]
    async fn title_filter_keeps_sort_order_within_matches() {
        let store = SurrealStore::open_memory().await.unwrap();
        let grace = store
            .create_person(
                Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();

        for title in ["alpha waves", "Beta modes", "GAMMA nodes"] {
            sortable_resource(&store, vec![(grace.id, None)], title, None).await;
        }

        let query = ResourceQuery {
            sort: Some((ResourceSort::Title, SortDirection::Ascending)),
            filter: ResourceFilter {
                title: Some(NonEmptyStr::try_new("DE").unwrap()),
                creator: None,
                published: None,
            },
        };
        let first = store
            .list_resources_query_page(&query, 0, 1)
            .await
            .unwrap();
        assert_eq!(sorted_titles(&first), ["Beta modes"]);
        assert_eq!(first.next_offset, Some(1));

        let second = store
            .list_resources_query_page(&query, 1, 1)
            .await
            .unwrap();
        assert_eq!(sorted_titles(&second), ["GAMMA nodes"]);
        assert_eq!(second.next_offset, None);
    }


    fn creator_query(person: PersonId) -> ResourceQuery {
        ResourceQuery {
            sort: None,
            filter: ResourceFilter {
                title: None,
                creator: Some(person),
                published: None,
            },
        }
    }

    #[tokio::test]
    async fn creator_filter_matches_only_that_persons_works() {
        let store = SurrealStore::open_memory().await.unwrap();
        let ada = store
            .create_person(
                Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();
        let grace = store
            .create_person(
                Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();

        let alpha = sortable_resource(&store, vec![(ada.id, None)], "Alpha Waves", None)
            .await
            .id;
        sortable_resource(&store, vec![(grace.id, None)], "Beta Modes", None).await;
        let shared = sortable_resource(
            &store,
            vec![(grace.id, None), (ada.id, None)],
            "Shared Authorship",
            None,
        )
        .await
        .id;
        sortable_container(&store, "Journal of Nothing").await;

        // The created edges drive the predicate, so shared authorship
        // counts; creatorless containers stay out and the matches keep
        // the newest-first order, so the expectation follows the ids.
        let mut expected = vec![shared, alpha];
        expected.sort_by(|a, b| b.as_ulid().cmp(&a.as_ulid()));
        let page = store
            .list_resources_query_page(&creator_query(ada.id), 0, 10)
            .await
            .unwrap();
        assert_eq!(
            page.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(page.next_offset, None);

        let empty = store
            .list_resources_query_page(&creator_query(PersonId::generate()), 0, 10)
            .await
            .unwrap();
        assert!(empty.rows.is_empty());
        assert_eq!(empty.next_offset, None);
    }

    #[tokio::test]
    async fn creator_filter_keeps_sort_order_within_matches() {
        let store = SurrealStore::open_memory().await.unwrap();
        let ada = store
            .create_person(
                Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();
        let grace = store
            .create_person(
                Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();

        sortable_resource(&store, vec![(ada.id, None)], "Gamma Nodes", None).await;
        sortable_resource(&store, vec![(grace.id, None)], "Beta Modes", None).await;
        sortable_resource(&store, vec![(ada.id, None)], "Alpha Waves", None).await;

        let query = ResourceQuery {
            sort: Some((ResourceSort::Title, SortDirection::Ascending)),
            filter: ResourceFilter {
                title: None,
                creator: Some(ada.id),
                published: None,
            },
        };
        let first = store
            .list_resources_query_page(&query, 0, 1)
            .await
            .unwrap();
        assert_eq!(sorted_titles(&first), ["Alpha Waves"]);
        assert_eq!(first.next_offset, Some(1));

        let second = store
            .list_resources_query_page(&query, 1, 1)
            .await
            .unwrap();
        assert_eq!(sorted_titles(&second), ["Gamma Nodes"]);
        assert_eq!(second.next_offset, None);
    }

    fn ranged_query(from: Option<i64>, to: Option<i64>) -> ResourceQuery {
        ResourceQuery {
            sort: None,
            filter: ResourceFilter {
                title: None,
                creator: None,
                published: Some(oeuvre_core::PublishedRange {
                    from: from.map(oeuvre_core::UtcMillis::from_unix_ms),
                    to: to.map(oeuvre_core::UtcMillis::from_unix_ms),
                }),
            },
        }
    }

    #[tokio::test]
    async fn published_range_filters_by_instant_not_label() {
        let store = SurrealStore::open_memory().await.unwrap();
        let grace = store
            .create_person(
                Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();

        let mut created: Vec<(ResourceId, i64, &str)> = Vec::new();
        for (title, instant, label) in [
            ("alpha waves", 1_000i64, "1999"),
            ("Beta modes", 2_000, "1900"),
            ("charlie nodes", 3_000, "2000"),
        ] {
            let id = sortable_resource(&store, vec![(grace.id, None)], title, Some((instant, label)))
                .await
                .id;
            created.push((id, instant, title));
        }
        let delta = sortable_resource(&store, vec![(grace.id, None)], "Delta notes", None)
            .await
            .id;

        // Bounded window: only instants between the bounds match, and the
        // labels they claim never enter the decision; the newest-first
        // expectation follows the ids.
        let mut matches: Vec<(ResourceId, i64, &str)> = created
            .iter()
            .filter(|(_, instant, _)| (1_500..3_500).contains(instant))
            .map(|(id, instant, title)| (*id, *instant, *title))
            .collect();
        matches.sort_by(|a, b| b.0.as_ulid().cmp(&a.0.as_ulid()));

        let bounded = store
            .list_resources_query_page(&ranged_query(Some(1_500), Some(3_500)), 0, 10)
            .await
            .unwrap();
        assert_eq!(
            bounded.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            matches.iter().map(|(id, _, _)| *id).collect::<Vec<_>>()
        );
        assert_eq!(bounded.next_offset, None);

        // Open "from": rows before the bound, never the undated one;
        // the expectation follows the ids.
        let mut open_expected: Vec<ResourceId> = created
            .iter()
            .filter(|(_, instant, _)| *instant < 2_500)
            .map(|(id, _, _)| *id)
            .collect();
        open_expected.sort_by(|a, b| b.as_ulid().cmp(&a.as_ulid()));
        let open_from = store
            .list_resources_query_page(&ranged_query(None, Some(2_500)), 0, 10)
            .await
            .unwrap();
        assert_eq!(
            open_from.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            open_expected
        );
        assert_eq!(open_from.next_offset, None);

        // Open "to": only rows from the bound on.
        let open_to = store
            .list_resources_query_page(&ranged_query(Some(2_500), None), 0, 10)
            .await
            .unwrap();
        assert_eq!(sorted_titles(&open_to), ["charlie nodes"]);

        // Both open: the window narrows nothing, undated included.
        let unset = store
            .list_resources_query_page(&ranged_query(None, None), 0, 10)
            .await
            .unwrap();
        assert_eq!(unset.rows.len(), 4);
        assert!(unset.rows.iter().any(|row| row.id == delta));
    }

    #[tokio::test]
    async fn published_range_keeps_sort_order_within_matches() {
        let store = SurrealStore::open_memory().await.unwrap();
        let grace = store
            .create_person(
                Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();

        for (title, instant, label) in [
            ("GAMMA nodes", 3_000i64, "Spring 2001"),
            ("Beta modes", 2_000, "1900"),
            ("alpha waves", 1_000, "1999"),
            ("Delta notes", 4_000, "2002"),
        ] {
            sortable_resource(&store, vec![(grace.id, None)], title, Some((instant, label)))
                .await;
        }

        let query = ResourceQuery {
            sort: Some((ResourceSort::Title, SortDirection::Ascending)),
            filter: ResourceFilter {
                title: None,
                creator: None,
                published: Some(oeuvre_core::PublishedRange {
                    from: Some(oeuvre_core::UtcMillis::from_unix_ms(1_500)),
                    to: Some(oeuvre_core::UtcMillis::from_unix_ms(3_500)),
                }),
            },
        };
        let first = store
            .list_resources_query_page(&query, 0, 1)
            .await
            .unwrap();
        assert_eq!(sorted_titles(&first), ["Beta modes"]);
        assert_eq!(first.next_offset, Some(1));

        let second = store
            .list_resources_query_page(&query, 1, 1)
            .await
            .unwrap();
        assert_eq!(sorted_titles(&second), ["GAMMA nodes"]);
        assert_eq!(second.next_offset, None);
    }

    #[tokio::test]
    async fn filters_and_together() {
        let store = SurrealStore::open_memory().await.unwrap();
        let ada = store
            .create_person(
                Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();
        let grace = store
            .create_person(
                Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap(),
                vec![],
            )
            .await
            .unwrap();

        // Every predicate alone matches several rows; only their
        // intersection names one.
        sortable_resource(&store, vec![(ada.id, None)], "alpha waves", Some((1_000, "1999")))
            .await;
        sortable_resource(&store, vec![(ada.id, None)], "alpha nodes", Some((2_000, "2001")))
            .await;
        sortable_resource(&store, vec![(grace.id, None)], "alpha notes", Some((2_000, "2000")))
            .await;
        sortable_resource(&store, vec![(grace.id, None)], "beta waves", Some((1_000, "1998")))
            .await;

        let query = ResourceQuery {
            sort: None,
            filter: ResourceFilter {
                title: Some(NonEmptyStr::try_new("ALPHA").unwrap()),
                creator: Some(ada.id),
                published: Some(oeuvre_core::PublishedRange {
                    from: Some(oeuvre_core::UtcMillis::from_unix_ms(1_500)),
                    to: Some(oeuvre_core::UtcMillis::from_unix_ms(3_500)),
                }),
            },
        };
        let page = store
            .list_resources_query_page(&query, 0, 10)
            .await
            .unwrap();
        assert_eq!(sorted_titles(&page), ["alpha nodes"]);
        assert_eq!(page.next_offset, None);
    }

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

    #[tokio::test]
    async fn list_resources_batch_summaries() {
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

        let journal = store
            .create_resource(NewResource {
                kind: ResourceKind::Journal,
                title: NonEmptyStr::try_new("Speculum").unwrap(),
                creators: vec![],
                publisher: None,
                published: None,
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
            })
            .await
            .unwrap();

        let article = store
            .create_resource(NewResource {
                kind: ResourceKind::JournalArticle,
                title: NonEmptyStr::try_new("On Grace").unwrap(),
                creators: vec![
                    CreatorSpec {
                        person_id: author.id,
                        role: CreatorRole::Author,
                        credited_as: Some(NonEmptyStr::try_new("St. Augustine").unwrap()),
                    },
                    CreatorSpec {
                        person_id: editor.id,
                        role: CreatorRole::Editor,
                        credited_as: None,
                    },
                ],
                publisher: Some("Medieval Academy".into()),
                published: Some(
                    BibliographicDate::try_new(
                        UtcMillis::from_unix_ms(UNIX_MS_2021_01_01_UTC),
                        None,
                    )
                    .unwrap(),
                ),
                accessed: None,
                parent_id: Some(journal.id),
                series_id: None,
                series_number: None,
            })
            .await
            .unwrap();

        let book = store
            .create_resource(NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new("Confessions").unwrap(),
                creators: vec![CreatorSpec {
                    person_id: author.id,
                    role: CreatorRole::Author,
                    credited_as: None,
                }],
                publisher: Some("Harvard".into()),
                published: Some(
                    BibliographicDate::try_new(
                        UtcMillis::from_unix_ms(0),
                        Some(NonEmptyStr::try_new("397").unwrap()),
                    )
                    .unwrap(),
                ),
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
            })
            .await
            .unwrap();

        let listed = store.list_resources().await.unwrap();
        assert_eq!(listed.len(), 3);
        assert!(listed
            .windows(2)
            .all(|w| w[0].id.as_ulid() > w[1].id.as_ulid()));

        let by_id = |id: ResourceId| {
            listed
                .iter()
                .find(|row| row.id == id)
                .unwrap_or_else(|| panic!("missing summary for {id}"))
        };

        let book_row = by_id(book.id);
        assert_eq!(book_row.creators_label, "Augustine");
        assert_eq!(
            book_row.published_label.as_ref().map(NonEmptyStr::as_str),
            Some("397")
        );

        let article_row = by_id(article.id);
        assert_eq!(article_row.creators_label, "St. Augustine; Henry Chadwick");
        assert_eq!(
            article_row
                .published_label
                .as_ref()
                .map(NonEmptyStr::as_str),
            Some("2021")
        );
        assert_eq!(
            article_row
                .container_title
                .as_ref()
                .map(NonEmptyStr::as_str),
            Some("Speculum")
        );

        let journal_row = by_id(journal.id);
        assert_eq!(journal_row.creators_label, "");
        assert!(journal_row.container_title.is_none());
    }
}
