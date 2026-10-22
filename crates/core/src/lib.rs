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
    format_recorded_date, primary_creator_sort_key, published_label_from, utc_year_from_millis,
    BibliographicDate, Creator, CreatorRole, CreatorSpec, DetailCreators, NewResource, ParentRule,
    PublishedRange, Resource, ResourceDetail, ResourceError, ResourceFilter, ResourceId,
    ResourceKind, ResourcePage, ResourceQuery, ResourceSort, ResourceSummary, SortDirection,
    SortedResourcePage, UtcMillis,
};
pub use series::{NewSeries, Series, SeriesId};
pub use store::{Store, StoreError, StoreHealth};
