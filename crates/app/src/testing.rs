use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use oeuvre_core::{
    BibliographicDate, Collection, CollectionId, CollectionTreeNode, Creator, CreatorRole, Name,
    NameKey, NewResource, NewSeries, NonEmptyStr, Person, PersonId, Resource, ResourceCounts,
    ResourceFilter, ResourceId, ResourceKind, ResourcePage, ResourceQuery, ResourceSort,
    ResourceSummary, Series, SeriesId, SortDirection, SortedResourcePage, Store, StoreError,
    StoreHealth, Tag, TagId, UtcMillis, build_collection_forest, collection_descendant_closure,
    creator_label_name, format_creators_label, primary_creator_sort_key, published_label_from,
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

struct ResourceRow {
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
    favorite: bool,
    date_added: UtcMillis,
    date_edited: UtcMillis,
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

pub struct MemoryStore {
    people: Mutex<PeopleState>,
    series: Mutex<HashMap<SeriesId, Series>>,
    tags: Mutex<HashMap<TagId, Tag>>,
    collections: Mutex<HashMap<CollectionId, Collection>>,
    /// `(child, parent)` edges for `child_of`.
    collection_child_of: Mutex<Vec<(CollectionId, CollectionId)>>,
    resources: Mutex<HashMap<ResourceId, ResourceRow>>,
    created: Mutex<Vec<CreatedEdge>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            people: Mutex::new(PeopleState::new()),
            series: Mutex::new(HashMap::new()),
            tags: Mutex::new(HashMap::new()),
            collections: Mutex::new(HashMap::new()),
            collection_child_of: Mutex::new(Vec::new()),
            resources: Mutex::new(HashMap::new()),
            created: Mutex::new(Vec::new()),
        }
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

            let container_title = match row.parent_id {
                Some(parent_id) => resources.get(&parent_id).map(|parent| parent.title.clone()),
                None => None,
            };

            summaries.push(ResourceSummary {
                id: *id,
                kind: row.kind,
                title: row.title.clone(),
                creators_label: format_creators_label(&names),
                published_label: published_label_from(row.published.as_ref()),
                publisher: row.publisher.clone(),
                container_title,
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
        let resources = self.resources.lock().expect("MemoryStore lock");
        Ok(ids
            .into_iter()
            .filter(|id| {
                let resource = resources
                    .get(id)
                    .expect("ids come from the resources map keys");
                if let Some(kind) = filter.kind {
                    if resource.kind != kind {
                        return false;
                    }
                }
                if let Some(favorite) = filter.favorite {
                    if resource.favorite != favorite {
                        return false;
                    }
                }
                if let Some(tag) = filter.tag {
                    if !resource.tag_ids.contains(&tag) {
                        return false;
                    }
                }
                if let Some(needle) = needle.as_deref() {
                    if !resource.title.as_str().to_lowercase().contains(needle) {
                        return false;
                    }
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
        if edges
            .iter()
            .any(|(c, p)| *c == child && *p == parent)
        {
            return Err(StoreError::invalid(
                "collection parent edge already exists".to_owned(),
            ));
        }
        edges.push((child, parent));
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

    async fn get_resource(&self, id: ResourceId) -> Result<Resource, StoreError> {
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

        Resource::from_stored(
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
            row.favorite,
            row.date_added,
            row.date_edited,
        )
        .map_err(StoreError::from)
    }

    async fn list_resources(&self) -> Result<Vec<ResourceSummary>, StoreError> {
        // The full list is the first page with an unbounded cursor, so one
        // projection path serves both reads.
        let page = self.list_resources_page(None, u32::MAX).await?;
        Ok(page.rows)
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
        ids.sort_by(|a, b| b.as_ulid().cmp(&a.as_ulid()));
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
                ids.sort_by(|a, b| b.as_ulid().cmp(&a.as_ulid()));
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

    async fn delete_collection(&self, _id: CollectionId) -> Result<(), StoreError> {
        Err(unsupported("delete_collection"))
    }

    async fn collection_descendant_ids(
        &self,
        _id: CollectionId,
    ) -> Result<Vec<CollectionId>, StoreError> {
        Err(unsupported("collection_descendant_ids"))
    }

    async fn create_resource(&self, _new: NewResource) -> Result<Resource, StoreError> {
        Err(unsupported("create_resource"))
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
