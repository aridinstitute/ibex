use std::collections::HashMap;

use oeuvre_core::{
    Creator, InheritanceError, NewResource, PersonId, Resource, ResourceCounts, ResourceDetail,
    ResourceError, ResourceId, ResourcePage, ResourceQuery, ResourceSummary, SeriesId,
    SortedResourcePage, StoreError, creator_label_name, detail_creator_groups, resolve_effective,
};
use oeuvre_core::{ResourceGridPage, ResourceGridParent};

use crate::event::{self, Event};
use crate::{AppState, CommandError, Handler, Outcome};

#[derive(Debug, Clone)]
pub struct CreateResource {
    pub new: NewResource,
}

#[derive(Debug, Clone)]
pub struct UpdateResource {
    /// Id of the record being replaced. The payload in `new` is the full
    /// replacement, not a delta: shells merge a fetched record's fields with
    /// the user's edits before invoking.
    pub id: ResourceId,
    pub new: NewResource,
}

#[derive(Debug, Clone)]
pub struct SetResourceFavorite {
    pub id: ResourceId,
    pub favorite: bool,
}

#[derive(Debug, Clone)]
pub struct GetResource {
    pub id: ResourceId,
}

#[derive(Debug, Clone)]
pub struct GetResourceDetail {
    pub id: ResourceId,
}

#[derive(Debug, Clone)]
pub struct ListResources;

#[derive(Debug, Clone)]
pub struct ListResourcesPage {
    /// Cursor from a previous page's `next_before`; `None` starts at the newest row.
    pub before: Option<ResourceId>,
    /// Page size in rows. Zero is rejected as invalid input.
    pub limit: u32,
}

#[derive(Debug, Clone)]
pub struct ListResourcesQueryPage {
    /// Order for the listing; `None` is the default newest-first order.
    pub query: ResourceQuery,
    /// 0-based window into the ordered list; `next_offset` continues.
    pub offset: u64,
    /// Page size in rows. Zero is rejected as invalid input.
    pub limit: u32,
}

#[derive(Debug, Clone)]
pub struct ListResourceGridPage {
    pub parent: ResourceGridParent,
    pub query: ResourceQuery,
    pub offset: u64,
    pub limit: u32,
}

pub(crate) struct ListedGridPage(pub ResourceGridPage);

impl From<ListedGridPage> for Outcome {
    fn from(ListedGridPage(page): ListedGridPage) -> Self {
        Outcome::ResourceGridPage(page)
    }
}

#[async_trait::async_trait]
impl Handler for ListResourceGridPage {
    const NAME: &'static str = "list_resource_grid_page";
    type Output = ListedGridPage;

