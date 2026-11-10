use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

use ibex_core::{
    BibliographicDate, Collection, CollectionId, CollectionTreeNode, Creator, CreatorRole,
    ImportedResourceWrite, Name, NameKey, NewResource, NewSeries, NonEmptyStr, Person, PersonId,
    Resource, ResourceCounts, ResourceFilter, ResourceId, ResourceKind, ResourceMetadata,
    ResourcePage, ResourceQuery, ResourceSort, ResourceSummary, Series, SeriesId, SortDirection,
    SortedResourcePage, Store, StoreError, StoreHealth, Tag, TagId, UtcMillis,
    build_collection_forest, collection_descendant_closure, creator_label_name,
    format_creators_label, primary_creator_sort_key, published_label_from, resolve_effective,
};

use crate::event::{EventSink, FinishedEvent};

/// Captures finished events in memory. Clone shares the buffer.
#[derive(Clone, Default)]
pub struct MemorySink {
    events: Arc<Mutex<Vec<FinishedEvent>>>,
}

impl MemorySink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn take(&self) -> Vec<FinishedEvent> {
        std::mem::take(&mut *self.events.lock().expect("MemorySink lock"))
    }
}

impl EventSink for MemorySink {
    fn emit(&self, event: &FinishedEvent) {
        self.events
            .lock()
            .expect("MemorySink lock")
            .push(event.clone());
    }
}

struct PeopleState {
    by_id: HashMap<PersonId, Person>,
    by_name: HashMap<NameKey, HashSet<PersonId>>,
}

impl PeopleState {
    fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_name: HashMap::new(),
        }
    }

    fn reindex_person(&mut self, before: Option<&Person>, after: Option<&Person>) {
        if let Some(before) = before {
            for key in before.name_keys() {
                let empty = match self.by_name.get_mut(&key) {
                    Some(ids) => {
                        ids.remove(&before.id);
                        ids.is_empty()
                    }
                    None => false,
                };
                if empty {
                    self.by_name.remove(&key);
                }
            }
        }
        if let Some(after) = after {
            for key in after.name_keys() {
                self.by_name.entry(key).or_default().insert(after.id);
            }
        }
    }
}

#[derive(Clone)]
struct ResourceRow {
    subtitle: Option<NonEmptyStr>,
    title_addon: Option<NonEmptyStr>,
    short_title: Option<NonEmptyStr>,
    place: Option<String>,
    volume: Option<String>,
    issue: Option<String>,
    pages: Option<ibex_core::Pages>,
    total_pages: Option<String>,
    article_number: Option<String>,
    edition: Option<String>,
    doi: Option<String>,
    url: Option<String>,
    eprint: Option<String>,
    isbns: Vec<ibex_core::Identifier>,
    issns: Vec<ibex_core::Identifier>,
    isrns: Vec<ibex_core::Identifier>,
    original_date: Option<BibliographicDate>,
    event_date: Option<BibliographicDate>,
    language: Option<String>,
    document_type: Option<String>,
    publication_method: Option<String>,
    institution: Option<String>,
    organization: Option<String>,
    school: Option<String>,
    abstract_text: Option<String>,
    keywords: Vec<String>,
    kind: ResourceKind,
    title: NonEmptyStr,
    publisher: Option<String>,
    published: Option<BibliographicDate>,
    accessed: Option<BibliographicDate>,
    parent_id: Option<ResourceId>,
    series_id: Option<SeriesId>,
    series_number: Option<NonEmptyStr>,
    tag_ids: Vec<TagId>,
    collection_ids: Vec<CollectionId>,
    attachment_ids: Vec<ibex_core::AttachmentId>,
    metadata: ResourceMetadata,
    favorite: bool,
    date_added: UtcMillis,
    date_edited: UtcMillis,
}

impl ResourceRow {
    /// Domain resource for inheritance resolution (creators omitted).
    fn to_resource(&self, id: ResourceId) -> Result<Resource, StoreError> {
        let mut resource = Resource::from_stored(
            id,
            self.kind,
            self.title.clone(),
            vec![],
            self.publisher.clone(),
            self.published.clone(),
            self.accessed.clone(),
            self.parent_id,
            self.series_id,
            self.series_number.clone(),
            self.tag_ids.clone(),
            self.collection_ids.clone(),
            self.metadata.clone(),
            self.favorite,
            self.date_added,
            self.date_edited,
        )
        .map_err(StoreError::from)?;
        resource.subtitle = self.subtitle.clone();
        resource.title_addon = self.title_addon.clone();
        resource.short_title = self.short_title.clone();
        resource.place = self.place.clone();
        resource.volume = self.volume.clone();
        resource.issue = self.issue.clone();
        resource.pages = self.pages.clone();
        resource.total_pages = self.total_pages.clone();
        resource.article_number = self.article_number.clone();
        resource.edition = self.edition.clone();
        resource.doi = self.doi.clone();
        resource.url = self.url.clone();
        resource.eprint = self.eprint.clone();
        resource.isbns = self.isbns.clone();
        resource.issns = self.issns.clone();
        resource.isrns = self.isrns.clone();
        resource.original_date = self.original_date.clone();
        resource.event_date = self.event_date.clone();
        resource.language = self.language.clone();
        resource.document_type = self.document_type.clone();
        resource.publication_method = self.publication_method.clone();
        resource.institution = self.institution.clone();
        resource.organization = self.organization.clone();
        resource.school = self.school.clone();
        resource.abstract_text = self.abstract_text.clone();
        resource.keywords = self.keywords.clone();
        resource.attachment_ids = self.attachment_ids.clone();
        Ok(resource)
    }
}

struct CreatedEdge {
    person_id: PersonId,
    resource_id: ResourceId,
    role: CreatorRole,
    order: u32,
    credited_as: Option<NonEmptyStr>,
}

/// Key one resource sorts by, `None` inside marking the field absent.
enum SortKey {
    Text(Option<String>),
    Instant(Option<i64>),
}

impl SortKey {
    /// Key order only: rows without the field land at the no-data end (last
    /// ascending, first descending). Ties are the caller's to break.
    fn cmp(&self, other: &Self, direction: SortDirection) -> Ordering {
        let ord = match (self, other) {
            (SortKey::Text(a), SortKey::Text(b)) => cmp_field(a.as_deref(), b.as_deref()),
            (SortKey::Instant(a), SortKey::Instant(b)) => cmp_field(*a, *b),
            // One listing keys every row with the same variant.
            _ => unreachable!("sort keys share one variant per listing"),
        };
        match direction {
            SortDirection::Ascending => ord,
            SortDirection::Descending => ord.reverse(),
        }
    }
}

/// Field order with absent values last: `None` is missing data, not smallest.
fn cmp_field<T: Ord>(a: Option<T>, b: Option<T>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.cmp(&b),
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
    }
}

