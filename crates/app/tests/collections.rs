#![cfg(feature = "testing")]

use oeuvre_app::command::{
    AddCollectionParent, CreateCollection, CreateResource, DeleteCollection, GetResource,
    ListCollectionForest, ListResourcesQueryPage,
};
use oeuvre_app::testing::MemoryStore;
use oeuvre_app::{AppState, Command, Outcome, Surface};
use oeuvre_core::{
    NewResource, NonEmptyStr, ResourceFilter, ResourceKind, ResourceQuery, Store,
};

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

fn book(title: &str) -> NewResource {
    NewResource {
        kind: ResourceKind::Book,
        title: NonEmptyStr::try_new(title).unwrap(),
        creators: vec![],
        publisher: None,
        published: None,
        accessed: None,
        parent_id: None,
        series_id: None,
        series_number: None,
        tag_ids: vec![],
        collection_ids: vec![],
    }
}

#[tokio::test]
async fn delete_cascades_exclusive_children_but_preserves_shared() {
    let store = MemoryStore::new();
    let state = AppState::builder(store).surface(Surface::Test).build();

    let Outcome::CreateCollection(alpha) = state
        .execute(Command::CreateCollection(CreateCollection {
            name: NonEmptyStr::try_new("Alpha").unwrap(),
            parent_id: None,
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateCollection");
    };
    let Outcome::CreateCollection(beta) = state
        .execute(Command::CreateCollection(CreateCollection {
            name: NonEmptyStr::try_new("Beta").unwrap(),
            parent_id: None,
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateCollection");
    };
    let Outcome::CreateCollection(shared) = state
        .execute(Command::CreateCollection(CreateCollection {
            name: NonEmptyStr::try_new("Shared").unwrap(),
            parent_id: Some(alpha.id),
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateCollection");
    };
    let Outcome::CreateCollection(exclusive) = state
        .execute(Command::CreateCollection(CreateCollection {
            name: NonEmptyStr::try_new("Exclusive").unwrap(),
            parent_id: Some(alpha.id),
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateCollection");
    };
    state
        .execute(Command::AddCollectionParent(AddCollectionParent {
            child_id: shared.id,
            parent_id: beta.id,
        }))
        .await
        .unwrap();

    state
        .execute(Command::DeleteCollection(DeleteCollection { id: alpha.id }))
        .await
        .unwrap();

    let Outcome::ListCollectionForest(forest) = state
        .execute(Command::ListCollectionForest(ListCollectionForest))
        .await
        .unwrap()
    else {
        panic!("expected ListCollectionForest");
    };

    assert_eq!(forest.len(), 1);
    assert_eq!(forest[0].collection.id, beta.id);
    assert_eq!(forest[0].children.len(), 1);
    assert_eq!(forest[0].children[0].collection.id, shared.id);
    // Exclusive child of alpha is gone; shared remains under beta.
    assert!(
        !forest
            .iter()
            .flat_map(|n| std::iter::once(n.collection.id).chain(
                n.children.iter().map(|c| c.collection.id)
            ))
            .any(|id| id == exclusive.id || id == alpha.id)
    );
}

#[tokio::test]
async fn delete_collection_clears_membership_without_deleting_resources() {
    let store = MemoryStore::new();
    let collection = store
        .create_collection(NonEmptyStr::try_new("Shelf").unwrap(), None)
        .await
        .unwrap();
    let state = AppState::builder(store).surface(Surface::Test).build();

    let mut new = book("Kept Book");
    new.collection_ids = vec![collection.id];
    let Outcome::CreateResource(resource) = state
        .execute(Command::CreateResource(CreateResource { new }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };
    assert_eq!(resource.collection_ids, vec![collection.id]);

    state
        .execute(Command::DeleteCollection(DeleteCollection {
            id: collection.id,
        }))
        .await
        .unwrap();

    let Outcome::GetResource(kept) = state
        .execute(Command::GetResource(GetResource { id: resource.id }))
        .await
        .unwrap()
    else {
        panic!("expected GetResource");
    };
    assert_eq!(kept.id, resource.id);
    assert!(kept.collection_ids.is_empty());
}

#[tokio::test]
async fn collection_filter_includes_descendant_membership() {
    let store = MemoryStore::new();
    let state = AppState::builder(store).surface(Surface::Test).build();

    let Outcome::CreateCollection(root) = state
        .execute(Command::CreateCollection(CreateCollection {
            name: NonEmptyStr::try_new("Root").unwrap(),
            parent_id: None,
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateCollection");
    };
    let Outcome::CreateCollection(child) = state
        .execute(Command::CreateCollection(CreateCollection {
            name: NonEmptyStr::try_new("Child").unwrap(),
            parent_id: Some(root.id),
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateCollection");
    };

    let mut nested = book("In Child");
    nested.collection_ids = vec![child.id];
    let Outcome::CreateResource(in_child) = state
        .execute(Command::CreateResource(CreateResource { new: nested }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };
    state
        .execute(Command::CreateResource(CreateResource {
            new: book("Elsewhere"),
        }))
        .await
        .unwrap();

    let Outcome::ListResourcesQueryPage(page) = state
        .execute(Command::ListResourcesQueryPage(ListResourcesQueryPage {
            query: ResourceQuery {
                sort: None,
                filter: ResourceFilter {
                    collection: Some(root.id),
                    ..ResourceFilter::none()
                },
            },
            offset: 0,
            limit: 10,
        }))
        .await
        .unwrap()
    else {
        panic!("expected query page");
    };
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].id, in_child.id);
}