    fn annotate(&self, ev: &mut Event) {
        ListResourcesQueryPage {
            query: self.query.clone(),
            offset: self.offset,
            limit: self.limit,
        }
        .annotate(ev);
        match self.parent {
            ResourceGridParent::Roots => {
                ev.set(event::RESOURCE_GRID_PARENT, "roots");
            }
            ResourceGridParent::Children(id) => {
                ev.set(event::RESOURCE_GRID_PARENT, "children");
                ev.set(event::RESOURCE_PARENT_ID, id.to_string());
            }
        }
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<ListedGridPage, CommandError> {
        if self.limit == 0 {
            return Err(CommandError::Invalid(
                "page limit must be at least 1".into(),
            ));
        }
        let page = state
            .store()
            .list_resource_grid_page(self.parent, &self.query, self.offset, self.limit)
            .await?;
        ev.set(event::RESOURCE_GRID_ROWS, page.rows.len() as u64);
        ev.set(event::RESOURCE_GRID_HAS_MORE, page.next_offset.is_some());
        Ok(ListedGridPage(page))
    }
}

#[derive(Debug, Clone)]
pub struct CountResources;

pub(crate) struct Created(pub Resource);
pub(crate) struct Updated(pub Resource);
pub(crate) struct Favorited(pub Resource);
pub(crate) struct Fetched(pub Resource);
pub(crate) struct FetchedDetail(pub ResourceDetail);
pub(crate) struct Listed(pub Vec<ResourceSummary>);
pub(crate) struct ListedPage(pub ResourcePage);
pub(crate) struct ListedQueryPage(pub SortedResourcePage);
pub(crate) struct Counted(pub ResourceCounts);

impl From<Created> for Outcome {
    fn from(Created(resource): Created) -> Self {
        Outcome::CreateResource(resource)
    }
}

impl From<Updated> for Outcome {
    fn from(Updated(resource): Updated) -> Self {
        Outcome::UpdateResource(resource)
    }
}

impl From<Favorited> for Outcome {
    fn from(Favorited(resource): Favorited) -> Self {
        Outcome::SetResourceFavorite(resource)
    }
}

impl From<Fetched> for Outcome {
    fn from(Fetched(resource): Fetched) -> Self {
        Outcome::GetResource(resource)
    }
}

impl From<FetchedDetail> for Outcome {
    fn from(FetchedDetail(detail): FetchedDetail) -> Self {
        Outcome::GetResourceDetail(detail)
    }
}

impl From<Listed> for Outcome {
    fn from(Listed(rows): Listed) -> Self {
        Outcome::ListResources(rows)
    }
}

impl From<ListedPage> for Outcome {
    fn from(ListedPage(page): ListedPage) -> Self {
        Outcome::ListResourcesPage(page)
    }
}

impl From<ListedQueryPage> for Outcome {
    fn from(ListedQueryPage(page): ListedQueryPage) -> Self {
        Outcome::ListResourcesQueryPage(page)
    }
}

impl From<Counted> for Outcome {
    fn from(Counted(counts): Counted) -> Self {
        Outcome::CountResources(counts)
    }
}

fn map_person_lookup(err: StoreError, id: PersonId) -> CommandError {
    if err.is_not_found() {
        CommandError::NotFound(id.to_string())
    } else if err.is_invalid() {
        CommandError::Invalid(err.message.clone())
    } else {
        CommandError::Store(err)
    }
}

fn map_resource_lookup(err: StoreError, id: ResourceId) -> CommandError {
    if err.is_not_found() {
        CommandError::NotFound(id.to_string())
    } else if err.is_invalid() {
        CommandError::Invalid(err.message.clone())
    } else {
        CommandError::Store(err)
    }
}

fn map_series_lookup(err: StoreError, id: SeriesId) -> CommandError {
    if err.is_not_found() {
        CommandError::NotFound(id.to_string())
    } else if err.is_invalid() {
        CommandError::Invalid(err.message.clone())
    } else {
        CommandError::Store(err)
    }
}

fn map_resource_err(err: ResourceError) -> CommandError {
    CommandError::Invalid(err.to_string())
}

fn map_inheritance_err(err: InheritanceError) -> CommandError {
    CommandError::Invalid(err.to_string())
}

/// Load the parent chain for read-time inheritance (nearest parent first).
async fn load_ancestors(
    state: &AppState,
    resource: &Resource,
) -> Result<Vec<Resource>, CommandError> {
    let mut chain = Vec::new();
    let mut next = resource.parent_id;
    while let Some(parent_id) = next {
        if chain.len() >= oeuvre_core::INHERITANCE_MAX_DEPTH {
            return Err(map_inheritance_err(InheritanceError::DepthExceeded));
        }
        let parent = state
            .store()
            .get_resource(parent_id)
            .await
            .map_err(|e| map_resource_lookup(e, parent_id))?;
        next = parent.parent_id;
        chain.push(parent);
    }
    Ok(chain)
}

fn map_store(err: StoreError) -> CommandError {
    if err.is_invalid() {
        CommandError::Invalid(err.message.clone())
    } else if err.is_not_found() {
        CommandError::NotFound(err.message.clone())
    } else {
        CommandError::Store(err)
    }
}

/// Shared reference checks for ordinary create and import.
///
/// `pending_people` / `pending_series` / `pending_resources` are ids that will
/// be created in the same atomic write, so they need not already exist in the
/// store. Pending resources still contribute their kinds via a follow-up load
/// only when absent from the pending set — callers must ensure parent kinds of
/// pending deps are valid (import plans journal before issue).
pub(crate) async fn validate_new_resource_refs(
    state: &AppState,
    new: &NewResource,
    pending_people: &[PersonId],
    pending_series: Option<SeriesId>,
    pending_resources: &[ResourceId],
) -> Result<(), CommandError> {
    for spec in &new.creators {
        if pending_people.contains(&spec.person_id) {
            continue;
        }
        state
            .store()
            .get_person(spec.person_id)
            .await
            .map_err(|e| map_person_lookup(e, spec.person_id))?;
    }

    if let Some(parent_id) = new.parent_id {
        if pending_resources.contains(&parent_id) {
            // Kind checked when the pending parent itself was validated, or by
            // the import planner (journal → issue → article).
        } else {
            let parent = state
                .store()
                .get_resource(parent_id)
                .await
                .map_err(|e| map_resource_lookup(e, parent_id))?;
            new.kind
                .accept_parent(parent.kind)
                .map_err(map_resource_err)?;
        }
    }

    if let Some(series_id) = new.series_id {
        if pending_series != Some(series_id) {
            state
                .store()
                .get_series(series_id)
                .await
                .map_err(|e| map_series_lookup(e, series_id))?;
        }
    }

    for tag_id in &new.tag_ids {
        state.store().get_tag(*tag_id).await.map_err(|e| {
            if e.is_not_found() {
                CommandError::NotFound(tag_id.to_string())
            } else if e.is_invalid() {
                CommandError::Invalid(e.message.clone())
            } else {
                CommandError::Store(e)
            }
        })?;
    }

    for collection_id in &new.collection_ids {
        state
            .store()
            .get_collection(*collection_id)
            .await
            .map_err(|e| {
                if e.is_not_found() {
                    CommandError::NotFound(collection_id.to_string())
                } else if e.is_invalid() {
                    CommandError::Invalid(e.message.clone())
                } else {
                    CommandError::Store(e)
                }
            })?;
    }

    Ok(())
}

/// Field names that differ between the stored record and its replacement,
/// in a fixed column order. The update's wide event joins them into
/// `resource.changed_fields`; an empty vec means the write is a no-op.
fn changed_fields(old: &Resource, new: &NewResource) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if old.kind != new.kind {
        changed.push("kind");
    }
    if old.title != new.title {
        changed.push("title");
    }
    if !old
        .creators
        .iter()
        .map(Creator::to_spec)
        .eq(new.creators.iter().cloned())
    {
        changed.push("creators");
    }
    if old.publisher != new.publisher {
        changed.push("publisher");
    }
    if old.published != new.published {
        changed.push("published");
    }
    if old.accessed != new.accessed {
        changed.push("accessed");
    }
    if old.parent_id != new.parent_id {
        changed.push("parent");
    }
    if old.series_id != new.series_id {
        changed.push("series");
    }
    if old.series_number != new.series_number {
        changed.push("series_number");
    }
    if old.tag_ids != new.tag_ids {
        changed.push("tags");
    }
    if old.collection_ids != new.collection_ids {
        changed.push("collections");
    }
    if old.metadata != new.metadata {
        changed.push("metadata");
    }
    if old.subtitle != new.subtitle {
        changed.push("subtitle");
    }
    if old.title_addon != new.title_addon {
        changed.push("title_addon");
    }
    if old.short_title != new.short_title {
        changed.push("short_title");
    }
    if old.place != new.place {
        changed.push("place");
    }
    if old.volume != new.volume {
        changed.push("volume");
    }
    if old.issue != new.issue {
        changed.push("issue");
    }
    if old.pages != new.pages {
        changed.push("pages");
    }
    if old.total_pages != new.total_pages {
        changed.push("total_pages");
    }
    if old.article_number != new.article_number {
        changed.push("article_number");
    }
    if old.edition != new.edition {
        changed.push("edition");
    }
    if old.doi != new.doi {
        changed.push("doi");
    }
    if old.url != new.url {
        changed.push("url");
    }
    if old.eprint != new.eprint {
        changed.push("eprint");
    }
    if old.isbns != new.isbns {
        changed.push("isbns");
    }
    if old.issns != new.issns {
        changed.push("issns");
    }
    if old.isrns != new.isrns {
        changed.push("isrns");
    }
    if old.original_date != new.original_date {
        changed.push("original_date");
    }
    if old.event_date != new.event_date {
        changed.push("event_date");
    }
    if old.language != new.language {
        changed.push("language");
    }
    if old.document_type != new.document_type {
        changed.push("document_type");
    }
    if old.publication_method != new.publication_method {
        changed.push("publication_method");
    }
    if old.institution != new.institution {
        changed.push("institution");
    }
    if old.organization != new.organization {
        changed.push("organization");
    }
    if old.school != new.school {
        changed.push("school");
    }
    if old.abstract_text != new.abstract_text {
        changed.push("abstract_text");
    }
    if old.keywords != new.keywords {
        changed.push("keywords");
    }
    changed
}