/// In-memory store. Cheap to clone: all state is behind shared mutexes.
#[derive(Clone)]
pub struct MemoryStore {
    people: Arc<Mutex<PeopleState>>,
    series: Arc<Mutex<HashMap<SeriesId, Series>>>,
    tags: Arc<Mutex<HashMap<TagId, Tag>>>,
    collections: Arc<Mutex<HashMap<CollectionId, Collection>>>,
    /// `(child, parent)` edges for `child_of`.
    collection_child_of: Arc<Mutex<Vec<(CollectionId, CollectionId)>>>,
    resources: Arc<Mutex<HashMap<ResourceId, ResourceRow>>>,
    attachments: Arc<Mutex<HashMap<ibex_core::AttachmentId, ibex_core::Attachment>>>,
    notes: Arc<Mutex<HashMap<ibex_core::NoteId, ibex_core::Note>>>,
    created: Arc<Mutex<Vec<CreatedEdge>>>,
    /// When set, the Nth subsequent `create_imported_resource` fails (0-based).
    import_fail_at: Arc<Mutex<Option<usize>>>,
    import_create_count: Arc<Mutex<usize>>,
    /// When set, the Nth subsequent `add_resource_labels` fails (0-based).
    label_fail_at: Arc<Mutex<Option<usize>>>,
    label_write_count: Arc<Mutex<usize>>,
    get_resource_calls: Arc<AtomicUsize>,
    find_persons_by_name_calls: Arc<AtomicUsize>,
    list_persons_calls: Arc<AtomicUsize>,
    list_resources_for_import_calls: Arc<AtomicUsize>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            people: Arc::new(Mutex::new(PeopleState::new())),
            series: Arc::new(Mutex::new(HashMap::new())),
            tags: Arc::new(Mutex::new(HashMap::new())),
            collections: Arc::new(Mutex::new(HashMap::new())),
            collection_child_of: Arc::new(Mutex::new(Vec::new())),
            resources: Arc::new(Mutex::new(HashMap::new())),
            attachments: Arc::new(Mutex::new(HashMap::new())),
            notes: Arc::new(Mutex::new(HashMap::new())),
            created: Arc::new(Mutex::new(Vec::new())),
            import_fail_at: Arc::new(Mutex::new(None)),
            import_create_count: Arc::new(Mutex::new(0)),
            label_fail_at: Arc::new(Mutex::new(None)),
            label_write_count: Arc::new(Mutex::new(0)),
            get_resource_calls: Arc::new(AtomicUsize::new(0)),
            find_persons_by_name_calls: Arc::new(AtomicUsize::new(0)),
            list_persons_calls: Arc::new(AtomicUsize::new(0)),
            list_resources_for_import_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn reset_query_counts(&self) {
        self.get_resource_calls.store(0, AtomicOrdering::SeqCst);
        self.find_persons_by_name_calls
            .store(0, AtomicOrdering::SeqCst);
        self.list_persons_calls.store(0, AtomicOrdering::SeqCst);
        self.list_resources_for_import_calls
            .store(0, AtomicOrdering::SeqCst);
    }

    pub fn get_resource_count(&self) -> usize {
        self.get_resource_calls.load(AtomicOrdering::SeqCst)
    }

    pub fn find_persons_by_name_count(&self) -> usize {
        self.find_persons_by_name_calls.load(AtomicOrdering::SeqCst)
    }

    pub fn list_persons_count(&self) -> usize {
        self.list_persons_calls.load(AtomicOrdering::SeqCst)
    }

    pub fn list_resources_for_import_count(&self) -> usize {
        self.list_resources_for_import_calls
            .load(AtomicOrdering::SeqCst)
    }

    /// Fail the Nth `create_imported_resource` call (0-based) with a store error.
    pub fn fail_imported_create_at(&self, n: usize) {
        *self.import_fail_at.lock().expect("MemoryStore lock") = Some(n);
        *self.import_create_count.lock().expect("MemoryStore lock") = 0;
    }

    /// Fail the Nth `add_resource_labels` call (0-based) with a store error.
    pub fn fail_add_resource_labels_at(&self, n: usize) {
        *self.label_fail_at.lock().expect("MemoryStore lock") = Some(n);
        *self.label_write_count.lock().expect("MemoryStore lock") = 0;
    }

    /// Clear fault injection so subsequent imports persist normally.
    pub fn clear_import_fail(&self) {
        *self.import_fail_at.lock().expect("MemoryStore lock") = None;
        *self.import_create_count.lock().expect("MemoryStore lock") = 0;
        *self.label_fail_at.lock().expect("MemoryStore lock") = None;
        *self.label_write_count.lock().expect("MemoryStore lock") = 0;
    }

    /// Test helper: remove a resource and its creator edges (Task 06 stale preview).
    pub fn test_delete_resource(&self, id: ResourceId) {
        self.resources.lock().expect("MemoryStore lock").remove(&id);
        self.created
            .lock()
            .expect("MemoryStore lock")
            .retain(|e| e.resource_id != id);
    }

    /// Smart cascade: exclusive children recurse; shared children only unlink.
    /// Membership refs on resources are cleared; resource rows stay.
    fn delete_collection_recursive(&self, id: CollectionId) {
        let mut edges = self.collection_child_of.lock().expect("MemoryStore lock");
        let children: Vec<CollectionId> = edges
            .iter()
            .filter(|(_, parent)| *parent == id)
            .map(|(child, _)| *child)
            .collect();
        for child in children {
            let parent_count = edges.iter().filter(|(c, _)| *c == child).count();
            if parent_count <= 1 {
                drop(edges);
                self.delete_collection_recursive(child);
                edges = self.collection_child_of.lock().expect("MemoryStore lock");
            } else {
                edges.retain(|(c, p)| !(*c == child && *p == id));
            }
        }
        edges.retain(|(c, p)| *c != id && *p != id);
        drop(edges);
        self.collections
            .lock()
            .expect("MemoryStore lock")
            .remove(&id);
        let mut resources = self.resources.lock().expect("MemoryStore lock");
        for row in resources.values_mut() {
            row.collection_ids.retain(|cid| *cid != id);
        }
    }

    /// Build summary rows for `ids` in the given order, projecting creator
    /// labels and container titles from the in-memory maps.
    fn summaries_in_order(&self, ids: &[ResourceId]) -> Result<Vec<ResourceSummary>, StoreError> {
        let resources = self.resources.lock().expect("MemoryStore lock");
        let created = self.created.lock().expect("MemoryStore lock");
        let people = self.people.lock().expect("MemoryStore lock");

        let mut summaries = Vec::with_capacity(ids.len());
        for id in ids {
            let row = resources
                .get(id)
                .expect("ids come from the resources map keys");
            let mut edges: Vec<&CreatedEdge> =
                created.iter().filter(|e| e.resource_id == *id).collect();
            edges.sort_by_key(|e| e.order);

            let names: Vec<String> = edges
                .iter()
                .map(|e| {
                    let person = people.by_id.get(&e.person_id).ok_or_else(|| {
                        StoreError::new(format!(
                            "created edge references missing person {}",
                            e.person_id
                        ))
                    })?;
                    Ok(creator_label_name(e.credited_as.as_ref(), &person.name))
                })
                .collect::<Result<Vec<_>, StoreError>>()?;

            let child = row.to_resource(*id)?;
            let mut ancestor_buf: Vec<Resource> = Vec::new();
            let mut next = child.parent_id;
            while let Some(parent_id) = next {
                if ancestor_buf.len() >= ibex_core::INHERITANCE_MAX_DEPTH {
                    return Err(StoreError::invalid("inheritance depth exceeded"));
                }
                let Some(parent_row) = resources.get(&parent_id) else {
                    return Err(StoreError::invalid(format!(
                        "expected parent {parent_id} missing"
                    )));
                };
                let parent = parent_row.to_resource(parent_id)?;
                next = parent.parent_id;
                ancestor_buf.push(parent);
            }
            let effective = resolve_effective(&child, &ancestor_buf)
                .map_err(|e| StoreError::invalid(e.to_string()))?;

            summaries.push(ResourceSummary {
                id: *id,
                kind: row.kind,
                title: row.title.clone(),
                creators_label: format_creators_label(&names),
                published_label: published_label_from(effective.published.value.as_ref()),
                publisher: effective.publisher.value,
                container_title: effective.container_title.value,
                date_added: row.date_added,
                date_edited: row.date_edited,
            });
        }
        Ok(summaries)
    }

