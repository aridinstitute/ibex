use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use oeuvre_core::{
    creator_label_name, format_creators_label, published_label_from, BibliographicDate, Creator,
    CreatorRole, Name, NameKey, NewResource, NewSeries, NonEmptyStr, Person, PersonId, Resource,
    ResourceId, ResourceKind, ResourceSummary, Series, SeriesId, Store, StoreError, StoreHealth,
};

use crate::event::{EventSink, FinishedEvent};

/// Captures finished events in memory. Clone shares the buffer.
#[derive(Clone, Default)]
pub struct MemorySink {
    events: Arc<Mutex<Vec<FinishedEvent>>>,
}

impl MemorySink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn take(&self) -> Vec<FinishedEvent> {
        std::mem::take(&mut *self.events.lock().expect("MemorySink lock"))
    }
}

impl EventSink for MemorySink {
    fn emit(&self, event: &FinishedEvent) {
        self.events
            .lock()
            .expect("MemorySink lock")
            .push(event.clone());
    }
}

struct PeopleState {
    by_id: HashMap<PersonId, Person>,
    by_name: HashMap<NameKey, HashSet<PersonId>>,
}

impl PeopleState {
    fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_name: HashMap::new(),
        }
    }

    fn reindex_person(&mut self, before: Option<&Person>, after: Option<&Person>) {
        if let Some(before) = before {
            for key in before.name_keys() {
                let empty = match self.by_name.get_mut(&key) {
                    Some(ids) => {
                        ids.remove(&before.id);
                        ids.is_empty()
                    }
                    None => false,
                };
                if empty {
                    self.by_name.remove(&key);
                }
            }
        }
        if let Some(after) = after {
            for key in after.name_keys() {
                self.by_name.entry(key).or_default().insert(after.id);
            }
        }
    }
}

struct ResourceRow {
    kind: ResourceKind,
    title: NonEmptyStr,
    publisher: Option<String>,
    published: Option<BibliographicDate>,
    accessed: Option<BibliographicDate>,
    parent_id: Option<ResourceId>,
    series_id: Option<SeriesId>,
    series_number: Option<NonEmptyStr>,
}

struct CreatedEdge {
    person_id: PersonId,
    resource_id: ResourceId,
    role: CreatorRole,
    order: u32,
    credited_as: Option<NonEmptyStr>,
}

