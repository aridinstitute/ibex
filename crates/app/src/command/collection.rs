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

pub(crate) struct Created(pub Collection);
pub(crate) struct Forest(pub Vec<CollectionTreeNode>);

impl From<Created> for Outcome {
    fn from(Created(collection): Created) -> Self {
        Outcome::CreateCollection(collection)
    }
}

impl From<Forest> for Outcome {
    fn from(Forest(nodes): Forest) -> Self {
        Outcome::ListCollectionForest(nodes)
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
impl Handler for ListCollectionForest {
    const NAME: &'static str = "list_collection_forest";
    type Output = Forest;

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Forest, CommandError> {
        let forest = state.store().list_collection_forest().await?;
        ev.set(event::COLLECTION_FOREST_ROOTS, forest.len() as i64);
        Ok(Forest(forest))
    }
}
