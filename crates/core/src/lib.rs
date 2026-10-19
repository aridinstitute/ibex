//! Oeuvre core domain library.
//!
//! Ports and shared domain types. Application runtime lives in `oeuvre-app`.

mod person;
mod store;

pub use person::{Name, NameError, NonEmptyStr, Person, PersonId};
pub use store::{Store, StoreError, StoreHealth};
