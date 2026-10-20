use oeuvre_core::{
    NewResource, PersonId, Resource, ResourceError, ResourceId, SeriesId, StoreError,
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

pub(crate) struct Created(pub Resource);
pub(crate) struct Fetched(pub Resource);

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
