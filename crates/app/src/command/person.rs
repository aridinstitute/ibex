use oeuvre_core::{Name, Person, PersonId, StoreError};

use crate::event::{self, Event};
use crate::{AppState, CommandError, Handler, Outcome};

#[derive(Debug, Clone)]
pub struct CreatePerson {
    pub name: Name,
    pub aliases: Vec<Name>,
}

#[derive(Debug, Clone)]
pub struct GetPerson {
    pub id: PersonId,
}

#[derive(Debug, Clone)]
pub struct UpdatePerson {
    pub id: PersonId,
    pub name: Name,
    pub aliases: Vec<Name>,
}

#[derive(Debug, Clone)]
pub struct DeletePerson {
    pub id: PersonId,
}

#[derive(Debug, Clone)]
pub struct FindPersonsByName {
    pub name: Name,
}

#[derive(Debug, Clone)]
pub struct ListPersons;

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeletedPerson {
    pub id: PersonId,
}

pub(crate) struct Created(pub Person);
pub(crate) struct Fetched(pub Person);
pub(crate) struct Updated(pub Person);
pub(crate) struct Found(pub Vec<Person>);
pub(crate) struct Listed(pub Vec<Person>);

impl From<Created> for Outcome {
    fn from(Created(person): Created) -> Self {
        Outcome::CreatePerson(person)
    }
}

impl From<Fetched> for Outcome {
    fn from(Fetched(person): Fetched) -> Self {
        Outcome::GetPerson(person)
    }
}

impl From<Updated> for Outcome {
    fn from(Updated(person): Updated) -> Self {
        Outcome::UpdatePerson(person)
    }
}

impl From<DeletedPerson> for Outcome {
    fn from(d: DeletedPerson) -> Self {
        Outcome::DeletePerson(d)
    }
}

impl From<Found> for Outcome {
    fn from(Found(people): Found) -> Self {
        Outcome::FindPersonsByName(people)
    }
}

impl From<Listed> for Outcome {
    fn from(Listed(people): Listed) -> Self {
        Outcome::ListPersons(people)
    }
}

fn map_store_err(err: StoreError, id: PersonId) -> CommandError {
    if err.is_not_found() {
        CommandError::NotFound(id.to_string())
    } else if err.is_invalid() {
        CommandError::Invalid(err.message.clone())
    } else {
        CommandError::Store(err)
    }
}

fn map_create_err(err: StoreError) -> CommandError {
    if err.is_invalid() {
        CommandError::Invalid(err.message.clone())
    } else {
        CommandError::Store(err)
    }
}

#[async_trait::async_trait]
impl Handler for CreatePerson {
    const NAME: &'static str = "create_person";
    type Output = Created;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::PERSON_DISPLAY, self.name.display());
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Created, CommandError> {
        let person = state
            .store()
            .create_person(self.name, self.aliases)
            .await
            .map_err(map_create_err)?;
        ev.set(event::PERSON_ID, person.id.to_string());
        ev.set(event::PERSON_DISPLAY, person.name.display());
        Ok(Created(person))
    }
}

#[async_trait::async_trait]
impl Handler for GetPerson {
    const NAME: &'static str = "get_person";
    type Output = Fetched;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::PERSON_ID, self.id.to_string());
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Fetched, CommandError> {
        let person = state
            .store()
            .get_person(self.id)
            .await
            .map_err(|e| map_store_err(e, self.id))?;
        ev.set(event::PERSON_DISPLAY, person.name.display());
        Ok(Fetched(person))
    }
}

#[async_trait::async_trait]
impl Handler for UpdatePerson {
    const NAME: &'static str = "update_person";
    type Output = Updated;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::PERSON_ID, self.id.to_string());
        ev.set(event::PERSON_DISPLAY, self.name.display());
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Updated, CommandError> {
        let person = state
            .store()
            .update_person(self.id, self.name, self.aliases)
            .await
            .map_err(|e| map_store_err(e, self.id))?;
        ev.set(event::PERSON_DISPLAY, person.name.display());
        Ok(Updated(person))
    }
}

#[async_trait::async_trait]
impl Handler for DeletePerson {
    const NAME: &'static str = "delete_person";
    type Output = DeletedPerson;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::PERSON_ID, self.id.to_string());
    }

    async fn run(
        self,
        state: &AppState,
        _ev: &mut Event,
    ) -> Result<DeletedPerson, CommandError> {
        state
            .store()
            .delete_person(self.id)
            .await
            .map_err(|e| map_store_err(e, self.id))?;
        Ok(DeletedPerson { id: self.id })
    }
}

#[async_trait::async_trait]
impl Handler for FindPersonsByName {
    const NAME: &'static str = "find_persons_by_name";
    type Output = Found;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::PERSON_DISPLAY, self.name.display());
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Found, CommandError> {
        let people = state.store().find_persons_by_name(&self.name).await?;
        ev.set(event::PERSON_DISPLAY, self.name.display());
        Ok(Found(people))
    }
}

#[async_trait::async_trait]
impl Handler for ListPersons {
    const NAME: &'static str = "list_persons";
    type Output = Listed;

    async fn run(self, state: &AppState, _ev: &mut Event) -> Result<Listed, CommandError> {
        let people = state.store().list_persons().await?;
        Ok(Listed(people))
    }
}
