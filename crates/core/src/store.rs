//! Storage port. Adapters implement this; commands call it through `AppState`.

/// Storage backend. Grows one method per capability a command needs.
#[async_trait::async_trait]
pub trait Store: Send + Sync + 'static {
    /// Cheap liveness probe. Must not mutate state.
    async fn ping(&self) -> Result<StoreHealth, StoreError>;
}

/// Result of [`Store::ping`].
#[derive(Debug, Clone)]
pub struct StoreHealth {
    pub backend: &'static str,
    pub ok: bool,
}

/// Failure from a store operation.
#[derive(Debug, thiserror::Error)]
#[error("store error: {message}")]
pub struct StoreError {
    pub message: String,
}

impl StoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
