//! SurrealDB storage adapter for Oeuvre.

use oeuvre_core::{Store, StoreError, StoreHealth};

/// SurrealDB-backed store. Client wiring lands in a later change.
pub struct SurrealStore {
    // Reserved for `surrealdb::Surreal<Any>` once the dependency is added.
}

impl SurrealStore {
    pub async fn open(_path: &std::path::Path) -> Result<Self, StoreError> {
        Ok(Self {})
    }
}

#[async_trait::async_trait]
impl Store for SurrealStore {
    async fn ping(&self) -> Result<StoreHealth, StoreError> {
        Ok(StoreHealth {
            backend: "surrealdb",
            ok: true,
        })
    }
}
