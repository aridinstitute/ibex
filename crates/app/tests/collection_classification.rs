#![cfg(feature = "testing")]

use oeuvre_app::command::{
    CreateCollection, CreateResource, GetResourceDetail, RenameCollection,
    SetCollectionClassification,
};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, CommandError, FieldValue, Outcome, Status, Surface};
use oeuvre_core::{
    Collection, CollectionId, NewResource, NonEmptyStr, ResourceKind, keyword_suggestions,
};

fn state() -> (AppState, MemorySink) {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();
    (state, sink)
}

async fn create(state: &AppState, name: &str) -> Collection {
    let Outcome::CreateCollection(c) = state
        .execute(Command::CreateCollection(CreateCollection {
            name: NonEmptyStr::try_new(name).unwrap(),
            parent_id: None,
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateCollection");
    };
    c
}

async fn classify(
    state: &AppState,
    id: CollectionId,
    keywords: &[&str],
    jev_enabled: bool,
) -> Result<Outcome, CommandError> {
    state
        .execute(Command::SetCollectionClassification(
            SetCollectionClassification {
                id,
                keywords: keywords.iter().map(|k| k.to_string()).collect(),
                jev_enabled,
            },
        ))
        .await
}

#[tokio::test]
async fn set_classification_round_trips_and_normalizes() {
    let (state, _) = state();
    let methods = create(&state, "Methods").await;
    let Outcome::SetCollectionClassification(updated) = classify(
        &state,
        methods.id,
        &[" research methods ", "Provenance", "provenance"],
        true,
    )
    .await
    .unwrap() else {
        panic!("expected SetCollectionClassification");
    };
    assert_eq!(updated.keywords, vec!["research methods", "Provenance"]);
    assert!(updated.jev_enabled);

    let Outcome::RenameCollection(renamed) = state
        .execute(Command::RenameCollection(RenameCollection {
            id: methods.id,
            name: NonEmptyStr::try_new("Method").unwrap(),
        }))
        .await
        .unwrap()
    else {
        panic!("expected RenameCollection");
    };
    assert_eq!(renamed.keywords, updated.keywords);
    assert!(renamed.jev_enabled);
}

#[tokio::test]
async fn set_classification_rejects_invalid_keywords_and_missing_ids() {
    let (state, sink) = state();
    let methods = create(&state, "Methods").await;
    let err = classify(&state, methods.id, &["ok", "  "], false)
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Invalid(_)));
    let long = "x".repeat(200);
    let err = classify(&state, methods.id, &[&long], false)
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Invalid(_)));
    let err = classify(&state, CollectionId::generate(), &["ok"], false)
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::NotFound(_)));
    let events = sink.take();
    assert!(events.iter().skip(1).all(|e| e.status() == Status::Error));
}

#[tokio::test]
async fn set_classification_emits_one_annotated_event() {
    let (state, sink) = state();
    let methods = create(&state, "Methods").await;
    sink.take();
    classify(&state, methods.id, &["a", "b"], true)
        .await
        .unwrap();
    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.command(), "set_collection_classification");
    assert_eq!(ev.status(), Status::Ok);
    assert_eq!(
        ev.get("collection.id"),
        Some(&FieldValue::from(methods.id.to_string()))
    );
    assert_eq!(
        ev.get("collection.keyword_count"),
        Some(&FieldValue::from(2usize))
    );
    assert_eq!(
        ev.get("collection.jev_enabled"),
        Some(&FieldValue::from(true))
    );
}

#[tokio::test]
async fn update_collection_is_one_command_and_rolls_back_later_parent_failure() {
    use oeuvre_app::command::UpdateCollection;
    use oeuvre_core::CollectionUpdate;
    let (state, sink) = state();
    let a = create(&state, "A").await;
    let b = create(&state, "B").await;
    let child = create(&state, "Child").await;
    sink.take();
    let patch = CollectionUpdate {
        id: child.id,
        name: Some(NonEmptyStr::try_new("Changed").unwrap()),
        keywords: Some(vec![" Keyword ".into(), "keyword".into()]),
        jev_enabled: Some(true),
        add_parents: vec![a.id, b.id],
        remove_parents: vec![],
    };
    let Outcome::UpdateCollection(result) = state
        .execute(Command::UpdateCollection(UpdateCollection(patch.clone())))
        .await
        .unwrap()
    else {
        panic!("update result")
    };
    assert_eq!(result.keywords, vec!["Keyword"]);
    assert!(result.jev_enabled);
    let events = sink.take();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].command(), "update_collection");
    let mut failed = patch;
    failed.name = Some(NonEmptyStr::try_new("MustNotPersist").unwrap());
    failed.remove_parents = vec![a.id];
    failed.add_parents = vec![a.id, CollectionId::generate()];
    assert!(
        state
            .execute(Command::UpdateCollection(UpdateCollection(failed)))
            .await
            .is_err()
    );
    let events = sink.take();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status(), Status::Error);
    let Outcome::ListCollectionForest(forest) = state
        .execute(Command::ListCollectionForest(
            oeuvre_app::command::ListCollectionForest,
        ))
        .await
        .unwrap()
    else {
        panic!("forest")
    };
    for parent in [a.id, b.id] {
        let node = forest
            .iter()
            .find(|node| node.collection.id == parent)
            .unwrap();
        assert_eq!(node.children.len(), 1);
        assert_eq!(node.children[0].collection.name.as_str(), "Changed");
    }
}

#[tokio::test]
async fn detail_resolves_collections_in_membership_order() {
    let (state, _) = state();
    let a = create(&state, "Alpha").await;
    let b = create(&state, "Beta").await;
    let Outcome::SetCollectionClassification(b) =
        classify(&state, b.id, &["citation networks"], false)
            .await
            .unwrap()
    else {
        panic!("expected SetCollectionClassification");
    };
    let Outcome::CreateResource(resource) = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                collection_ids: vec![b.id, a.id],
                abstract_text: Some("Mapping citation networks.".into()),
                ..book("Linked")
            },
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };
    let Outcome::GetResourceDetail(detail) = state
        .execute(Command::GetResourceDetail(GetResourceDetail {
            id: resource.id,
        }))
        .await
        .unwrap()
    else {
        panic!("expected GetResourceDetail");
    };
    let names: Vec<&str> = detail.collections.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["Beta", "Alpha"]);
    assert_eq!(keyword_suggestions(&detail, &[a, b.clone()]), vec![b.id]);
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
        subtitle: None,
        title_addon: None,
        short_title: None,
        place: None,
        volume: None,
        issue: None,
        pages: None,
        total_pages: None,
        article_number: None,
        edition: None,
        doi: None,
        url: None,
        eprint: None,
        isbns: vec![],
        issns: vec![],
        isrns: vec![],
        original_date: None,
        event_date: None,
        language: None,
        document_type: None,
        publication_method: None,
        institution: None,
        organization: None,
        school: None,
        abstract_text: None,
        keywords: vec![],
        metadata: Default::default(),
    }
}
