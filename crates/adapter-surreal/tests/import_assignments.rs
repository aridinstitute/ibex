//! Batch collection/tag assignment through the real Surreal adapter.

use ibex_adapter_surreal::SurrealStore;
use ibex_app::command::{
    CreateCollection, GetResource, ImportBiblatex, ListTags, SetResourceFavorite,
};
use ibex_app::{AppState, Command, Outcome, Surface};
use ibex_core::{
    ExistingIdOrName, ImportAssignments, ImportChoices, ImportReport, NonEmptyStr, Resource,
    ResourceId,
};

fn name(s: &str) -> NonEmptyStr {
    NonEmptyStr::try_new(s).unwrap()
}

async fn import(state: &AppState, source: &str, choices: ImportChoices) -> ImportReport {
    let Outcome::ImportBiblatex(report) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: source.into(),
            choices,
        }))
        .await
        .unwrap()
    else {
        panic!("import");
    };
    report
}

async fn resource(state: &AppState, id: ResourceId) -> Resource {
    let Outcome::GetResource(r) = state
        .execute(Command::GetResource(GetResource { id }))
        .await
        .unwrap()
    else {
        panic!("get");
    };
    r
}

#[tokio::test]
async fn batch_targets_persist_and_retries_stay_additive_through_surreal() {
    let state = AppState::builder(SurrealStore::open_memory().await.unwrap())
        .surface(Surface::Test)
        .build();
    let Outcome::CreateCollection(methods) = state
        .execute(Command::CreateCollection(CreateCollection {
            name: name("Methods"),
            parent_id: None,
        }))
        .await
        .unwrap()
    else {
        panic!("collection");
    };
    let source = r#"
        @book{a, title={First}, author={Ada, Lovelace}, year={1843}}
        @article{b, title={Second}, author={Hopper, Grace}, journaltitle={Journal}, year={1952}}
    "#;
    let assign = |tags: &[&str]| ImportChoices {
        assignments: ImportAssignments {
            collections: vec![
                ExistingIdOrName::Name {
                    name: name("Research"),
                },
                ExistingIdOrName::Existing { id: methods.id },
            ],
            tags: tags
                .iter()
                .map(|t| ExistingIdOrName::Name { name: name(t) })
                .collect(),
        },
        ..Default::default()
    };

    let first = import(&state, source, assign(&["review"])).await;
    assert_eq!(first.imported.len(), 2, "{first:?}");
    for entry in &first.imported {
        let r = resource(&state, entry.resource_id).await;
        assert_eq!(r.tag_ids.len(), 1, "{r:?}");
        assert_eq!(r.collection_ids.len(), 2, "{r:?}");
        assert!(r.collection_ids.contains(&methods.id));
    }
    let article = resource(&state, first.imported[1].resource_id).await;
    let journal = resource(&state, article.parent_id.expect("journal")).await;
    assert!(journal.tag_ids.is_empty() && journal.collection_ids.is_empty());

    let id = first.imported[0].resource_id;
    state
        .execute(Command::SetResourceFavorite(SetResourceFavorite {
            id,
            favorite: true,
        }))
        .await
        .unwrap();
    let before = resource(&state, id).await;

    for _ in 0..2 {
        let retry = import(&state, source, assign(&["review", "2026"])).await;
        assert_eq!(retry.reused.len(), 2, "{retry:?}");
        assert!(retry.imported.is_empty(), "{retry:?}");
    }
    let after = resource(&state, id).await;
    assert_eq!(after.tag_ids.len(), 2, "{after:?}");
    assert_eq!(after.tag_ids[0], before.tag_ids[0], "existing kept first");
    assert_eq!(after.collection_ids, before.collection_ids);
    assert!(after.favorite);
    assert_eq!(after.date_edited, before.date_edited);
    assert_eq!(after.title, before.title);
    assert_eq!(after.creators, before.creators);

    let Outcome::ListTags(tags) = state.execute(Command::ListTags(ListTags)).await.unwrap() else {
        panic!("tags");
    };
    assert_eq!(tags.len(), 2, "{tags:?}");
}
