use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use oeuvre_core::{Name, NameKey, Person, PersonId, Store, StoreError, StoreHealth};

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

pub struct MemoryStore {
    people: Mutex<PeopleState>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            people: Mutex::new(PeopleState::new()),
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
}
