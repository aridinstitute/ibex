use oeuvre_core::{NonEmptyStr, StoreError, Tag, TagId};

use crate::event::{self, Event};
use crate::{AppState, CommandError, Handler, Outcome};

#[derive(Debug, Clone)]
pub struct CreateTag {
    pub text: NonEmptyStr,
}

#[derive(Debug, Clone)]
pub struct GetTag {
    pub id: TagId,
}

#[derive(Debug, Clone)]
pub struct FindTagByText {
    pub text: NonEmptyStr,
}

#[derive(Debug, Clone)]
pub struct ListTags;

pub(crate) struct Created(pub Tag);
pub(crate) struct Fetched(pub Tag);
pub(crate) struct Found(pub Option<Tag>);
pub(crate) struct Listed(pub Vec<Tag>);

impl From<Created> for Outcome {
    fn from(Created(tag): Created) -> Self {
        Outcome::CreateTag(tag)
    }
}

impl From<Fetched> for Outcome {
    fn from(Fetched(tag): Fetched) -> Self {
        Outcome::GetTag(tag)
    }
}

impl From<Found> for Outcome {
    fn from(Found(tag): Found) -> Self {
        Outcome::FindTagByText(tag)
    }
}

impl From<Listed> for Outcome {
    fn from(Listed(tags): Listed) -> Self {
        Outcome::ListTags(tags)
    }
}

fn map_create(err: StoreError) -> CommandError {
    if err.is_invalid() {
        CommandError::Invalid(err.message.clone())
    } else {
        CommandError::Store(err)
    }
}

fn map_lookup(err: StoreError, id: TagId) -> CommandError {
    if err.is_not_found() {
        CommandError::NotFound(id.to_string())
    } else if err.is_invalid() {
        CommandError::Invalid(err.message.clone())
    } else {
        CommandError::Store(err)
    }
}

#[async_trait::async_trait]
impl Handler for CreateTag {
    const NAME: &'static str = "create_tag";
    type Output = Created;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::TAG_TEXT, self.text.as_str().to_owned());
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Created, CommandError> {
        let tag = state
            .store()
            .create_tag(self.text)
            .await
            .map_err(map_create)?;
        ev.set(event::TAG_ID, tag.id.to_string());
        ev.set(event::TAG_TEXT, tag.text.as_str().to_owned());
        Ok(Created(tag))
    }
}

#[async_trait::async_trait]
impl Handler for GetTag {
    const NAME: &'static str = "get_tag";
    type Output = Fetched;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::TAG_ID, self.id.to_string());
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Fetched, CommandError> {
        let tag = state
            .store()
            .get_tag(self.id)
            .await
            .map_err(|e| map_lookup(e, self.id))?;
        ev.set(event::TAG_TEXT, tag.text.as_str().to_owned());
        Ok(Fetched(tag))
    }
}

#[async_trait::async_trait]
impl Handler for FindTagByText {
    const NAME: &'static str = "find_tag_by_text";
    type Output = Found;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::TAG_TEXT, self.text.as_str().to_owned());
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Found, CommandError> {
        let tag = state
            .store()
            .find_tag_by_text(&self.text)
            .await
            .map_err(map_create)?;
        if let Some(tag) = &tag {
            ev.set(event::TAG_ID, tag.id.to_string());
        }
        Ok(Found(tag))
    }
}

#[async_trait::async_trait]
impl Handler for ListTags {
    const NAME: &'static str = "list_tags";
    type Output = Listed;

    async fn run(self, state: &AppState, _ev: &mut Event) -> Result<Listed, CommandError> {
        Ok(Listed(state.store().list_tags().await?))
    }
}
