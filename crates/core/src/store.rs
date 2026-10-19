//! Storage port. Adapters implement this; commands call it through `AppState`.

use crate::{Name, Person, PersonId};

/// Storage backend. Grows one method per capability a command needs.
#[async_trait::async_trait]
pub trait Store: Send + Sync + 'static {
    /// Cheap liveness probe. Must not mutate state.
    async fn ping(&self) -> Result<StoreHealth, StoreError>;

    /// Insert a new person. Store allocates [`PersonId`].
    async fn create_person(&self, name: Name) -> Result<Person, StoreError>;

    /// Fetch by id. Missing → [`StoreError::not_found`].
    async fn get_person(&self, id: PersonId) -> Result<Person, StoreError>;

    /// Replace name for an existing id. Missing → not-found.
    async fn update_person(&self, id: PersonId, name: Name) -> Result<Person, StoreError>;

    /// Remove by id. Missing → not-found.
    async fn delete_person(&self, id: PersonId) -> Result<(), StoreError>;
}

/// Result of [`Store::ping`].
#[derive(Debug, Clone)]
pub struct StoreHealth {
    pub backend: &'static str,
    pub ok: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoreErrorKind {
    Other,
    NotFound,
}

/// Failure from a store operation.
#[derive(Debug, thiserror::Error)]
#[error("store error: {message}")]
pub struct StoreError {
    pub message: String,
    kind: StoreErrorKind,
}

impl StoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: StoreErrorKind::Other,
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: StoreErrorKind::NotFound,
        }
    }

    pub fn is_not_found(&self) -> bool {
        self.kind == StoreErrorKind::NotFound
    }
}
