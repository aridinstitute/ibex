//! SurrealDB storage adapter for Oeuvre.

use oeuvre_core::{Name, Person, PersonId, Store, StoreError, StoreHealth};

/// SurrealDB-backed store. Client wiring lands in a later change.
pub struct SurrealStore;

impl SurrealStore {
    pub async fn open(_path: &std::path::Path) -> Result<Self, StoreError> {
        Ok(Self)
    }
}

fn unsupported(op: &str) -> StoreError {
    StoreError::new(format!("{op} not supported by SurrealStore yet"))
}

#[async_trait::async_trait]
impl Store for SurrealStore {
    async fn ping(&self) -> Result<StoreHealth, StoreError> {
        Ok(StoreHealth {
            backend: "surrealdb",
            ok: true,
        })
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
