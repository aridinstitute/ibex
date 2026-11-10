//! Storage port. Adapters implement this; commands call it through `AppState`.

use crate::{
    Collection, CollectionId, CollectionTreeNode, Name, NewResource, NewSeries, NonEmptyStr,
    Person, PersonError, PersonId, Resource, ResourceError, ResourceId, ResourcePage,
    ResourceQuery, ResourceSummary, Series, SeriesId, SortedResourcePage, Tag, TagId,
};

/// One imported resource together with people/series/containers that must be
/// created with it.
///
/// Existing people, series, and resources are referenced by id only and are
/// neither created nor deleted. Callers allocate ids for [`Self::new_people`] /
/// [`Self::new_series`] / [`Self::new_dependencies`] up front so
/// [`Self::resource`] (and nested dependency parents) can point at them.
///
/// [`Self::new_dependencies`] are persisted in order before the main resource
/// (parents before children). On failure, none of the new rows or edges remain.
#[derive(Debug, Clone)]
pub struct ImportedResourceWrite {
    pub new_people: Vec<Person>,
    pub new_series: Option<Series>,
    /// Newly required containers (journal, issue, book, …), parents first.
    pub new_dependencies: Vec<Resource>,
    pub resource: NewResource,
}

/// Atomic patch: omitted scalar fields keep their existing values; parent edits
/// apply together, and any invalid edit leaves the row and every edge unchanged.
#[derive(Debug, Clone)]
pub struct CollectionUpdate {
    pub id: CollectionId,
    pub name: Option<NonEmptyStr>,
    pub keywords: Option<Vec<String>>,
    pub jev_enabled: Option<bool>,
    pub add_parents: Vec<CollectionId>,
    pub remove_parents: Vec<CollectionId>,
}

/// Validate and stage the complete parent graph before any write.
pub fn collection_update_edges(
    update: &CollectionUpdate,
    ids: &std::collections::HashSet<CollectionId>,
    edges: &[(CollectionId, CollectionId)],
) -> Result<Vec<(CollectionId, CollectionId)>, StoreError> {
    if !ids.contains(&update.id) {
        return Err(StoreError::not_found(format!("collection {}", update.id)));
    }
    let mut next = edges.to_vec();
    for parent in update
        .remove_parents
        .iter()
        .chain(update.add_parents.iter())
    {
        if !ids.contains(parent) {
            return Err(StoreError::not_found(format!("collection {parent}")));
        }
    }
    for parent in &update.remove_parents {
        let pos = next
            .iter()
            .position(|edge| *edge == (update.id, *parent))
            .ok_or_else(|| {
                StoreError::not_found(format!("collection parent edge {} -> {parent}", update.id))
            })?;
        next.remove(pos);
    }
    for parent in &update.add_parents {
        if *parent == update.id
            || crate::collection_descendant_closure(update.id, next.iter().copied())
                .contains(parent)
        {
            return Err(StoreError::invalid("collection parent cycle"));
        }
        if next.contains(&(update.id, *parent)) {
            return Err(StoreError::invalid("collection parent edge already exists"));
        }
        next.push((update.id, *parent));
    }
    Ok(next)
}