#[async_trait::async_trait]
impl Handler for CreateResource {
    const NAME: &'static str = "create_resource";
    type Output = Created;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::RESOURCE_KIND, self.new.kind.as_str());
        ev.set(event::RESOURCE_TITLE, self.new.title.as_str().to_owned());
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Created, CommandError> {
        validate_new_resource_refs(state, &self.new, &[], None, &[]).await?;

        let resource = state
            .store()
            .create_resource(self.new)
            .await
            .map_err(map_store)?;

        ev.set(event::RESOURCE_ID, resource.id.to_string());
        ev.set(event::RESOURCE_KIND, resource.kind.as_str());
        ev.set(event::RESOURCE_TITLE, resource.title.as_str().to_owned());
        Ok(Created(resource))
    }
}

#[async_trait::async_trait]
impl Handler for UpdateResource {
    const NAME: &'static str = "update_resource";
    type Output = Updated;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::RESOURCE_ID, self.id.to_string());
        ev.set(event::RESOURCE_KIND, self.new.kind.as_str());
        ev.set(event::RESOURCE_TITLE, self.new.title.as_str().to_owned());
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Updated, CommandError> {
        // One read covers both the not-found check and the changed-field
        // diff, so the event records what the update would have changed even
        // when reference validation later rejects it.
        let old = state
            .store()
            .get_resource(self.id)
            .await
            .map_err(|e| map_resource_lookup(e, self.id))?;
        ev.set(
            event::RESOURCE_CHANGED_FIELDS,
            changed_fields(&old, &self.new).join(","),
        );

        // Same reference checks as CreateResource, against the replacement.
        for spec in &self.new.creators {
            state
                .store()
                .get_person(spec.person_id)
                .await
                .map_err(|e| map_person_lookup(e, spec.person_id))?;
        }

        if let Some(parent_id) = self.new.parent_id {
            let parent = state
                .store()
                .get_resource(parent_id)
                .await
                .map_err(|e| map_resource_lookup(e, parent_id))?;
            self.new
                .kind
                .accept_parent(parent.kind)
                .map_err(map_resource_err)?;
        }

        if let Some(series_id) = self.new.series_id {
            state
                .store()
                .get_series(series_id)
                .await
                .map_err(|e| map_series_lookup(e, series_id))?;
        }

        for tag_id in &self.new.tag_ids {
            state.store().get_tag(*tag_id).await.map_err(|e| {
                if e.is_not_found() {
                    CommandError::NotFound(tag_id.to_string())
                } else if e.is_invalid() {
                    CommandError::Invalid(e.message.clone())
                } else {
                    CommandError::Store(e)
                }
            })?;
        }

        for collection_id in &self.new.collection_ids {
            state
                .store()
                .get_collection(*collection_id)
                .await
                .map_err(|e| {
                    if e.is_not_found() {
                        CommandError::NotFound(collection_id.to_string())
                    } else if e.is_invalid() {
                        CommandError::Invalid(e.message.clone())
                    } else {
                        CommandError::Store(e)
                    }
                })?;
        }

        let resource = state
            .store()
            .update_resource(self.id, self.new)
            .await
            .map_err(map_store)?;

        Ok(Updated(resource))
    }
}

