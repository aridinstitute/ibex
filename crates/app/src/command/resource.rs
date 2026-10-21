use std::collections::HashMap;

use oeuvre_core::{
    creator_label_name, detail_creator_groups, NewResource, PersonId, Resource, ResourceDetail,
    ResourceError, ResourceId, ResourcePage, ResourceQuery, ResourceSummary, SeriesId,
    SortedResourcePage, StoreError,
};

use crate::event::{self, Event};
use crate::{AppState, CommandError, Handler, Outcome};

#[derive(Debug, Clone)]
pub struct CreateResource {
    pub new: NewResource,
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

pub(crate) struct Created(pub Resource);
pub(crate) struct Fetched(pub Resource);
pub(crate) struct FetchedDetail(pub ResourceDetail);
pub(crate) struct Listed(pub Vec<ResourceSummary>);
pub(crate) struct ListedPage(pub ResourcePage);
pub(crate) struct ListedQueryPage(pub SortedResourcePage);

impl From<Created> for Outcome {
    fn from(Created(resource): Created) -> Self {
        Outcome::CreateResource(resource)
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

fn map_store(err: StoreError) -> CommandError {
    if err.is_invalid() {
        CommandError::Invalid(err.message.clone())
    } else if err.is_not_found() {
        CommandError::NotFound(err.message.clone())
    } else {
        CommandError::Store(err)
    }
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

        // Detail views name the container and series rather than their ids,
        // so resolve both here where a missing row still maps to the id the
        // caller asked about.
        let container_title = match resource.parent_id {
            Some(parent_id) => {
                let parent = state
                    .store()
                    .get_resource(parent_id)
                    .await
                    .map_err(|e| map_resource_lookup(e, parent_id))?;
                Some(parent.title)
            }
            None => None,
        };
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

        Ok(FetchedDetail(ResourceDetail {
            id: resource.id,
            kind: resource.kind,
            title: resource.title,
            creators: detail_creator_groups(&resource.creators, |creator| {
                creator_label_name(
                    creator.credited_as.as_ref(),
                    people.get(&creator.person_id).expect("person fetched above"),
                )
            }),
            publisher: resource.publisher,
            published: resource.published,
            accessed: resource.accessed,
            container_title,
            series_title,
            series_number: resource.series_number,
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
            return Err(CommandError::Invalid("page limit must be at least 1".into()));
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
        ev.set(event::RESOURCE_PAGE_OFFSET, self.offset);
        ev.set(event::RESOURCE_PAGE_LIMIT, self.limit);
    }

    async fn run(
        self,
        state: &AppState,
        _ev: &mut Event,
    ) -> Result<ListedQueryPage, CommandError> {
        // A zero limit has no meaning at the store layer, so reject it at
        // the command boundary where invalid input maps to an exit code.
        if self.limit == 0 {
            return Err(CommandError::Invalid("page limit must be at least 1".into()));
        }
        let page = state
            .store()
            .list_resources_query_page(&self.query, self.offset, self.limit)
            .await?;
        Ok(ListedQueryPage(page))
    }
}