    /// Ids of every resource in `sort` order, per the port contract: key
    /// order with no-data rows at the direction's end, ties newest-first.
    fn sorted_ids(
        &self,
        sort: ResourceSort,
        direction: SortDirection,
    ) -> Result<Vec<ResourceId>, StoreError> {
        let resources = self.resources.lock().expect("MemoryStore lock");
        let mut keyed: Vec<(ResourceId, SortKey)> = Vec::with_capacity(resources.len());
        match sort {
            ResourceSort::Title => {
                for (id, row) in resources.iter() {
                    keyed.push((*id, SortKey::Text(Some(row.title.as_str().to_lowercase()))));
                }
            }
            // The primary creator is the first edge in bibliography order;
            // the key mirrors what the SurrealQL query computes, so both
            // adapters order a library identically.
            ResourceSort::PrimaryCreator => {
                let created = self.created.lock().expect("MemoryStore lock");
                let people = self.people.lock().expect("MemoryStore lock");
                for id in resources.keys() {
                    let key = match created
                        .iter()
                        .filter(|edge| edge.resource_id == *id)
                        .min_by_key(|edge| edge.order)
                    {
                        Some(edge) => {
                            let person = people.by_id.get(&edge.person_id).ok_or_else(|| {
                                StoreError::new(format!(
                                    "created edge references missing person {}",
                                    edge.person_id
                                ))
                            })?;
                            SortKey::Text(Some(primary_creator_sort_key(
                                edge.credited_as.as_ref(),
                                &person.name,
                            )))
                        }
                        None => SortKey::Text(None),
                    };
                    keyed.push((*id, key));
                }
            }
            // Chronological instants, never the display labels: "Summer 397"
            // has no string order.
            ResourceSort::Published => {
                for (id, row) in resources.iter() {
                    keyed.push((
                        *id,
                        SortKey::Instant(
                            row.published
                                .as_ref()
                                .map(|date| date.instant().as_unix_ms()),
                        ),
                    ));
                }
            }
            // Publisher names order case-insensitively like titles; a row
            // without one lands at the direction's no-data end.
            ResourceSort::Publisher => {
                for (id, row) in resources.iter() {
                    keyed.push((
                        *id,
                        SortKey::Text(row.publisher.as_ref().map(|p| p.as_str().to_lowercase())),
                    ));
                }
            }
            // Record-keeping stamps are always present on a stored record —
            // create stamps both, edit restamps date_edited — so both date
            // sorts are plain instant orders.
            ResourceSort::DateAdded => {
                for (id, row) in resources.iter() {
                    keyed.push((*id, SortKey::Instant(Some(row.date_added.as_unix_ms()))));
                }
            }
            ResourceSort::DateEdited => {
                for (id, row) in resources.iter() {
                    keyed.push((*id, SortKey::Instant(Some(row.date_edited.as_unix_ms()))));
                }
            }
        }
        keyed.sort_by(|(a_id, a_key), (b_id, b_key)| {
            a_key
                .cmp(b_key, direction)
                .then_with(|| b_id.as_ulid().cmp(&a_id.as_ulid()))
        });
        Ok(keyed.into_iter().map(|(id, _)| id).collect())
    }

    /// Keep ids whose rows satisfy every set filter predicate.
    fn filter_ids(
        &self,
        ids: Vec<ResourceId>,
        filter: &ResourceFilter,
    ) -> Result<Vec<ResourceId>, StoreError> {
        if !filter.is_active() {
            return Ok(ids);
        }
        // The needle lowercases once, not per row; an unset predicate is a
        // no-op so the window math stays untouched.
        let needle = filter
            .title
            .as_ref()
            .map(|title| title.as_str().to_lowercase());
        let search = filter
            .search
            .as_ref()
            .map(|text| text.as_str().to_lowercase());
        // Creator membership resolves under its own lock, released before
        // the resource rows lock, so no path holds both at once.
        let by_creator: Option<HashSet<ResourceId>> = filter.creator.map(|creator| {
            self.created
                .lock()
                .expect("MemoryStore lock")
                .iter()
                .filter(|edge| edge.person_id == creator)
                .map(|edge| edge.resource_id)
                .collect()
        });
        let published = filter
            .published
            .filter(|range| range.is_active())
            .map(|range| {
                (
                    range.from.map(|from| from.as_unix_ms()),
                    range.to.map(|to| to.as_unix_ms()),
                )
            });
        let by_collection: Option<HashSet<CollectionId>> = filter.collection.map(|cid| {
            let edges = self
                .collection_child_of
                .lock()
                .expect("MemoryStore lock")
                .clone();
            collection_descendant_closure(cid, edges)
                .into_iter()
                .collect()
        });
        let resources = self.resources.lock().expect("MemoryStore lock");
        Ok(ids
            .into_iter()
            .filter(|id| {
                let resource = resources
                    .get(id)
                    .expect("ids come from the resources map keys");
                if let Some(kind) = filter.kind
                    && resource.kind != kind
                {
                    return false;
                }
                if let Some(favorite) = filter.favorite
                    && resource.favorite != favorite
                {
                    return false;
                }
                if let Some(tag) = filter.tag
                    && !resource.tag_ids.contains(&tag)
                {
                    return false;
                }
                if let Some(allowed) = by_collection.as_ref()
                    && !resource
                        .collection_ids
                        .iter()
                        .any(|cid| allowed.contains(cid))
                {
                    return false;
                }
                if let Some(needle) = needle.as_deref()
                    && !resource.title.as_str().to_lowercase().contains(needle)
                {
                    return false;
                }
                if let Some(needle) = search.as_deref() {
                    let title_matches = resource.title.as_str().to_lowercase().contains(needle);
                    let publisher_matches = resource
                        .publisher
                        .as_ref()
                        .is_some_and(|publisher| publisher.to_lowercase().contains(needle));
                    if !title_matches && !publisher_matches {
                        return false;
                    }
                }
                // Undated rows only match a window that bounds nothing.
                if let Some((from, to)) = published {
                    let Some(date) = resource.published.as_ref() else {
                        return false;
                    };
                    let instant = date.instant().as_unix_ms();
                    if from.is_some_and(|from| instant < from) {
                        return false;
                    }
                    if to.is_some_and(|to| instant >= to) {
                        return false;
                    }
                }
                by_creator
                    .as_ref()
                    .map(|set| set.contains(id))
                    .unwrap_or(true)
            })
            .collect())
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Store for MemoryStore {
    async fn create_note(
        &self,
        resource_id: ResourceId,
        content: ibex_core::NoteContent,
    ) -> Result<ibex_core::Note, StoreError> {
        content.validate()?;
        self.get_resource(resource_id).await?;
        let now = UtcMillis::now();
        let note = ibex_core::Note {
            id: ibex_core::NoteId::generate(),
            resource_id: Some(resource_id),
            content,
            created_at: now,
            updated_at: now,
        };
        self.notes.lock().unwrap().insert(note.id, note.clone());
        Ok(note)
    }
    async fn get_note(&self, id: ibex_core::NoteId) -> Result<ibex_core::Note, StoreError> {
        self.notes
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("note {id}")))
    }
    async fn update_note(
        &self,
        id: ibex_core::NoteId,
        content: ibex_core::NoteContent,
    ) -> Result<ibex_core::Note, StoreError> {
        content.validate()?;
        let mut notes = self.notes.lock().unwrap();
        let note = notes
            .get_mut(&id)
            .ok_or_else(|| StoreError::not_found(format!("note {id}")))?;
        note.content = content;
        note.updated_at = UtcMillis::now();
        Ok(note.clone())
    }
    async fn delete_note(&self, id: ibex_core::NoteId) -> Result<(), StoreError> {
        self.notes
            .lock()
            .unwrap()
            .remove(&id)
            .ok_or_else(|| StoreError::not_found(format!("note {id}")))?;
        Ok(())
    }
    async fn list_notes(
        &self,
        resource_id: ResourceId,
    ) -> Result<Vec<ibex_core::Note>, StoreError> {
        self.get_resource(resource_id).await?;
        let mut notes: Vec<_> = self
            .notes
            .lock()
            .unwrap()
            .values()
            .filter(|n| n.resource_id == Some(resource_id))
            .cloned()
            .collect();
        notes.sort_by_key(|n| (n.created_at.as_unix_ms(), n.id.to_string()));
        Ok(notes)
    }
    async fn ping(&self) -> Result<StoreHealth, StoreError> {
        Ok(StoreHealth {
            backend: "memory",
            ok: true,
        })
    }