#[async_trait::async_trait]
impl Handler for SetResourceFavorite {
    const NAME: &'static str = "set_resource_favorite";
    type Output = Favorited;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::RESOURCE_ID, self.id.to_string());
        ev.set(event::RESOURCE_FAVORITE, self.favorite);
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Favorited, CommandError> {
        let resource = state
            .store()
            .set_resource_favorite(self.id, self.favorite)
            .await
            .map_err(|e| map_resource_lookup(e, self.id))?;
        ev.set(event::RESOURCE_KIND, resource.kind.as_str());
        ev.set(event::RESOURCE_TITLE, resource.title.as_str().to_owned());
        ev.set(event::RESOURCE_FAVORITE, resource.favorite);
        Ok(Favorited(resource))
    }
}

#[async_trait::async_trait]
impl Handler for GetResource {
    const NAME: &'static str = "get_resource";
    type Output = Fetched;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::RESOURCE_ID, self.id.to_string());
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Fetched, CommandError> {
        let resource = state
            .store()
            .get_resource(self.id)
            .await
            .map_err(|e| map_resource_lookup(e, self.id))?;
        ev.set(event::RESOURCE_KIND, resource.kind.as_str());
        ev.set(event::RESOURCE_TITLE, resource.title.as_str().to_owned());
        Ok(Fetched(resource))
    }
}

