//! Storage port. Adapters implement this; commands call it through `AppState`.

use crate::{
    Name, NewResource, NewSeries, NonEmptyStr, Person, PersonError, PersonId, Resource,
    ResourceError, ResourceId, ResourcePage, ResourceQuery, ResourceSummary, Series, SeriesId,
    SortedResourcePage, Tag, TagId,
};

/// Storage backend. Grows one method per capability a command needs.
#[async_trait::async_trait]
pub trait Store: Send + Sync + 'static {
    /// Cheap liveness probe. Must not mutate state.
    async fn ping(&self) -> Result<StoreHealth, StoreError>;

    /// Insert a new person. Store allocates [`PersonId`], then [`Person::try_new`].
    async fn create_person(&self, name: Name, aliases: Vec<Name>) -> Result<Person, StoreError>;

    /// Fetch by id. Missing → [`StoreError::not_found`].
    async fn get_person(&self, id: PersonId) -> Result<Person, StoreError>;

    /// Replace primary name and aliases in one write. Missing → not-found.
    async fn update_person(
        &self,
        id: PersonId,
        name: Name,
        aliases: Vec<Name>,
    ) -> Result<Person, StoreError>;

    /// Remove by id. Missing → not-found.
    async fn delete_person(&self, id: PersonId) -> Result<(), StoreError>;

    /// Non-unique lookup. Matches primary or any alias via [`Name::key`].
    /// Empty vec when none match.
    async fn find_persons_by_name(&self, name: &Name) -> Result<Vec<Person>, StoreError>;

    /// All people in the library. Empty vec when none.
    async fn list_persons(&self) -> Result<Vec<Person>, StoreError>;

    /// Allocate [`SeriesId`], [`Series::try_new`], insert series row.
    async fn create_series(&self, new: NewSeries) -> Result<Series, StoreError>;

    /// Fetch series by id. Missing → [`StoreError::not_found`].
    async fn get_series(&self, id: SeriesId) -> Result<Series, StoreError>;

    /// Allocate [`TagId`], insert tag row. Text must be unique (exact string).
    /// Duplicate text → [`StoreError::invalid`].
    async fn create_tag(&self, text: NonEmptyStr) -> Result<Tag, StoreError>;

    /// Fetch tag by id. Missing → [`StoreError::not_found`].
    async fn get_tag(&self, id: TagId) -> Result<Tag, StoreError>;

    /// Lookup by exact stored text. `None` when no row matches.
    async fn find_tag_by_text(&self, text: &NonEmptyStr) -> Result<Option<Tag>, StoreError>;

    /// Every tag, ordered by text ascending. Empty vec when none.
    async fn list_tags(&self) -> Result<Vec<Tag>, StoreError>;

    /// Allocate id, [`Resource::try_new`], persist resource row **without** creators,
    /// then create one `created` edge per creator (order = [`Creator::order`](crate::Creator::order)).
    ///
    /// Does **not** load/validate parent kind or series existence — caller
    /// (`CreateResource`) does that before invoke.
    async fn create_resource(&self, new: NewResource) -> Result<Resource, StoreError>;

    /// Replace every stored field of `id` with `new`: the row's content plus
    /// its whole creator list. Missing → [`StoreError::not_found`].
    ///
    /// Rebuilds via [`Resource::try_new`] so the replacement passes the same
    /// shape validation as create; creators are replaced wholesale, with
    /// `order` re-assigned 0..n from `new.creators` position (stale edges are
    /// gone afterwards). Like [`Store::create_resource`], does **not** check
    /// parent kind or series existence — the caller (`UpdateResource`) does
    /// that before invoke.
    async fn update_resource(
        &self,
        id: ResourceId,
        new: NewResource,
    ) -> Result<Resource, StoreError>;

    /// Flip only the favorite flag on `id`. Missing → [`StoreError::not_found`].
    ///
    /// Leaves every bibliographic field alone and does not restamp
    /// `date_edited`: starring is a library bookmark, not a content edit.
    async fn set_resource_favorite(
        &self,
        id: ResourceId,
        favorite: bool,
    ) -> Result<Resource, StoreError>;

    /// Load resource row + reconstruct `creators` from edges ordered by `order`.
    /// Missing resource → [`StoreError::not_found`].
    async fn get_resource(&self, id: ResourceId) -> Result<Resource, StoreError>;

    /// All resources as summary rows. Empty vec when none.
    ///
    /// Newest first by [`ResourceId`] ULID (descending). Labels and
    /// `container_title` are read-time projections from edges / parent links.
    async fn list_resources(&self) -> Result<Vec<ResourceSummary>, StoreError>;

    /// Newest-first page of summary rows, starting strictly below `before`
    /// (`None` starts at the newest row).
    ///
    /// `limit` bounds the page size. Implementations fetch one extra row so
    /// [`ResourcePage::next_before`] is exact and callers never land on an
    /// empty trailing page.
    async fn list_resources_page(
        &self,
        before: Option<ResourceId>,
        limit: u32,
    ) -> Result<ResourcePage, StoreError>;

    /// One page of summary rows for `query`: the database query owns the
    /// ordering, shells only walk offsets.
    ///
    /// Order contract, shared by every adapter:
    /// - `query.sort: None` orders newest-first by id (descending);
    /// - title rows order case-insensitively;
    /// - primary-creator rows order by the first creator's family name;
    /// - published rows order by the chronological instant;
    /// - publisher rows order case-insensitively over the name;
    /// - dateAdded / dateEdited rows order by their chronological
    ///   instants, which every stored record carries;
    /// - rows without the sort field land at the no-data end (last
    ///   ascending, first descending);
    /// - ties keep newest-first (id descending) in both directions.
    ///
    /// Filter contract, shared by every adapter:
    /// - `filter.title` holds when the title contains the needle,
    ///   comparing case-insensitively;
    /// - `filter.search` holds when title OR publisher contains the needle,
    ///   comparing case-insensitively; missing publishers never match;
    /// - `filter.kind` holds when the row's kind equals it;
    /// - `filter.favorite` holds when the row's favorite flag equals it;
    /// - `filter.tag` holds when the row's `tags` array references that tag;
    /// - set predicates AND together: a row must satisfy every one.
    ///
    /// `offset` is a 0-based window into the ordered list; the page returns
    /// `limit` rows and [`SortedResourcePage::next_offset`] — the offset of
    /// the following page — only when more rows may exist. An offset past
    /// the end yields an empty page.
    ///
    /// Backends that cannot order in the query inherit an unsupported error.
    async fn list_resources_query_page(
        &self,
        query: &ResourceQuery,
        offset: u64,
        limit: u32,
    ) -> Result<SortedResourcePage, StoreError> {
        let _ = (query, offset, limit);
        Err(StoreError::new(
            "list_resources_query_page not supported by this store",
        ))
    }

    /// Unfiltered totals across the full library, independent of pagination.
    async fn resource_counts(&self) -> Result<crate::ResourceCounts, StoreError> {
        Err(StoreError::new(
            "resource_counts not supported by this store",
        ))
    }
}

/// Result of [`Store::ping`].
#[derive(Debug, Clone)]
pub struct StoreHealth {
    pub backend: &'static str,
    pub ok: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoreErrorKind {
    Other,
    NotFound,
    Invalid,
}

/// Failure from a store operation.
#[derive(Debug, thiserror::Error)]
#[error("store error: {message}")]
pub struct StoreError {
    pub message: String,
    kind: StoreErrorKind,
}

impl StoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: StoreErrorKind::Other,
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: StoreErrorKind::NotFound,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: StoreErrorKind::Invalid,
        }
    }

    pub fn is_not_found(&self) -> bool {
        self.kind == StoreErrorKind::NotFound
    }

    pub fn is_invalid(&self) -> bool {
        self.kind == StoreErrorKind::Invalid
    }
}

impl From<PersonError> for StoreError {
    fn from(err: PersonError) -> Self {
        StoreError::invalid(err.to_string())
    }
}

impl From<ResourceError> for StoreError {
    fn from(err: ResourceError) -> Self {
        StoreError::invalid(err.to_string())
    }
}
