use oeuvre_core::{NewResource, PersonId, Resource, ResourceError, ResourceId, StoreError};

use crate::event::{self, Event};
use crate::{AppState, CommandError, Handler, Outcome};

#[derive(Debug, Clone)]
pub struct CreateResource {
    pub new: NewResource,
}

pub(crate) struct Created(pub Resource);

impl From<Created> for Outcome {
    fn from(Created(resource): Created) -> Self {
        Outcome::CreateResource(resource)
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

fn map_resource_err(err: ResourceError) -> CommandError {
    CommandError::Invalid(err.to_string())
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
        for creator in &self.new.creators {
            state
                .store()
                .get_person(creator.person_id)
                .await
                .map_err(|e| map_person_lookup(e, creator.person_id))?;
        }
        let resource = Resource::try_new(ResourceId::generate(), self.new).map_err(map_resource_err)?;
        ev.set(event::RESOURCE_ID, resource.id.to_string());
        ev.set(event::RESOURCE_KIND, resource.kind.as_str());
        ev.set(event::RESOURCE_TITLE, resource.title.as_str().to_owned());
        Ok(Created(resource))
    }
}