#[async_trait::async_trait]
impl Handler for GetResourceDetail {
    const NAME: &'static str = "get_resource_detail";
    type Output = FetchedDetail;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::RESOURCE_ID, self.id.to_string());
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<FetchedDetail, CommandError> {
        let resource = state
            .store()
            .get_resource(self.id)
            .await
            .map_err(|e| map_resource_lookup(e, self.id))?;
        ev.set(event::RESOURCE_KIND, resource.kind.as_str());
        ev.set(event::RESOURCE_TITLE, resource.title.as_str().to_owned());

        // One person lookup per distinct id: a person may fill several roles,
        // and credited_as belongs to the creator slot, not the person, so
        // labels are composed per slot from the cached names.
        let mut people: HashMap<PersonId, oeuvre_core::Name> = HashMap::new();
        for creator in &resource.creators {
            if people.contains_key(&creator.person_id) {
                continue;
            }
            let person = state
                .store()
                .get_person(creator.person_id)
                .await
                .map_err(|e| map_person_lookup(e, creator.person_id))?;
            people.insert(creator.person_id, person.name);
        }

        // Central read-time inheritance: allowlisted fields + container title.
        // Stored child rows are not mutated; GetResource still returns raw data.
        let ancestors = load_ancestors(state, &resource).await?;
        let effective = resolve_effective(&resource, &ancestors).map_err(map_inheritance_err)?;

        let series_title = match resource.series_id {
            Some(series_id) => {
                let series = state
                    .store()
                    .get_series(series_id)
                    .await
                    .map_err(|e| map_series_lookup(e, series_id))?;
                Some(series.title)
            }
            None => None,
        };

        let mut tags = Vec::with_capacity(resource.tag_ids.len());
        for tag_id in &resource.tag_ids {
            let tag = state.store().get_tag(*tag_id).await.map_err(|e| {
                if e.is_not_found() {
                    CommandError::NotFound(tag_id.to_string())
                } else if e.is_invalid() {
                    CommandError::Invalid(e.message.clone())
                } else {
                    CommandError::Store(e)
                }
            })?;
            tags.push(tag);
        }

        Ok(FetchedDetail(ResourceDetail {
            subtitle: resource.subtitle,
            title_addon: resource.title_addon,
            short_title: resource.short_title,
            place: effective.place.value,
            volume: effective.volume.value,
            issue: effective.issue.value,
            pages: resource.pages,
            total_pages: resource.total_pages,
            article_number: resource.article_number,
            edition: effective.edition.value,
            doi: resource.doi,
            url: resource.url,
            eprint: resource.eprint,
            isbns: effective.isbns.value.unwrap_or_default(),
            issns: effective.issns.value.unwrap_or_default(),
            isrns: resource.isrns,
            original_date: resource.original_date,
            event_date: resource.event_date,
            language: effective.language.value,
            document_type: resource.document_type,
            publication_method: resource.publication_method,
            institution: resource.institution,
            organization: resource.organization,
            school: resource.school,
            abstract_text: resource.abstract_text,
            keywords: resource.keywords,
            id: resource.id,
            kind: resource.kind,
            title: resource.title,
            creators: detail_creator_groups(&resource.creators, |creator| {
                creator_label_name(
                    creator.credited_as.as_ref(),
                    people
                        .get(&creator.person_id)
                        .expect("person fetched above"),
                )
            }),
            publisher: effective.publisher.value,
            published: effective.published.value,
            accessed: resource.accessed,
            container_title: effective.container_title.value,
            series_title,
            series_number: resource.series_number,
            favorite: resource.favorite,
            tags,
            date_added: resource.date_added,
            date_edited: resource.date_edited,
        }))
    }
}