    async fn create_person(&self, name: Name, aliases: Vec<Name>) -> Result<Person, StoreError> {
        let person = Person::try_new(PersonId::generate(), name, aliases)?;
        let mut state = self.people.lock().expect("MemoryStore lock");
        state.by_id.insert(person.id, person.clone());
        state.reindex_person(None, Some(&person));
        Ok(person)
    }

    async fn get_person(&self, id: PersonId) -> Result<Person, StoreError> {
        self.people
            .lock()
            .expect("MemoryStore lock")
            .by_id
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("person {id}")))
    }

    async fn update_person(
        &self,
        id: PersonId,
        name: Name,
        aliases: Vec<Name>,
    ) -> Result<Person, StoreError> {
        let mut state = self.people.lock().expect("MemoryStore lock");
        let before = state
            .by_id
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("person {id}")))?;
        let person = Person::try_new(id, name, aliases)?;
        state.by_id.insert(id, person.clone());
        state.reindex_person(Some(&before), Some(&person));
        Ok(person)
    }

    async fn delete_person(&self, id: PersonId) -> Result<(), StoreError> {
        let mut state = self.people.lock().expect("MemoryStore lock");
        let before = state
            .by_id
            .remove(&id)
            .ok_or_else(|| StoreError::not_found(format!("person {id}")))?;
        state.reindex_person(Some(&before), None);
        Ok(())
    }

    async fn find_persons_by_name(&self, name: &Name) -> Result<Vec<Person>, StoreError> {
        self.find_persons_by_name_calls
            .fetch_add(1, AtomicOrdering::SeqCst);
        let state = self.people.lock().expect("MemoryStore lock");
        let key = name.key();
        let Some(ids) = state.by_name.get(&key) else {
            return Ok(Vec::new());
        };
        Ok(ids
            .iter()
            .filter_map(|id| state.by_id.get(id).cloned())
            .collect())
    }

    async fn list_persons(&self) -> Result<Vec<Person>, StoreError> {
        self.list_persons_calls.fetch_add(1, AtomicOrdering::SeqCst);
        let state = self.people.lock().expect("MemoryStore lock");
        Ok(state.by_id.values().cloned().collect())
    }

    async fn create_series(&self, new: NewSeries) -> Result<Series, StoreError> {
        let series = Series::try_new(SeriesId::generate(), new);
        self.series
            .lock()
            .expect("MemoryStore lock")
            .insert(series.id, series.clone());
        Ok(series)
    }

    async fn get_series(&self, id: SeriesId) -> Result<Series, StoreError> {
        self.series
            .lock()
            .expect("MemoryStore lock")
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("series {id}")))
    }

    async fn create_tag(&self, text: NonEmptyStr) -> Result<Tag, StoreError> {
        let mut tags = self.tags.lock().expect("MemoryStore lock");
        if tags.values().any(|tag| tag.text == text) {
            return Err(StoreError::invalid(format!(
                "tag text already exists: {}",
                text.as_str()
            )));
        }
        let tag = Tag::try_new(TagId::generate(), text);
        tags.insert(tag.id, tag.clone());
        Ok(tag)
    }

    async fn get_tag(&self, id: TagId) -> Result<Tag, StoreError> {
        self.tags
            .lock()
            .expect("MemoryStore lock")
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("tag {id}")))
    }

    async fn find_tag_by_text(&self, text: &NonEmptyStr) -> Result<Option<Tag>, StoreError> {
        Ok(self
            .tags
            .lock()
            .expect("MemoryStore lock")
            .values()
            .find(|tag| &tag.text == text)
            .cloned())
    }

    async fn list_tags(&self) -> Result<Vec<Tag>, StoreError> {
        let mut tags: Vec<Tag> = self
            .tags
            .lock()
            .expect("MemoryStore lock")
            .values()
            .cloned()
            .collect();
        tags.sort_by(|a, b| a.text.as_str().cmp(b.text.as_str()));
        Ok(tags)
    }

    async fn create_collection(
        &self,
        name: NonEmptyStr,
        parent: Option<CollectionId>,
    ) -> Result<Collection, StoreError> {
        if let Some(parent_id) = parent {
            let _ = self.get_collection(parent_id).await?;
        }
        let collection = Collection::try_new(CollectionId::generate(), name);
        self.collections
            .lock()
            .expect("MemoryStore lock")
            .insert(collection.id, collection.clone());
        if let Some(parent_id) = parent {
            self.collection_child_of
                .lock()
                .expect("MemoryStore lock")
                .push((collection.id, parent_id));
        }
        Ok(collection)
    }

    async fn get_collection(&self, id: CollectionId) -> Result<Collection, StoreError> {
        self.collections
            .lock()
            .expect("MemoryStore lock")
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("collection {id}")))
    }

    async fn rename_collection(
        &self,
        id: CollectionId,
        name: NonEmptyStr,
    ) -> Result<Collection, StoreError> {
        let mut collections = self.collections.lock().expect("MemoryStore lock");
        let collection = collections
            .get_mut(&id)
            .ok_or_else(|| StoreError::not_found(format!("collection {id}")))?;
        collection.name = name;
        Ok(collection.clone())
    }

    async fn set_collection_classification(
        &self,
        id: CollectionId,
        keywords: Vec<String>,
        jev_enabled: bool,
    ) -> Result<Collection, StoreError> {
        let mut collections = self.collections.lock().expect("MemoryStore lock");
        let collection = collections
            .get_mut(&id)
            .ok_or_else(|| StoreError::not_found(format!("collection {id}")))?;
        collection.keywords = keywords;
        collection.jev_enabled = jev_enabled;
        Ok(collection.clone())
    }
    async fn update_collection(
        &self,
        update: ibex_core::CollectionUpdate,
    ) -> Result<Collection, StoreError> {
        let mut collections = self.collections.lock().expect("MemoryStore lock");
        let mut edges = self.collection_child_of.lock().expect("MemoryStore lock");
        let ids = collections.keys().copied().collect();
        let next = ibex_core::collection_update_edges(&update, &ids, &edges)?;
        let collection = collections
            .get_mut(&update.id)
            .expect("validated collection");
        if let Some(name) = update.name {
            collection.name = name;
        }
        if let Some(keywords) = update.keywords {
            collection.keywords = keywords;
        }
        if let Some(jev_enabled) = update.jev_enabled {
            collection.jev_enabled = jev_enabled;
        }
        *edges = next;
        Ok(collection.clone())
    }

    async fn list_collection_forest(&self) -> Result<Vec<CollectionTreeNode>, StoreError> {
        let collections: Vec<Collection> = self
            .collections
            .lock()
            .expect("MemoryStore lock")
            .values()
            .cloned()
            .collect();
        let edges = self
            .collection_child_of
            .lock()
            .expect("MemoryStore lock")
            .clone();
        Ok(build_collection_forest(collections, edges))
    }

    async fn add_collection_parent(
        &self,
        child: CollectionId,
        parent: CollectionId,
    ) -> Result<(), StoreError> {
        if child == parent {
            return Err(StoreError::invalid(
                "collection cannot be its own parent".to_owned(),
            ));
        }
        let _ = self.get_collection(child).await?;
        let _ = self.get_collection(parent).await?;
        let mut edges = self.collection_child_of.lock().expect("MemoryStore lock");
        if edges.iter().any(|(c, p)| *c == child && *p == parent) {
            return Err(StoreError::invalid(
                "collection parent edge already exists".to_owned(),
            ));
        }
        edges.push((child, parent));
        Ok(())
    }

    async fn remove_collection_parent(
        &self,
        child: CollectionId,
        parent: CollectionId,
    ) -> Result<(), StoreError> {
        let _ = self.get_collection(child).await?;
        let _ = self.get_collection(parent).await?;
        let mut edges = self.collection_child_of.lock().expect("MemoryStore lock");
        let before = edges.len();
        edges.retain(|(c, p)| !(*c == child && *p == parent));
        if edges.len() == before {
            return Err(StoreError::not_found(format!(
                "collection parent edge {child} -> {parent}"
            )));
        }
        Ok(())
    }

    async fn delete_collection(&self, id: CollectionId) -> Result<(), StoreError> {
        let _ = self.get_collection(id).await?;
        self.delete_collection_recursive(id);
        Ok(())
    }

    async fn collection_descendant_ids(
        &self,
        id: CollectionId,
    ) -> Result<Vec<CollectionId>, StoreError> {
        let _ = self.get_collection(id).await?;
        let edges = self
            .collection_child_of
            .lock()
            .expect("MemoryStore lock")
            .clone();
        Ok(collection_descendant_closure(id, edges))
    }

    async fn add_attachment(
        &self,
        resource_id: ResourceId,
        attachment: ibex_core::Attachment,
    ) -> Result<ibex_core::Attachment, StoreError> {
        let mut resources = self.resources.lock().expect("MemoryStore lock");
        let row = resources
            .get_mut(&resource_id)
            .ok_or_else(|| StoreError::not_found(format!("resource {resource_id}")))?;
        row.attachment_ids.push(attachment.id);
        self.attachments
            .lock()
            .expect("MemoryStore lock")
            .insert(attachment.id, attachment.clone());
        Ok(attachment)
    }

    async fn get_attachment(
        &self,
        id: ibex_core::AttachmentId,
    ) -> Result<ibex_core::Attachment, StoreError> {
        self.attachments
            .lock()
            .expect("MemoryStore lock")
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("attachment {id}")))
    }
    async fn create_resource(&self, new: NewResource) -> Result<Resource, StoreError> {
        let resource = Resource::try_new(ResourceId::generate(), new)?;
        let row = ResourceRow {
            kind: resource.kind,
            title: resource.title.clone(),
            publisher: resource.publisher.clone(),
            published: resource.published.clone(),
            accessed: resource.accessed.clone(),
            parent_id: resource.parent_id,
            series_id: resource.series_id,
            series_number: resource.series_number.clone(),
            tag_ids: resource.tag_ids.clone(),
            collection_ids: resource.collection_ids.clone(),
            attachment_ids: vec![],
            metadata: resource.metadata.clone(),
            subtitle: resource.subtitle.clone(),
            title_addon: resource.title_addon.clone(),
            short_title: resource.short_title.clone(),
            place: resource.place.clone(),
            volume: resource.volume.clone(),
            issue: resource.issue.clone(),
            pages: resource.pages.clone(),
            total_pages: resource.total_pages.clone(),
            article_number: resource.article_number.clone(),
            edition: resource.edition.clone(),
            doi: resource.doi.clone(),
            url: resource.url.clone(),
            eprint: resource.eprint.clone(),
            isbns: resource.isbns.clone(),
            issns: resource.issns.clone(),
            isrns: resource.isrns.clone(),
            original_date: resource.original_date.clone(),
            event_date: resource.event_date.clone(),
            language: resource.language.clone(),
            document_type: resource.document_type.clone(),
            publication_method: resource.publication_method.clone(),
            institution: resource.institution.clone(),
            organization: resource.organization.clone(),
            school: resource.school.clone(),
            abstract_text: resource.abstract_text.clone(),
            keywords: resource.keywords.clone(),
            favorite: resource.favorite,
            date_added: resource.date_added,
            date_edited: resource.date_edited,
        };
        self.resources
            .lock()
            .expect("MemoryStore lock")
            .insert(resource.id, row);

        let mut edges = self.created.lock().expect("MemoryStore lock");
        for creator in &resource.creators {
            edges.push(CreatedEdge {
                person_id: creator.person_id,
                resource_id: resource.id,
                role: creator.role,
                order: creator.order,
                credited_as: creator.credited_as.clone(),
            });
        }
        Ok(resource)
    }

    async fn create_imported_resource(
        &self,
        write: ImportedResourceWrite,
    ) -> Result<Resource, StoreError> {
        {
            let mut count = self.import_create_count.lock().expect("MemoryStore lock");
            let fail_at = *self.import_fail_at.lock().expect("MemoryStore lock");
            if fail_at == Some(*count) {
                *count += 1;
                return Err(StoreError::new("injected import persistence failure"));
            }
            *count += 1;
        }
        // Validate every creator against the pending people or the store before
        // any write, so a failed unit leaves no partial MemoryStore state.
        for creator in &write.resource.creators {
            let pending = write
                .new_people
                .iter()
                .any(|person| person.id == creator.person_id);
            if !pending {
                self.get_person(creator.person_id).await?;
            }
        }
        if let Some(series_id) = write.resource.series_id {
            let pending = write
                .new_series
                .as_ref()
                .is_some_and(|series| series.id == series_id);
            if !pending {
                self.get_series(series_id).await?;
            }
        } else if write.new_series.is_some() {
            return Err(StoreError::invalid(
                "imported write includes a series but resource.series_id is unset",
            ));
        }
        for dep in &write.new_dependencies {
            if let Some(parent_id) = dep.parent_id {
                let pending = write.new_dependencies.iter().any(|r| r.id == parent_id);
                if !pending {
                    self.get_resource(parent_id).await?;
                }
            }
        }
        if let Some(parent_id) = write.resource.parent_id {
            let pending = write.new_dependencies.iter().any(|r| r.id == parent_id);
            if !pending {
                self.get_resource(parent_id).await?;
            }
        }

        {
            let mut people = self.people.lock().expect("MemoryStore lock");
            for person in &write.new_people {
                if people.by_id.contains_key(&person.id) {
                    return Err(StoreError::invalid(format!(
                        "person {} already exists",
                        person.id
                    )));
                }
                people.by_id.insert(person.id, person.clone());
                people.reindex_person(None, Some(person));
            }
        }
        if let Some(series) = &write.new_series {
            let mut series_map = self.series.lock().expect("MemoryStore lock");
            if series_map.contains_key(&series.id) {
                return Err(StoreError::invalid(format!(
                    "series {} already exists",
                    series.id
                )));
            }
            series_map.insert(series.id, series.clone());
        }
        {
            let mut resources = self.resources.lock().expect("MemoryStore lock");
            for dep in &write.new_dependencies {
                if resources.contains_key(&dep.id) {
                    return Err(StoreError::invalid(format!(
                        "resource {} already exists",
                        dep.id
                    )));
                }
                resources.insert(
                    dep.id,
                    ResourceRow {
                        kind: dep.kind,
                        title: dep.title.clone(),
                        publisher: dep.publisher.clone(),
                        published: dep.published.clone(),
                        accessed: dep.accessed.clone(),
                        parent_id: dep.parent_id,
                        series_id: dep.series_id,
                        series_number: dep.series_number.clone(),
                        tag_ids: dep.tag_ids.clone(),
                        collection_ids: dep.collection_ids.clone(),
                        attachment_ids: vec![],
                        metadata: dep.metadata.clone(),
                        subtitle: dep.subtitle.clone(),
                        title_addon: dep.title_addon.clone(),
                        short_title: dep.short_title.clone(),
                        place: dep.place.clone(),
                        volume: dep.volume.clone(),
                        issue: dep.issue.clone(),
                        pages: dep.pages.clone(),
                        total_pages: dep.total_pages.clone(),
                        article_number: dep.article_number.clone(),
                        edition: dep.edition.clone(),
                        doi: dep.doi.clone(),
                        url: dep.url.clone(),
                        eprint: dep.eprint.clone(),
                        isbns: dep.isbns.clone(),
                        issns: dep.issns.clone(),
                        isrns: dep.isrns.clone(),
                        original_date: dep.original_date.clone(),
                        event_date: dep.event_date.clone(),
                        language: dep.language.clone(),
                        document_type: dep.document_type.clone(),
                        publication_method: dep.publication_method.clone(),
                        institution: dep.institution.clone(),
                        organization: dep.organization.clone(),
                        school: dep.school.clone(),
                        abstract_text: dep.abstract_text.clone(),
                        keywords: dep.keywords.clone(),
                        favorite: dep.favorite,
                        date_added: dep.date_added,
                        date_edited: dep.date_edited,
                    },
                );
            }
        }
        self.create_resource(write.resource).await
    }

    async fn update_resource(
        &self,
        id: ResourceId,
        new: NewResource,
    ) -> Result<Resource, StoreError> {
        let mut resources = self.resources.lock().expect("MemoryStore lock");
        // Existence first, matching the surreal adapter; the row's stamp is
        // the date_added an edit must carry forward.
        let (date_added, favorite) = match resources.get(&id) {
            Some(row) => (row.date_added, row.favorite),
            None => return Err(StoreError::not_found(format!("resource {id}"))),
        };
        let mut resource = Resource::try_new_updated(id, date_added, new)?;
        // Bibliographic replace never clears the star.
        resource.favorite = favorite;
        resource.attachment_ids = resources[&id].attachment_ids.clone();
        resources.insert(
            id,
            ResourceRow {
                kind: resource.kind,
                title: resource.title.clone(),
                publisher: resource.publisher.clone(),
                published: resource.published.clone(),
                accessed: resource.accessed.clone(),
                parent_id: resource.parent_id,
                series_id: resource.series_id,
                series_number: resource.series_number.clone(),
                tag_ids: resource.tag_ids.clone(),
                collection_ids: resource.collection_ids.clone(),
                attachment_ids: resource.attachment_ids.clone(),
                metadata: resource.metadata.clone(),
                subtitle: resource.subtitle.clone(),
                title_addon: resource.title_addon.clone(),
                short_title: resource.short_title.clone(),
                place: resource.place.clone(),
                volume: resource.volume.clone(),
                issue: resource.issue.clone(),
                pages: resource.pages.clone(),
                total_pages: resource.total_pages.clone(),
                article_number: resource.article_number.clone(),
                edition: resource.edition.clone(),
                doi: resource.doi.clone(),
                url: resource.url.clone(),
                eprint: resource.eprint.clone(),
                isbns: resource.isbns.clone(),
                issns: resource.issns.clone(),
                isrns: resource.isrns.clone(),
                original_date: resource.original_date.clone(),
                event_date: resource.event_date.clone(),
                language: resource.language.clone(),
                document_type: resource.document_type.clone(),
                publication_method: resource.publication_method.clone(),
                institution: resource.institution.clone(),
                organization: resource.organization.clone(),
                school: resource.school.clone(),
                abstract_text: resource.abstract_text.clone(),
                keywords: resource.keywords.clone(),
                favorite: resource.favorite,
                date_added: resource.date_added,
                date_edited: resource.date_edited,
            },
        );
        drop(resources);

        // Creators are replaced wholesale: stale edges go first, then the
        // new list lands with exactly the orders try_new assigned.
        let mut edges = self.created.lock().expect("MemoryStore lock");
        edges.retain(|e| e.resource_id != id);
        for creator in &resource.creators {
            edges.push(CreatedEdge {
                person_id: creator.person_id,
                resource_id: id,
                role: creator.role,
                order: creator.order,
                credited_as: creator.credited_as.clone(),
            });
        }
        Ok(resource)
    }

    async fn set_resource_favorite(
        &self,
        id: ResourceId,
        favorite: bool,
    ) -> Result<Resource, StoreError> {
        {
            let mut resources = self.resources.lock().expect("MemoryStore lock");
            let row = resources
                .get_mut(&id)
                .ok_or_else(|| StoreError::not_found(format!("resource {id}")))?;
            row.favorite = favorite;
        }
        self.get_resource(id).await
    }

    async fn add_resource_labels(
        &self,
        id: ResourceId,
        tag_ids: &[TagId],
        collection_ids: &[CollectionId],
    ) -> Result<(), StoreError> {
        {
            let mut count = self.label_write_count.lock().expect("MemoryStore lock");
            let fail_at = *self.label_fail_at.lock().expect("MemoryStore lock");
            if fail_at == Some(*count) {
                *count += 1;
                return Err(StoreError::new("injected label persistence failure"));
            }
            *count += 1;
        }
        {
            let mut resources = self.resources.lock().expect("MemoryStore lock");
            let row = resources
                .get_mut(&id)
                .ok_or_else(|| StoreError::not_found(format!("resource {id}")))?;
            for tag_id in tag_ids {
                if !row.tag_ids.contains(tag_id) {
                    row.tag_ids.push(*tag_id);
                }
            }
            for collection_id in collection_ids {
                if !row.collection_ids.contains(collection_id) {
                    row.collection_ids.push(*collection_id);
                }
            }
        }
        Ok(())
    }

    async fn get_resource(&self, id: ResourceId) -> Result<Resource, StoreError> {
        self.get_resource_calls.fetch_add(1, AtomicOrdering::SeqCst);
        let row = self
            .resources
            .lock()
            .expect("MemoryStore lock")
            .get(&id)
            .map(|r| ResourceRow {
                kind: r.kind,
                title: r.title.clone(),
                publisher: r.publisher.clone(),
                published: r.published.clone(),
                accessed: r.accessed.clone(),
                parent_id: r.parent_id,
                series_id: r.series_id,
                series_number: r.series_number.clone(),
                tag_ids: r.tag_ids.clone(),
                collection_ids: r.collection_ids.clone(),
                attachment_ids: r.attachment_ids.clone(),
                metadata: r.metadata.clone(),
                subtitle: r.subtitle.clone(),
                title_addon: r.title_addon.clone(),
                short_title: r.short_title.clone(),
                place: r.place.clone(),
                volume: r.volume.clone(),
                issue: r.issue.clone(),
                pages: r.pages.clone(),
                total_pages: r.total_pages.clone(),
                article_number: r.article_number.clone(),
                edition: r.edition.clone(),
                doi: r.doi.clone(),
                url: r.url.clone(),
                eprint: r.eprint.clone(),
                isbns: r.isbns.clone(),
                issns: r.issns.clone(),
                isrns: r.isrns.clone(),
                original_date: r.original_date.clone(),
                event_date: r.event_date.clone(),
                language: r.language.clone(),
                document_type: r.document_type.clone(),
                publication_method: r.publication_method.clone(),
                institution: r.institution.clone(),
                organization: r.organization.clone(),
                school: r.school.clone(),
                abstract_text: r.abstract_text.clone(),
                keywords: r.keywords.clone(),
                favorite: r.favorite,
                date_added: r.date_added,
                date_edited: r.date_edited,
            })
            .ok_or_else(|| StoreError::not_found(format!("resource {id}")))?;

        let mut creators: Vec<Creator> = self
            .created
            .lock()
            .expect("MemoryStore lock")
            .iter()
            .filter(|e| e.resource_id == id)
            .map(|e| Creator {
                person_id: e.person_id,
                role: e.role,
                order: e.order,
                credited_as: e.credited_as.clone(),
            })
            .collect();
        creators.sort_by_key(|c| c.order);

        let mut resource = Resource::from_stored(
            id,
            row.kind,
            row.title,
            creators,
            row.publisher,
            row.published,
            row.accessed,
            row.parent_id,
            row.series_id,
            row.series_number,
            row.tag_ids,
            row.collection_ids,
            row.metadata,
            row.favorite,
            row.date_added,
            row.date_edited,
        )
        .map_err(StoreError::from)?;
        resource.subtitle = row.subtitle;
        resource.title_addon = row.title_addon;
        resource.short_title = row.short_title;
        resource.place = row.place;
        resource.volume = row.volume;
        resource.issue = row.issue;
        resource.pages = row.pages;
        resource.total_pages = row.total_pages;
        resource.article_number = row.article_number;
        resource.edition = row.edition;
        resource.doi = row.doi;
        resource.url = row.url;
        resource.eprint = row.eprint;
        resource.isbns = row.isbns;
        resource.issns = row.issns;
        resource.isrns = row.isrns;
        resource.original_date = row.original_date;
        resource.event_date = row.event_date;
        resource.language = row.language;
        resource.document_type = row.document_type;
        resource.publication_method = row.publication_method;
        resource.institution = row.institution;
        resource.organization = row.organization;
        resource.school = row.school;
        resource.abstract_text = row.abstract_text;
        resource.keywords = row.keywords;
        resource.attachment_ids = row.attachment_ids;
        Ok(resource)
    }

    async fn list_resources(&self) -> Result<Vec<ResourceSummary>, StoreError> {
        // The full list is the first page with an unbounded cursor, so one
        // projection path serves both reads.
        let page = self.list_resources_page(None, u32::MAX).await?;
        Ok(page.rows)
    }

    async fn list_resources_for_import(&self) -> Result<Vec<Resource>, StoreError> {
        self.list_resources_for_import_calls
            .fetch_add(1, AtomicOrdering::SeqCst);
        let resources = self.resources.lock().expect("MemoryStore lock");
        let mut out = Vec::with_capacity(resources.len());
        for (id, row) in resources.iter() {
            // Identity/container seeding does not need creator edges.
            out.push(row.to_resource(*id)?);
        }
        Ok(out)
    }

    async fn resource_counts(&self) -> Result<ResourceCounts, StoreError> {
        let resources = self.resources.lock().expect("MemoryStore lock");
        let mut counts = ResourceCounts {
            all_items: resources.len() as u64,
            ..ResourceCounts::default()
        };
        for resource in resources.values() {
            *counts.by_kind.entry(resource.kind).or_default() += 1;
            if resource.favorite {
                counts.favorites += 1;
            }
            for tag_id in &resource.tag_ids {
                *counts.by_tag.entry(*tag_id).or_default() += 1;
            }
        }
        Ok(counts)
    }

    async fn list_resources_page(
        &self,
        before: Option<ResourceId>,
        limit: u32,
    ) -> Result<ResourcePage, StoreError> {
        let mut ids: Vec<ResourceId> = {
            let resources = self.resources.lock().expect("MemoryStore lock");
            resources
                .keys()
                .copied()
                .filter(|id| before.is_none_or(|b| id.as_ulid() < b.as_ulid()))
                .collect()
        };
        ids.sort_by_key(|a| std::cmp::Reverse(a.as_ulid()));
        ids.truncate(limit as usize + 1);
        let next_before = if ids.len() > limit as usize {
            ids.pop();
            ids.last().copied()
        } else {
            None
        };
        let rows = self.summaries_in_order(&ids)?;
        Ok(ResourcePage { rows, next_before })
    }

    async fn list_resources_query_page(
        &self,
        query: &ResourceQuery,
        offset: u64,
        limit: u32,
    ) -> Result<SortedResourcePage, StoreError> {
        // The order is the contract, so it is built once and windowed.
        let ids = match query.sort {
            // The default listing order: newest first by id.
            None => {
                let mut ids: Vec<ResourceId> = self
                    .resources
                    .lock()
                    .expect("MemoryStore lock")
                    .keys()
                    .copied()
                    .collect();
                ids.sort_by_key(|a| std::cmp::Reverse(a.as_ulid()));
                ids
            }
            Some((field, direction)) => self.sorted_ids(field, direction)?,
        };
        // Filters AND before the window: every set predicate must hold.
        let ids = self.filter_ids(ids, &query.filter)?;
        let start = (offset as usize).min(ids.len());
        let end = start.saturating_add(limit as usize).min(ids.len());
        let rows = self.summaries_in_order(&ids[start..end])?;
        // A full window with rows behind it leaves a cursor; a short window
        // is the end, so callers never land on an empty trailing page.
        let next_offset = (end < ids.len()).then(|| offset.saturating_add(u64::from(limit)));
        Ok(SortedResourcePage { rows, next_offset })
    }

    async fn list_resource_grid_page(
        &self,
        parent: ibex_core::ResourceGridParent,
        query: &ResourceQuery,
        offset: u64,
        limit: u32,
    ) -> Result<ibex_core::ResourceGridPage, StoreError> {
        use ibex_core::{ResourceGridPage, ResourceGridParent, ResourceGridRow};
        if limit == 0 {
            return Err(StoreError::invalid("page limit must be at least 1"));
        }

        let mut ids: Vec<_> = self
            .resources
            .lock()
            .expect("MemoryStore lock")
            .keys()
            .copied()
            .collect();
        if let Some((field, direction)) = query.sort {
            ids = self.sorted_ids(field, direction)?;
        } else {
            ids.sort_by_key(|a| std::cmp::Reverse(a.as_ulid()));
        }
        let mut visible: HashSet<_> = self
            .filter_ids(ids.clone(), &query.filter)?
            .into_iter()
            .collect();
        let parents: HashMap<_, _> = self
            .resources
            .lock()
            .expect("MemoryStore lock")
            .iter()
            .map(|(id, row)| (*id, row.parent_id))
            .collect();
        let mut frontier: Vec<_> = visible.iter().copied().collect();
        while let Some(id) = frontier.pop() {
            if let Some(Some(parent)) = parents.get(&id)
                && visible.insert(*parent)
            {
                frontier.push(*parent);
            }
        }
        let parent_id = match parent {
            ResourceGridParent::Roots => None,
            ResourceGridParent::Children(id) => Some(id),
        };
        ids.retain(|id| visible.contains(id) && parents.get(id) == Some(&parent_id));
        let start = usize::try_from(offset).unwrap_or(usize::MAX).min(ids.len());
        let end = start.saturating_add(limit as usize).min(ids.len());
        let summaries = self.summaries_in_order(&ids[start..end])?;
        let visible_parents: HashSet<_> = visible
            .iter()
            .filter_map(|id| parents.get(id).copied().flatten())
            .collect();
        let rows = summaries
            .into_iter()
            .map(|summary| ResourceGridRow {
                has_children: visible_parents.contains(&summary.id),
                summary,
            })
            .collect();
        Ok(ResourceGridPage {
            rows,
            next_offset: (end < ids.len()).then_some(end as u64),
        })
    }
}