/// Storage backend. Grows one method per capability a command needs.
#[async_trait::async_trait]
pub trait Store: Send + Sync + 'static {
    /// Create a note linked to an existing resource. Blank content is invalid.
    async fn create_note(
        &self,
        resource_id: crate::ResourceId,
        content: crate::NoteContent,
    ) -> Result<crate::Note, StoreError>;
    /// Fetch one note by id. Missing → [`StoreError::not_found`].
    async fn get_note(&self, id: crate::NoteId) -> Result<crate::Note, StoreError>;
    /// Replace content without changing the note's resource or creation time.
    async fn update_note(
        &self,
        id: crate::NoteId,
        content: crate::NoteContent,
    ) -> Result<crate::Note, StoreError>;
    async fn delete_note(&self, id: crate::NoteId) -> Result<(), StoreError>;
    /// Notes for one existing resource, oldest first with id as a stable tie-breaker.
    async fn list_notes(
        &self,
        resource_id: crate::ResourceId,
    ) -> Result<Vec<crate::Note>, StoreError>;
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

    /// Allocate [`CollectionId`], insert collection row. When `parent` is set,
    /// also create `child -> child_of -> parent`. Missing parent → not-found.
    async fn create_collection(
        &self,
        name: NonEmptyStr,
        parent: Option<CollectionId>,
    ) -> Result<Collection, StoreError>;

    /// Fetch collection by id. Missing → [`StoreError::not_found`].
    async fn get_collection(&self, id: CollectionId) -> Result<Collection, StoreError>;

    /// Replace the display name of `id`. Missing → [`StoreError::not_found`].
    async fn rename_collection(
        &self,
        id: CollectionId,
        name: NonEmptyStr,
    ) -> Result<Collection, StoreError>;

    /// Replace the classification rules of `id`: keyword list and Jev opt-in.
    /// Callers validate keywords first. Missing → [`StoreError::not_found`].
    async fn set_collection_classification(
        &self,
        id: CollectionId,
        keywords: Vec<String>,
        jev_enabled: bool,
    ) -> Result<Collection, StoreError>;

    /// Apply a collection patch in one transaction, including all parent edges.
    /// Missing collections/edges fail without writing; cycles and duplicate edges
    /// are invalid. An omitted field is preserved.
    async fn update_collection(&self, update: CollectionUpdate) -> Result<Collection, StoreError>;

    /// Sidebar forest: roots have no parents; children follow `child_of`.
    /// The same collection may appear under multiple parents.
    async fn list_collection_forest(&self) -> Result<Vec<CollectionTreeNode>, StoreError>;

    /// Add `child -> child_of -> parent`. Self-parent, missing ids, or a
    /// duplicate edge → [`StoreError::invalid`] / not-found.
    async fn add_collection_parent(
        &self,
        child: CollectionId,
        parent: CollectionId,
    ) -> Result<(), StoreError>;

    /// Remove one `child -> child_of -> parent` edge. Missing edge or ids →
    /// not-found. Does not delete either collection row.
    async fn remove_collection_parent(
        &self,
        child: CollectionId,
        parent: CollectionId,
    ) -> Result<(), StoreError>;

    /// Delete `id` with smart cascade: exclusive children are deleted
    /// recursively; shared children only lose the edge to `id`. Never
    /// deletes resources (membership refs unset via schema).
    async fn delete_collection(&self, id: CollectionId) -> Result<(), StoreError>;

    /// `id` plus every transitive child via `child_of` (for listing filters).
    async fn collection_descendant_ids(
        &self,
        id: CollectionId,
    ) -> Result<Vec<CollectionId>, StoreError>;

    /// Allocate id, [`Resource::try_new`], persist resource row **without** creators,
    /// then create one `created` edge per creator (order = [`Creator::order`](crate::Creator::order)).
    ///
    /// Does **not** load/validate parent kind or series existence — caller
    /// (`CreateResource`) does that before invoke.
    async fn create_resource(&self, new: NewResource) -> Result<Resource, StoreError>;

    /// Create attachment row and append its link to the resource in one transaction.
    async fn add_attachment(
        &self,
        resource_id: ResourceId,
        attachment: crate::Attachment,
    ) -> Result<crate::Attachment, StoreError>;

    async fn get_attachment(
        &self,
        id: crate::AttachmentId,
    ) -> Result<crate::Attachment, StoreError>;

    /// Persist one imported resource and the people/series/containers newly
    /// required for it as a single atomic unit. On failure, none of those new
    /// rows or edges remain.
    ///
    /// Shared entities already in the store (referenced but not listed in
    /// [`ImportedResourceWrite::new_people`] / `new_series` /
    /// `new_dependencies`) are left untouched.
    /// Does **not** validate parent kind — the import/command layer does that.
    async fn create_imported_resource(
        &self,
        write: ImportedResourceWrite,
    ) -> Result<Resource, StoreError>;

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

    /// Add tag and collection memberships to `id`, keeping existing ones and
    /// never duplicating an id. Leaves every other field alone and does not
    /// restamp `date_edited`. Callers validate the target ids first.
    /// Missing resource → [`StoreError::not_found`].
    async fn add_resource_labels(
        &self,
        id: ResourceId,
        tag_ids: &[TagId],
        collection_ids: &[CollectionId],
    ) -> Result<(), StoreError> {
        let _ = (id, tag_ids, collection_ids);
        Err(StoreError::new(
            "add_resource_labels not supported by this store",
        ))
    }

    /// Load resource row + reconstruct `creators` from edges ordered by `order`.
    /// Missing resource → [`StoreError::not_found`].
    async fn get_resource(&self, id: ResourceId) -> Result<Resource, StoreError>;

    /// All resources as summary rows. Empty vec when none.
    ///
    /// Newest first by [`ResourceId`] ULID (descending). Labels and
    /// `container_title` are read-time projections from edges / parent links.
    async fn list_resources(&self) -> Result<Vec<ResourceSummary>, StoreError>;

    /// Full resource rows for import identity/container seeding.
    ///
    /// Default walks [`Self::list_resources`] + [`Self::get_resource`] (N+1).
    /// Adapters should override with a batched read. Creators may be empty when
    /// the caller only needs identifiers and container fields.
    async fn list_resources_for_import(&self) -> Result<Vec<Resource>, StoreError> {
        let rows = self.list_resources().await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(self.get_resource(row.id).await?);
        }
        Ok(out)
    }

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
    /// - `filter.collection` holds when the row's `collections` array
    ///   references that collection or any of its `child_of` descendants;
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

    /// Page exact siblings, retaining filtered matches and their ancestors.
    /// Child indicators describe the visible, filtered hierarchy. Ordering and
    /// offset semantics are the same as `list_resources_query_page`.
    async fn list_resource_grid_page(
        &self,
        parent: crate::ResourceGridParent,
        query: &ResourceQuery,
        offset: u64,
        limit: u32,
    ) -> Result<crate::ResourceGridPage, StoreError> {
        let _ = (parent, query, offset, limit);
        Err(StoreError::new(
            "list_resource_grid_page not supported by this store",
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
    InUse,
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

    /// The store is held by another process (an embedded datastore admits
    /// one owner at a time).
    pub fn in_use(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: StoreErrorKind::InUse,
        }
    }

    pub fn is_in_use(&self) -> bool {
        self.kind == StoreErrorKind::InUse
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
