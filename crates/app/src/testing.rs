use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use oeuvre_core::{Name, Person, PersonId, Store, StoreError, StoreHealth};

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

pub struct MemoryStore {
    people: Mutex<HashMap<PersonId, Person>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            people: Mutex::new(HashMap::new()),
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

    async fn create_person(&self, name: Name) -> Result<Person, StoreError> {
        let person = Person {
            id: PersonId::generate(),
            name,
        };
        self.people
            .lock()
            .expect("MemoryStore lock")
            .insert(person.id, person.clone());
        Ok(person)
    }

    async fn get_person(&self, id: PersonId) -> Result<Person, StoreError> {
        self.people
            .lock()
            .expect("MemoryStore lock")
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("person {id}")))
    }

    async fn update_person(&self, id: PersonId, name: Name) -> Result<Person, StoreError> {
        let mut people = self.people.lock().expect("MemoryStore lock");
        let person = people
            .get_mut(&id)
            .ok_or_else(|| StoreError::not_found(format!("person {id}")))?;
        person.name = name;
        Ok(person.clone())
    }

    async fn delete_person(&self, id: PersonId) -> Result<(), StoreError> {
        self.people
            .lock()
            .expect("MemoryStore lock")
            .remove(&id)
            .map(|_| ())
            .ok_or_else(|| StoreError::not_found(format!("person {id}")))
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

    async fn create_person(&self, _name: Name) -> Result<Person, StoreError> {
        Err(unsupported("create_person"))
    }

    async fn get_person(&self, _id: PersonId) -> Result<Person, StoreError> {
        Err(unsupported("get_person"))
    }

    async fn update_person(&self, _id: PersonId, _name: Name) -> Result<Person, StoreError> {
        Err(unsupported("update_person"))
    }

    async fn delete_person(&self, _id: PersonId) -> Result<(), StoreError> {
        Err(unsupported("delete_person"))
    }
}