pub struct StubStore {
    health: Result<StoreHealth, String>,
}

impl StubStore {
    pub fn healthy() -> Self {
        Self {
            health: Ok(StoreHealth {
                backend: "stub",
                ok: true,
            }),
        }
    }

    pub fn failing(msg: &str) -> Self {
        Self {
            health: Err(msg.to_string()),
        }
    }
}

fn unsupported(op: &str) -> StoreError {
    StoreError::new(format!("{op} not supported by StubStore"))
}

#[async_trait::async_trait]
impl Store for StubStore {
    async fn create_note(
        &self,
        _: ibex_core::ResourceId,
        _: ibex_core::NoteContent,
    ) -> Result<ibex_core::Note, ibex_core::StoreError> {
        Err(ibex_core::StoreError::new("notes unsupported"))
    }
    async fn get_note(
        &self,
        _: ibex_core::NoteId,
    ) -> Result<ibex_core::Note, ibex_core::StoreError> {
        Err(ibex_core::StoreError::new("notes unsupported"))
    }
    async fn update_note(
        &self,
        _: ibex_core::NoteId,
        _: ibex_core::NoteContent,
    ) -> Result<ibex_core::Note, ibex_core::StoreError> {
        Err(ibex_core::StoreError::new("notes unsupported"))
    }
    async fn delete_note(&self, _: ibex_core::NoteId) -> Result<(), ibex_core::StoreError> {
        Err(ibex_core::StoreError::new("notes unsupported"))
    }
    async fn list_notes(
        &self,
        _: ibex_core::ResourceId,
    ) -> Result<Vec<ibex_core::Note>, ibex_core::StoreError> {
        Err(ibex_core::StoreError::new("notes unsupported"))
    }
    async fn ping(&self) -> Result<StoreHealth, StoreError> {
        match &self.health {
            Ok(h) => Ok(h.clone()),
            Err(msg) => Err(StoreError::new(msg.clone())),
        }
    }

