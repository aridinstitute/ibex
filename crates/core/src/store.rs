//! Storage port. Adapters implement this; commands call it through `AppState`.

use crate::{Name, Person, PersonError, PersonId};

/// Storage backend. Grows one method per capability a command needs.
#[async_trait::async_trait]
pub trait Store: Send + Sync + 'static {
    /// Cheap liveness probe. Must not mutate state.
    async fn ping(&self) -> Result<StoreHealth, StoreError>;

    /// Insert a new person. Store allocates [`PersonId`], then [`Person::try_new`].
    async fn create_person(
        &self,
        name: Name,
        aliases: Vec<Name>,
    ) -> Result<Person, StoreError>;

    /// Fetch by id. Missing → [`StoreError::not_found`].
    async fn get_person(&self, id: PersonId) -> Result<Person, StoreError>;

    /// Replace primary name and aliases in one write. Missing → not-found.
    async fn update_person(
        &self,
        id: PersonId,
        name: Name,
        aliases: Vec<Name>,
    ) -> Result<Person, StoreError>;

    /// Remove by id. Missing → not-found.
    async fn delete_person(&self, id: PersonId) -> Result<(), StoreError>;

    /// Non-unique lookup. Matches primary or any alias via [`Name::key`].
    /// Empty vec when none match.
    async fn find_persons_by_name(&self, name: &Name) -> Result<Vec<Person>, StoreError>;

    /// All people in the library. Empty vec when none.
    async fn list_persons(&self) -> Result<Vec<Person>, StoreError>;
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
    Invalid,
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

    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: StoreErrorKind::Invalid,
        }
    }

    pub fn is_not_found(&self) -> bool {
        self.kind == StoreErrorKind::NotFound
    }

    pub fn is_invalid(&self) -> bool {
        self.kind == StoreErrorKind::Invalid
    }
}

impl From<PersonError> for StoreError {
    fn from(err: PersonError) -> Self {
        StoreError::invalid(err.to_string())
    }
}