#[async_trait::async_trait]
impl Handler for ListResources {
    const NAME: &'static str = "list_resources";
    type Output = Listed;

    async fn run(self, state: &AppState, _ev: &mut Event) -> Result<Listed, CommandError> {
        Ok(Listed(state.store().list_resources().await?))
    }
}

#[async_trait::async_trait]
impl Handler for CountResources {
    const NAME: &'static str = "count_resources";
    type Output = Counted;

    async fn run(self, state: &AppState, _ev: &mut Event) -> Result<Counted, CommandError> {
        Ok(Counted(state.store().resource_counts().await?))
    }
}

#[async_trait::async_trait]
impl Handler for ListResourcesPage {
    const NAME: &'static str = "list_resources_page";
    type Output = ListedPage;

    fn annotate(&self, ev: &mut Event) {
        if let Some(before) = self.before {
            ev.set(event::RESOURCE_ID, before.to_string());
        }
        ev.set(event::RESOURCE_PAGE_LIMIT, self.limit);
    }

    async fn run(self, state: &AppState, _ev: &mut Event) -> Result<ListedPage, CommandError> {
        // A zero limit has no meaning at the store layer, so reject it at the
        // command boundary where invalid input maps to an exit code.
        if self.limit == 0 {
            return Err(CommandError::Invalid(
                "page limit must be at least 1".into(),
            ));
        }
        let page = state
            .store()
            .list_resources_page(self.before, self.limit)
            .await?;
        Ok(ListedPage(page))
    }
}

#[async_trait::async_trait]
impl Handler for ListResourcesQueryPage {
    const NAME: &'static str = "list_resources_query_page";
    type Output = ListedQueryPage;

    fn annotate(&self, ev: &mut Event) {
        if let Some((sort, direction)) = self.query.sort {
            ev.set(event::RESOURCE_SORT_FIELD, sort.as_str());
            ev.set(event::RESOURCE_SORT_DIRECTION, direction.as_str());
        }
        if let Some(title) = self.query.filter.title.as_ref() {
            ev.set(event::RESOURCE_FILTER_TITLE, title.as_str().to_owned());
        }
        if let Some(search) = self.query.filter.search.as_ref() {
            ev.set(event::RESOURCE_FILTER_SEARCH, search.as_str().to_owned());
        }
        if let Some(creator) = self.query.filter.creator {
            ev.set(event::RESOURCE_FILTER_CREATOR, creator.to_string());
        }
        if let Some(kind) = self.query.filter.kind {
            ev.set(event::RESOURCE_FILTER_KIND, kind.as_str());
        }
        if let Some(favorite) = self.query.filter.favorite {
            ev.set(event::RESOURCE_FILTER_FAVORITE, favorite);
        }
        if let Some(tag) = self.query.filter.tag {
            ev.set(event::RESOURCE_FILTER_TAG, tag.to_string());
        }
        if let Some(collection) = self.query.filter.collection {
            ev.set(event::RESOURCE_FILTER_COLLECTION, collection.to_string());
        }
        if let Some(range) = self.query.filter.published {
            if let Some(from) = range.from {
                ev.set(event::RESOURCE_FILTER_PUBLISHED_FROM, from.as_unix_ms());
            }
            if let Some(to) = range.to {
                ev.set(event::RESOURCE_FILTER_PUBLISHED_TO, to.as_unix_ms());
            }
        }
        ev.set(event::RESOURCE_PAGE_OFFSET, self.offset);
        ev.set(event::RESOURCE_PAGE_LIMIT, self.limit);
    }

    async fn run(self, state: &AppState, _ev: &mut Event) -> Result<ListedQueryPage, CommandError> {
        // A zero limit has no meaning at the store layer, so reject it at
        // the command boundary where invalid input maps to an exit code.
        if self.limit == 0 {
            return Err(CommandError::Invalid(
                "page limit must be at least 1".into(),
            ));
        }
        let page = state
            .store()
            .list_resources_query_page(&self.query, self.offset, self.limit)
            .await?;
        Ok(ListedQueryPage(page))
    }
}
