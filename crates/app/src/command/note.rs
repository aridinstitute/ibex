use crate::event::{self, Event};
use crate::{AppState, CommandError, Handler, Outcome};
use ibex_core::{NoteContent, NoteId, ResourceId, StoreError};

#[derive(Debug, Clone)]
pub struct CreateNote {
    pub resource_id: ResourceId,
    pub content: NoteContent,
}
#[derive(Debug, Clone)]
pub struct UpdateNote {
    pub id: NoteId,
    pub content: NoteContent,
}
#[derive(Debug, Clone)]
pub struct DeleteNote {
    pub id: NoteId,
}
#[derive(Debug, Clone)]
pub struct ListNotes {
    pub resource_id: ResourceId,
}

fn map_error(e: StoreError) -> CommandError {
    if e.is_not_found() {
        CommandError::NotFound(e.message)
    } else if e.is_invalid() {
        CommandError::Invalid(e.message)
    } else {
        CommandError::Store(e)
    }
}

#[async_trait::async_trait]
impl Handler for CreateNote {
    const NAME: &'static str = "create_note";
    type Output = Outcome;
    fn annotate(&self, ev: &mut Event) {
        ev.set(event::RESOURCE_ID, self.resource_id.to_string());
        ev.set(event::NOTE_KIND, self.content.kind.label());
        ev.set(event::NOTE_LOCATION_COUNT, self.content.locations.len());
    }
    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Outcome, CommandError> {
        let note = state
            .store()
            .create_note(self.resource_id, self.content)
            .await
            .map_err(map_error)?;
        ev.set(event::NOTE_ID, note.id.to_string());
        Ok(Outcome::CreateNote(note))
    }
}
#[async_trait::async_trait]
impl Handler for UpdateNote {
    const NAME: &'static str = "update_note";
    type Output = Outcome;
    fn annotate(&self, ev: &mut Event) {
        ev.set(event::NOTE_ID, self.id.to_string());
        ev.set(event::NOTE_KIND, self.content.kind.label());
        ev.set(event::NOTE_LOCATION_COUNT, self.content.locations.len());
    }
    async fn run(self, state: &AppState, _: &mut Event) -> Result<Outcome, CommandError> {
        Ok(Outcome::UpdateNote(
            state
                .store()
                .update_note(self.id, self.content)
                .await
                .map_err(map_error)?,
        ))
    }
}
#[async_trait::async_trait]
impl Handler for DeleteNote {
    const NAME: &'static str = "delete_note";
    type Output = Outcome;
    fn annotate(&self, ev: &mut Event) {
        ev.set(event::NOTE_ID, self.id.to_string());
    }
    async fn run(self, state: &AppState, _: &mut Event) -> Result<Outcome, CommandError> {
        state
            .store()
            .delete_note(self.id)
            .await
            .map_err(map_error)?;
        Ok(Outcome::DeleteNote(()))
    }
}
#[async_trait::async_trait]
impl Handler for ListNotes {
    const NAME: &'static str = "list_notes";
    type Output = Outcome;
    fn annotate(&self, ev: &mut Event) {
        ev.set(event::RESOURCE_ID, self.resource_id.to_string());
    }
    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Outcome, CommandError> {
        let notes = state
            .store()
            .list_notes(self.resource_id)
            .await
            .map_err(map_error)?;
        ev.set(event::NOTE_COUNT, notes.len());
        Ok(Outcome::ListNotes(notes))
    }
}
