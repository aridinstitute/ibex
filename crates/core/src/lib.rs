//! Ibex core domain library.
//!
//! Ports and shared domain types. Application runtime lives in `ibex-app`.

mod attachment;
mod classify;
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

pub use attachment::{Attachment, AttachmentId};
pub use classify::{
    ClassifyInput, KEYWORD_MAX_CHARS, KEYWORDS_MAX_PER_COLLECTION, keyword_suggestions,
    normalize_keywords, parse_keywords,
};
pub use collection::{
    Collection, CollectionId, CollectionTreeNode, build_collection_forest,
    collection_descendant_closure,
};
pub use identifier::{Identifier, deserialize_identifiers};
pub use import::{
    AssignedTarget, ContainerPreview, CreatorPreview, DiagnosticSeverity, EntryChoice,
    EntryPreview, EntryProposal, EntryProvenance, ExistingIdOrName, IMPORT_MAX_SOURCE_BYTES,
    IMPORT_PARSER_VERSION, ImportAssignmentReport, ImportAssignments, ImportChoices,
    ImportConflict, ImportDiagnostic, ImportDraft, ImportEntryDraft, ImportEntryOutcomeKind,
    ImportFormat, ImportIssue, ImportPreview, ImportProgress, ImportReport, ImportTempId,
    ImportedEntry, PersonCandidate, PersonChoice, PersonMatchPreview, SeriesPreview,
    SourceProvenance, UnresolvedCreator, codes as import_codes, source_fingerprint,
};
pub use inheritance::{
    Effective, EffectiveValues, INHERITANCE_MAX_DEPTH, InheritableField, InheritanceError,
    ValueOrigin, fields_inherited_from, gather_ancestors, is_never_inherited, never_inherited,
    resolve_effective,
};
pub use pages::{PageBound, PageSpan, Pages, deserialize_pages_opt};
pub use person::{Name, NameError, NameKey, NonEmptyStr, Person, PersonError, PersonId};
pub use resource::{
    BibliographicDate, Creator, CreatorRole, CreatorSpec, DateCertainty, DatePrecision,
    DetailCreators, NewResource, ParentRule, PublishedRange, Resource, ResourceCounts,
    ResourceDetail, ResourceError, ResourceFilter, ResourceGridPage, ResourceGridParent,
    ResourceGridRow, ResourceId, ResourceKind, ResourceMetadata, ResourcePage, ResourceProvenance,
    ResourceQuery, ResourceSort, ResourceSummary, SortDirection, SortedResourcePage, UtcMillis,
    creator_label_name, detail_creator_groups, format_creators_label, format_published_label,
    format_recorded_date, primary_creator_sort_key, published_label_from, utc_year_from_millis,
};
pub use series::{NewSeries, Series, SeriesId};
pub use store::{
    CollectionUpdate, ImportedResourceWrite, Store, StoreError, StoreHealth,
    collection_update_edges,
};
pub use tag::{Tag, TagId};
