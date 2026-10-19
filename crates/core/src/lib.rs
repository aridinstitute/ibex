//! Oeuvre core domain library.
//!
//! Ports and shared domain types. Application runtime lives in `oeuvre-app`.

mod store;

pub use store::{Store, StoreError, StoreHealth};