    async fn create_person(&self, _name: Name, _aliases: Vec<Name>) -> Result<Person, StoreError> {
        Err(unsupported("create_person"))
    }

    async fn get_person(&self, _id: PersonId) -> Result<Person, StoreError> {
        Err(unsupported("get_person"))
    }

    async fn update_person(
        &self,
        _id: PersonId,
        _name: Name,
        _aliases: Vec<Name>,
    ) -> Result<Person, StoreError> {
        Err(unsupported("update_person"))
    }

    async fn delete_person(&self, _id: PersonId) -> Result<(), StoreError> {
        Err(unsupported("delete_person"))
    }

    async fn find_persons_by_name(&self, _name: &Name) -> Result<Vec<Person>, StoreError> {
        Err(unsupported("find_persons_by_name"))
    }

    async fn list_persons(&self) -> Result<Vec<Person>, StoreError> {
        Err(unsupported("list_persons"))
    }

    async fn create_series(&self, _new: NewSeries) -> Result<Series, StoreError> {
        Err(unsupported("create_series"))
    }

    async fn get_series(&self, _id: SeriesId) -> Result<Series, StoreError> {
        Err(unsupported("get_series"))
    }

    async fn create_tag(&self, _text: NonEmptyStr) -> Result<Tag, StoreError> {
        Err(unsupported("create_tag"))
    }