pub struct MemoryStore {
    people: Mutex<PeopleState>,
    series: Mutex<HashMap<SeriesId, Series>>,
    resources: Mutex<HashMap<ResourceId, ResourceRow>>,
    created: Mutex<Vec<CreatedEdge>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            people: Mutex::new(PeopleState::new()),
            series: Mutex::new(HashMap::new()),
            resources: Mutex::new(HashMap::new()),
            created: Mutex::new(Vec::new()),
        }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Store for MemoryStore {
    async fn ping(&self) -> Result<StoreHealth, StoreError> {
        Ok(StoreHealth {
            backend: "memory",
            ok: true,
        })
    }

    async fn create_person(
        &self,
        name: Name,
        aliases: Vec<Name>,
    ) -> Result<Person, StoreError> {
        let person = Person::try_new(PersonId::generate(), name, aliases)?;
        let mut state = self.people.lock().expect("MemoryStore lock");
        state.by_id.insert(person.id, person.clone());
        state.reindex_person(None, Some(&person));
        Ok(person)
    }

    async fn get_person(&self, id: PersonId) -> Result<Person, StoreError> {
        self.people
            .lock()
            .expect("MemoryStore lock")
            .by_id
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("person {id}")))
    }

    async fn update_person(
        &self,
        id: PersonId,
        name: Name,
        aliases: Vec<Name>,
    ) -> Result<Person, StoreError> {
        let mut state = self.people.lock().expect("MemoryStore lock");
        let before = state
            .by_id
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("person {id}")))?;
        let person = Person::try_new(id, name, aliases)?;
        state.by_id.insert(id, person.clone());
        state.reindex_person(Some(&before), Some(&person));
        Ok(person)
    }

    async fn delete_person(&self, id: PersonId) -> Result<(), StoreError> {
        let mut state = self.people.lock().expect("MemoryStore lock");
        let before = state
            .by_id
            .remove(&id)
            .ok_or_else(|| StoreError::not_found(format!("person {id}")))?;
        state.reindex_person(Some(&before), None);
        Ok(())
    }

    async fn find_persons_by_name(&self, name: &Name) -> Result<Vec<Person>, StoreError> {
        let state = self.people.lock().expect("MemoryStore lock");
        let key = name.key();
        let Some(ids) = state.by_name.get(&key) else {
            return Ok(Vec::new());
        };
        Ok(ids
            .iter()
            .filter_map(|id| state.by_id.get(id).cloned())
            .collect())
    }

    async fn list_persons(&self) -> Result<Vec<Person>, StoreError> {
        let state = self.people.lock().expect("MemoryStore lock");
        Ok(state.by_id.values().cloned().collect())
    }

    async fn create_series(&self, new: NewSeries) -> Result<Series, StoreError> {
        let series = Series::try_new(SeriesId::generate(), new);
        self.series
            .lock()
            .expect("MemoryStore lock")
            .insert(series.id, series.clone());
        Ok(series)
    }

    async fn get_series(&self, id: SeriesId) -> Result<Series, StoreError> {
        self.series
            .lock()
            .expect("MemoryStore lock")
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("series {id}")))
    }

    async fn create_resource(&self, new: NewResource) -> Result<Resource, StoreError> {
        let resource = Resource::try_new(ResourceId::generate(), new)?;
        let row = ResourceRow {
            kind: resource.kind,
            title: resource.title.clone(),
            publisher: resource.publisher.clone(),
            published: resource.published.clone(),
            accessed: resource.accessed.clone(),
            parent_id: resource.parent_id,
            series_id: resource.series_id,
            series_number: resource.series_number.clone(),
        };
        self.resources
            .lock()
            .expect("MemoryStore lock")
            .insert(resource.id, row);

        let mut edges = self.created.lock().expect("MemoryStore lock");
        for creator in &resource.creators {
            edges.push(CreatedEdge {
                person_id: creator.person_id,
                resource_id: resource.id,
                role: creator.role,
                order: creator.order,
                credited_as: creator.credited_as.clone(),
            });
        }
        Ok(resource)
    }

    async fn get_resource(&self, id: ResourceId) -> Result<Resource, StoreError> {
        let row = self
            .resources
            .lock()
            .expect("MemoryStore lock")
            .get(&id)
            .map(|r| ResourceRow {
                kind: r.kind,
                title: r.title.clone(),
                publisher: r.publisher.clone(),
                published: r.published.clone(),
                accessed: r.accessed.clone(),
                parent_id: r.parent_id,
                series_id: r.series_id,
                series_number: r.series_number.clone(),
            })
            .ok_or_else(|| StoreError::not_found(format!("resource {id}")))?;

        let mut creators: Vec<Creator> = self
            .created
            .lock()
            .expect("MemoryStore lock")
            .iter()
            .filter(|e| e.resource_id == id)
            .map(|e| Creator {
                person_id: e.person_id,
                role: e.role,
                order: e.order,
                credited_as: e.credited_as.clone(),
            })
            .collect();
        creators.sort_by_key(|c| c.order);

        Resource::from_stored(
            id,
            row.kind,
            row.title,
            creators,
            row.publisher,
            row.published,
            row.accessed,
            row.parent_id,
            row.series_id,
            row.series_number,
        )
        .map_err(StoreError::from)
    }

    async fn list_resources(&self) -> Result<Vec<ResourceSummary>, StoreError> {
        let resources = self.resources.lock().expect("MemoryStore lock");
        let created = self.created.lock().expect("MemoryStore lock");
        let people = self.people.lock().expect("MemoryStore lock");

        let mut ids: Vec<ResourceId> = resources.keys().copied().collect();
        ids.sort_by(|a, b| b.as_ulid().cmp(&a.as_ulid()));

        let mut summaries = Vec::with_capacity(ids.len());
        for id in ids {
            let row = resources
                .get(&id)
                .expect("id taken from resources keys");
            let mut edges: Vec<&CreatedEdge> = created
                .iter()
                .filter(|e| e.resource_id == id)
                .collect();
            edges.sort_by_key(|e| e.order);

            let names: Vec<String> = edges
                .iter()
                .map(|e| {
                    let person = people.by_id.get(&e.person_id).ok_or_else(|| {
                        StoreError::new(format!(
                            "created edge references missing person {}",
                            e.person_id
                        ))
                    })?;
                    Ok(creator_label_name(e.credited_as.as_ref(), &person.name))
                })
                .collect::<Result<Vec<_>, StoreError>>()?;

            let container_title = match row.parent_id {
                Some(parent_id) => resources
                    .get(&parent_id)
                    .map(|parent| parent.title.clone()),
                None => None,
            };

            summaries.push(ResourceSummary {
                id,
                kind: row.kind,
                title: row.title.clone(),
                creators_label: format_creators_label(&names),
                published_label: published_label_from(row.published.as_ref()),
                publisher: row.publisher.clone(),
                container_title,
            });
        }
        Ok(summaries)
    }
}

