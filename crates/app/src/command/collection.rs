use oeuvre_core::{Collection, CollectionId, CollectionTreeNode, NonEmptyStr, StoreError};

use crate::event::{self, Event};
use crate::{AppState, CommandError, Handler, Outcome};

#[derive(Debug, Clone)]
pub struct CreateCollection {
    pub name: NonEmptyStr,
    pub parent_id: Option<CollectionId>,
}

#[derive(Debug, Clone)]
pub struct ListCollectionForest;

#[derive(Debug, Clone)]
pub struct AddCollectionParent {
    pub child_id: CollectionId,
    pub parent_id: CollectionId,
}

#[derive(Debug, Clone)]
pub struct RemoveCollectionParent {
    pub child_id: CollectionId,
    pub parent_id: CollectionId,
}

#[derive(Debug, Clone)]
pub struct RenameCollection {
    pub id: CollectionId,
    pub name: NonEmptyStr,
}

#[derive(Debug, Clone)]
pub struct DeleteCollection {
    pub id: CollectionId,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeletedCollection {
    pub id: CollectionId,
}

pub(crate) struct Created(pub Collection);
pub(crate) struct Renamed(pub Collection);
pub(crate) struct Forest(pub Vec<CollectionTreeNode>);
pub(crate) struct ParentAdded;
pub(crate) struct ParentRemoved;
pub(crate) struct Deleted(pub DeletedCollection);

impl From<Created> for Outcome {
    fn from(Created(collection): Created) -> Self {
        Outcome::CreateCollection(collection)
    }
}

impl From<Renamed> for Outcome {
    fn from(Renamed(collection): Renamed) -> Self {
        Outcome::RenameCollection(collection)
    }
}

impl From<Forest> for Outcome {
    fn from(Forest(nodes): Forest) -> Self {
        Outcome::ListCollectionForest(nodes)
    }
}

impl From<ParentAdded> for Outcome {
    fn from(_: ParentAdded) -> Self {
        Outcome::AddCollectionParent(())
    }
}

impl From<ParentRemoved> for Outcome {
    fn from(_: ParentRemoved) -> Self {
        Outcome::RemoveCollectionParent(())
    }
}

impl From<Deleted> for Outcome {
    fn from(Deleted(deleted): Deleted) -> Self {
        Outcome::DeleteCollection(deleted)
    }
}

fn map_create(err: StoreError) -> CommandError {
    if err.is_invalid() {
        CommandError::Invalid(err.message.clone())
    } else if err.is_not_found() {
        CommandError::NotFound(err.message.clone())
    } else {
        CommandError::Store(err)
    }
}

#[async_trait::async_trait]
impl Handler for CreateCollection {
    const NAME: &'static str = "create_collection";
    type Output = Created;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::COLLECTION_NAME, self.name.as_str().to_owned());
        if let Some(parent_id) = self.parent_id {
            ev.set(event::COLLECTION_PARENT_ID, parent_id.to_string());
        }
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Created, CommandError> {
        let collection = state
            .store()
            .create_collection(self.name, self.parent_id)
            .await
            .map_err(map_create)?;
        ev.set(event::COLLECTION_ID, collection.id.to_string());
        ev.set(event::COLLECTION_NAME, collection.name.as_str().to_owned());
        Ok(Created(collection))
    }
}

#[async_trait::async_trait]
impl Handler for RenameCollection {
    const NAME: &'static str = "rename_collection";
    type Output = Renamed;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::COLLECTION_ID, self.id.to_string());
        ev.set(event::COLLECTION_NAME, self.name.as_str().to_owned());
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Renamed, CommandError> {
        let collection = state
            .store()
            .rename_collection(self.id, self.name)
            .await
            .map_err(map_create)?;
        ev.set(event::COLLECTION_ID, collection.id.to_string());
        ev.set(event::COLLECTION_NAME, collection.name.as_str().to_owned());
        Ok(Renamed(collection))
    }
}

#[async_trait::async_trait]
impl Handler for ListCollectionForest {
    const NAME: &'static str = "list_collection_forest";
    type Output = Forest;

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Forest, CommandError> {
        let forest = state.store().list_collection_forest().await?;
        ev.set(event::COLLECTION_FOREST_ROOTS, forest.len() as i64);
        Ok(Forest(forest))
    }
}

#[async_trait::async_trait]
impl Handler for AddCollectionParent {
    const NAME: &'static str = "add_collection_parent";
    type Output = ParentAdded;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::COLLECTION_ID, self.child_id.to_string());
        ev.set(event::COLLECTION_PARENT_ID, self.parent_id.to_string());
    }

    async fn run(self, state: &AppState, _ev: &mut Event) -> Result<ParentAdded, CommandError> {
        state
            .store()
            .add_collection_parent(self.child_id, self.parent_id)
            .await
            .map_err(map_create)?;
        Ok(ParentAdded)
    }
}

#[async_trait::async_trait]
impl Handler for RemoveCollectionParent {
    const NAME: &'static str = "remove_collection_parent";
    type Output = ParentRemoved;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::COLLECTION_ID, self.child_id.to_string());
        ev.set(event::COLLECTION_PARENT_ID, self.parent_id.to_string());
    }

    async fn run(self, state: &AppState, _ev: &mut Event) -> Result<ParentRemoved, CommandError> {
        state
            .store()
            .remove_collection_parent(self.child_id, self.parent_id)
            .await
            .map_err(map_create)?;
        Ok(ParentRemoved)
    }
}

#[async_trait::async_trait]
impl Handler for DeleteCollection {
    const NAME: &'static str = "delete_collection";
    type Output = Deleted;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::COLLECTION_ID, self.id.to_string());
    }

    async fn run(self, state: &AppState, _ev: &mut Event) -> Result<Deleted, CommandError> {
        state
            .store()
            .delete_collection(self.id)
            .await
            .map_err(map_create)?;
        Ok(Deleted(DeletedCollection { id: self.id }))
    }
}
