//! Oeuvre core domain library.
//!
//! Ports and shared domain types. Application runtime lives in `oeuvre-app`.

mod person;
mod resource;
mod series;
mod store;

pub use person::{Name, NameError, NameKey, NonEmptyStr, Person, PersonError, PersonId};
pub use resource::{
    creator_label_name, detail_creator_groups, format_creators_label, format_published_label,
    primary_creator_sort_key, published_label_from, BibliographicDate, Creator, CreatorRole,
    CreatorSpec, DetailCreators, NewResource, ParentRule, Resource, ResourceDetail, ResourceError,
    ResourceId, ResourceKind, ResourcePage, ResourceSort, ResourceSummary, SortDirection,
    SortedResourcePage, UtcMillis,
};
pub use series::{NewSeries, Series, SeriesId};
pub use store::{Store, StoreError, StoreHealth};