    async fn get_tag(&self, _id: TagId) -> Result<Tag, StoreError> {
        Err(unsupported("get_tag"))
    }

    async fn find_tag_by_text(&self, _text: &NonEmptyStr) -> Result<Option<Tag>, StoreError> {
        Err(unsupported("find_tag_by_text"))
    }

    async fn list_tags(&self) -> Result<Vec<Tag>, StoreError> {
        Err(unsupported("list_tags"))
    }

    async fn create_collection(
        &self,
        _name: NonEmptyStr,
        _parent: Option<CollectionId>,
    ) -> Result<Collection, StoreError> {
        Err(unsupported("create_collection"))
    }

    async fn get_collection(&self, _id: CollectionId) -> Result<Collection, StoreError> {
        Err(unsupported("get_collection"))
    }

    async fn rename_collection(
        &self,
        _id: CollectionId,
        _name: NonEmptyStr,
    ) -> Result<Collection, StoreError> {
        Err(unsupported("rename_collection"))
    }

    async fn set_collection_classification(
        &self,
        _id: CollectionId,
        _keywords: Vec<String>,
        _jev_enabled: bool,
    ) -> Result<Collection, StoreError> {
        Err(unsupported("set_collection_classification"))
    }

    async fn update_collection(
        &self,
        _update: ibex_core::CollectionUpdate,
    ) -> Result<Collection, StoreError> {
        Err(unsupported("update_collection"))
    }

