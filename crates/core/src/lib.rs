//! Oeuvre core domain library.
//!
//! Ports and shared domain types. Application runtime lives in `oeuvre-app`.

mod collection;
mod person;
mod resource;
mod series;
mod store;
mod tag;

pub use collection::{
    Collection, CollectionId, CollectionTreeNode, build_collection_forest,
    collection_descendant_closure,
};
pub use person::{Name, NameError, NameKey, NonEmptyStr, Person, PersonError, PersonId};
pub use resource::{
    BibliographicDate, Creator, CreatorRole, CreatorSpec, DetailCreators, NewResource, ParentRule,
    PublishedRange, Resource, ResourceCounts, ResourceDetail, ResourceError, ResourceFilter,
    ResourceId, ResourceKind, ResourceMetadata, ResourcePage, ResourceQuery, ResourceSort,
    ResourceSummary, SortDirection, SortedResourcePage, UtcMillis, creator_label_name,
    detail_creator_groups, format_creators_label, format_published_label, format_recorded_date,
    primary_creator_sort_key, published_label_from, utc_year_from_millis,
};
pub use series::{NewSeries, Series, SeriesId};
pub use store::{Store, StoreError, StoreHealth};
pub use tag::{Tag, TagId};
