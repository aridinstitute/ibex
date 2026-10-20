//! Oeuvre core domain library.
//!
//! Ports and shared domain types. Application runtime lives in `oeuvre-app`.

mod person;
mod resource;
mod series;
mod store;

pub use person::{Name, NameError, NameKey, NonEmptyStr, Person, PersonError, PersonId};
pub use resource::{
    creator_label_name, format_creators_label, format_published_label, published_label_from,
    BibliographicDate, Creator, CreatorRole, CreatorSpec, NewResource, ParentRule, Resource,
    ResourceError, ResourceId, ResourceKind, ResourcePage, ResourceSummary, UtcMillis,
};
pub use series::{NewSeries, Series, SeriesId};
pub use store::{Store, StoreError, StoreHealth};