    async fn list_collection_forest(&self) -> Result<Vec<CollectionTreeNode>, StoreError> {
        Err(unsupported("list_collection_forest"))
    }

    async fn add_collection_parent(
        &self,
        _child: CollectionId,
        _parent: CollectionId,
    ) -> Result<(), StoreError> {
        Err(unsupported("add_collection_parent"))
    }

    async fn remove_collection_parent(
        &self,
        _child: CollectionId,
        _parent: CollectionId,
    ) -> Result<(), StoreError> {
        Err(unsupported("remove_collection_parent"))
    }

    async fn delete_collection(&self, _id: CollectionId) -> Result<(), StoreError> {
        Err(unsupported("delete_collection"))
    }

    async fn collection_descendant_ids(
        &self,
        _id: CollectionId,
    ) -> Result<Vec<CollectionId>, StoreError> {
        Err(unsupported("collection_descendant_ids"))
    }

    async fn add_attachment(
        &self,
        _resource_id: ResourceId,
        _attachment: ibex_core::Attachment,
    ) -> Result<ibex_core::Attachment, StoreError> {
        Err(unsupported("add_attachment"))
    }

    async fn get_attachment(
        &self,
        _id: ibex_core::AttachmentId,
    ) -> Result<ibex_core::Attachment, StoreError> {
        Err(unsupported("get_attachment"))
    }
    async fn create_resource(&self, _new: NewResource) -> Result<Resource, StoreError> {
        Err(unsupported("create_resource"))
    }

    async fn create_imported_resource(
        &self,
        _write: ImportedResourceWrite,
    ) -> Result<Resource, StoreError> {
        Err(unsupported("create_imported_resource"))
    }

    async fn update_resource(
        &self,
        _id: ResourceId,
        _new: NewResource,
    ) -> Result<Resource, StoreError> {
        Err(unsupported("update_resource"))
    }

    async fn set_resource_favorite(
        &self,
        _id: ResourceId,
        _favorite: bool,
    ) -> Result<Resource, StoreError> {
        Err(unsupported("set_resource_favorite"))
    }

    async fn get_resource(&self, _id: ResourceId) -> Result<Resource, StoreError> {
        Err(unsupported("get_resource"))
    }

    async fn list_resources(&self) -> Result<Vec<ResourceSummary>, StoreError> {
        Err(unsupported("list_resources"))
    }

    async fn list_resources_page(
        &self,
        _before: Option<ResourceId>,
        _limit: u32,
    ) -> Result<ResourcePage, StoreError> {
        Err(unsupported("list_resources_page"))
    }
}
