#![cfg(feature = "testing")]

use oeuvre_app::command::{CreateCollection, ListCollectionForest};
use oeuvre_app::testing::MemoryStore;
use oeuvre_app::{AppState, Command, Outcome, Surface};
use oeuvre_core::NonEmptyStr;

#[tokio::test]
async fn create_root_and_nested_collections_list_as_forest() {
    let store = MemoryStore::new();
    let state = AppState::builder(store).surface(Surface::Test).build();

    let Outcome::CreateCollection(root) = state
        .execute(Command::CreateCollection(CreateCollection {
            name: NonEmptyStr::try_new("Research").unwrap(),
            parent_id: None,
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateCollection");
    };
    assert_eq!(root.name.as_str(), "Research");

    let Outcome::CreateCollection(child) = state
        .execute(Command::CreateCollection(CreateCollection {
            name: NonEmptyStr::try_new("Methods").unwrap(),
            parent_id: Some(root.id),
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateCollection");
    };
    assert_eq!(child.name.as_str(), "Methods");

    let Outcome::ListCollectionForest(forest) = state
        .execute(Command::ListCollectionForest(ListCollectionForest))
        .await
        .unwrap()
    else {
        panic!("expected ListCollectionForest");
    };

    assert_eq!(forest.len(), 1);
    assert_eq!(forest[0].collection.id, root.id);
    assert_eq!(forest[0].children.len(), 1);
    assert_eq!(forest[0].children[0].collection.id, child.id);
    assert!(forest[0].children[0].children.is_empty());
}