pub struct StubStore {
    health: Result<StoreHealth, String>,
}

impl StubStore {
    pub fn healthy() -> Self {
        Self {
            health: Ok(StoreHealth {
                backend: "stub",
                ok: true,
            }),
        }
    }

    pub fn failing(msg: &str) -> Self {
        Self {
            health: Err(msg.to_string()),
        }
    }
}

fn unsupported(op: &str) -> StoreError {
    StoreError::new(format!("{op} not supported by StubStore"))
}

#[async_trait::async_trait]
impl Store for StubStore {
    async fn ping(&self) -> Result<StoreHealth, StoreError> {
        match &self.health {
            Ok(h) => Ok(h.clone()),
            Err(msg) => Err(StoreError::new(msg.clone())),
        }
    }

    async fn create_person(
        &self,
        _name: Name,
        _aliases: Vec<Name>,
    ) -> Result<Person, StoreError> {
        Err(unsupported("create_person"))
    }

    async fn get_person(&self, _id: PersonId) -> Result<Person, StoreError> {
        Err(unsupported("get_person"))
    }

    async fn update_person(
        &self,
        _id: PersonId,
        _name: Name,
        _aliases: Vec<Name>,
    ) -> Result<Person, StoreError> {
        Err(unsupported("update_person"))
    }

    async fn delete_person(&self, _id: PersonId) -> Result<(), StoreError> {
        Err(unsupported("delete_person"))
    }

    async fn find_persons_by_name(&self, _name: &Name) -> Result<Vec<Person>, StoreError> {
        Err(unsupported("find_persons_by_name"))
    }

    async fn list_persons(&self) -> Result<Vec<Person>, StoreError> {
        Err(unsupported("list_persons"))
    }

    async fn create_series(&self, _new: NewSeries) -> Result<Series, StoreError> {
        Err(unsupported("create_series"))
    }

    async fn get_series(&self, _id: SeriesId) -> Result<Series, StoreError> {
        Err(unsupported("get_series"))
    }

    async fn create_resource(&self, _new: NewResource) -> Result<Resource, StoreError> {
        Err(unsupported("create_resource"))
    }

    async fn get_resource(&self, _id: ResourceId) -> Result<Resource, StoreError> {
        Err(unsupported("get_resource"))
    }

    async fn list_resources(&self) -> Result<Vec<ResourceSummary>, StoreError> {
        Err(unsupported("list_resources"))
    }
}
