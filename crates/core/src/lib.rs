//! Oeuvre core domain library.
//!
//! Ports and shared domain types. Application runtime lives in `oeuvre-app`.

mod collection;
mod identifier;
mod import;
mod inheritance;
mod pages;
mod person;
mod resource;
mod series;
mod store;
mod tag;

pub use collection::{
    Collection, CollectionId, CollectionTreeNode, build_collection_forest,
    collection_descendant_closure,
};
pub use identifier::{Identifier, deserialize_identifiers};
pub use inheritance::{
    Effective, EffectiveValues, InheritanceError, InheritableField, ValueOrigin,
    INHERITANCE_MAX_DEPTH, fields_inherited_from, gather_ancestors, is_never_inherited,
    never_inherited, resolve_effective,
};
pub use import::{
    codes as import_codes, source_fingerprint, ContainerPreview, CreatorPreview,
    DiagnosticSeverity, EntryChoice, EntryPreview, EntryProposal, EntryProvenance,
    IMPORT_MAX_SOURCE_BYTES, IMPORT_PARSER_VERSION, ImportChoices, ImportConflict,
    ImportDiagnostic, ImportDraft, ImportEntryDraft, ImportEntryOutcomeKind, ImportFormat,
    ImportIssue, ImportPreview, ImportProgress, ImportReport, ImportTempId, ImportedEntry,
    PersonCandidate, PersonChoice, PersonMatchPreview, SeriesPreview, SourceProvenance,
    UnresolvedCreator,
};
pub use pages::{PageBound, PageSpan, Pages, deserialize_pages_opt};
pub use person::{Name, NameError, NameKey, NonEmptyStr, Person, PersonError, PersonId};
pub use resource::{
    BibliographicDate, Creator, CreatorRole, CreatorSpec, DateCertainty, DatePrecision,
    DetailCreators, NewResource, ParentRule, PublishedRange, Resource, ResourceCounts,
    ResourceDetail, ResourceError, ResourceFilter, ResourceId, ResourceKind, ResourceMetadata,
    ResourcePage, ResourceProvenance, ResourceQuery, ResourceSort, ResourceSummary, SortDirection,
    SortedResourcePage, UtcMillis, creator_label_name, detail_creator_groups, format_creators_label,
    format_published_label, format_recorded_date, primary_creator_sort_key, published_label_from,
    utc_year_from_millis,
};
pub use series::{NewSeries, Series, SeriesId};
pub use store::{ImportedResourceWrite, Store, StoreError, StoreHealth};
pub use tag::{Tag, TagId};
