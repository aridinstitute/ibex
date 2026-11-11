#![cfg(feature = "testing")]

use ibex_app::cluster::{
    SuggestClusters, TextSource, apply_clusters, extract_document_text, suggest_clusters,
};
use ibex_app::command::{
    AddAttachment, CreateCollection, CreateResource, GetResource, GetResourceDetail,
    ListCollectionForest,
};
use ibex_app::testing::MemoryStore;
use ibex_app::{AppState, Command, Outcome, Surface};
use ibex_core::{NewResource, NonEmptyStr, ResourceId, ResourceKind};

fn article(title: &str, subtitle: Option<&str>, abstract_text: &str) -> NewResource {
    NewResource {
        kind: ResourceKind::JournalArticle,
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
        metadata: Default::default(),
        subtitle: subtitle.map(|s| NonEmptyStr::try_new(s).unwrap()),
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
        abstract_text: Some(abstract_text.into()),
        keywords: vec![],
    }
}

fn container(kind: ResourceKind, title: &str) -> NewResource {
    let mut new = article(title, None, "container row");
    new.kind = kind;
    new.abstract_text = None;
    new
}

async fn add(state: &AppState, new: NewResource) -> ResourceId {
    let Outcome::CreateResource(resource) = state
        .execute(Command::CreateResource(CreateResource { new }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };
    resource.id
}

async fn seed_two_topics(state: &AppState) {
    for (title, subtitle, body) in [
        (
            "Neural nets",
            Some("Deep architectures"),
            "deep learning neural networks representation learning backpropagation layers",
        ),
        (
            "CNN survey",
            None,
            "convolutional neural networks deep learning image recognition layers",
        ),
        (
            "Transformers",
            None,
            "transformer attention neural networks deep learning language models",
        ),
        (
            "Carbon sinks",
            Some("Forest carbon"),
            "climate change carbon sequestration forests greenhouse emissions",
        ),
        (
            "Warming trends",
            None,
            "climate change global warming temperature greenhouse emissions",
        ),
        (
            "Ocean acid",
            None,
            "climate change ocean acidification carbon dioxide greenhouse emissions",
        ),
    ] {
        add(state, article(title, subtitle, body)).await;
    }
}

#[tokio::test]
async fn suggest_and_apply_creates_topic_collections() {
    let store = MemoryStore::new();
    let state = AppState::builder(store).surface(Surface::Test).build();
    seed_two_topics(&state).await;

    let suggestion = suggest_clusters(
        &state,
        SuggestClusters {
            min_topic_size: 3,
            ..SuggestClusters::default()
        },
    )
    .await
    .unwrap();
    assert!(
        suggestion.report.topics.len() >= 2,
        "topics: {:?}",
        suggestion
            .report
            .topics
            .iter()
            .map(|t| (&t.label, t.members.len()))
            .collect::<Vec<_>>()
    );

    let applied = apply_clusters(&state, &suggestion, None).await.unwrap();
    assert_eq!(applied.len(), suggestion.report.topics.len());

    let Outcome::ListCollectionForest(forest) = state
        .execute(Command::ListCollectionForest(ListCollectionForest))
        .await
        .unwrap()
    else {
        panic!("expected forest");
    };
    assert_eq!(forest.len(), applied.len());

    for topic in &applied {
        for resource_id in &topic.resource_ids {
            let Outcome::GetResource(resource) = state
                .execute(Command::GetResource(GetResource { id: *resource_id }))
                .await
                .unwrap()
            else {
                panic!("expected resource");
            };
            assert!(
                resource.collection_ids.contains(&topic.collection.id),
                "resource {resource_id} missing collection {}",
                topic.collection.id
            );
        }
        assert!(!topic.collection.keywords.is_empty());
    }
}

#[tokio::test]
async fn suggest_is_read_only() {
    let store = MemoryStore::new();
    let state = AppState::builder(store).surface(Surface::Test).build();
    seed_two_topics(&state).await;

    let before = state
        .execute(Command::ListCollectionForest(ListCollectionForest))
        .await
        .unwrap();
    let Outcome::ListCollectionForest(before_forest) = before else {
        panic!("expected forest");
    };
    assert!(before_forest.is_empty());

    let suggestion = suggest_clusters(
        &state,
        SuggestClusters {
            min_topic_size: 3,
            ..SuggestClusters::default()
        },
    )
    .await
    .unwrap();
    assert!(!suggestion.report.topics.is_empty());

    let Outcome::ListCollectionForest(after) = state
        .execute(Command::ListCollectionForest(ListCollectionForest))
        .await
        .unwrap()
    else {
        panic!("expected forest");
    };
    assert!(after.is_empty(), "preview must not create collections");
}

#[tokio::test]
async fn skips_containers_and_counts_them() {
    let store = MemoryStore::new();
    let state = AppState::builder(store).surface(Surface::Test).build();
    seed_two_topics(&state).await;
    add(&state, container(ResourceKind::Journal, "Nature")).await;
    add(&state, container(ResourceKind::Podcast, "Lab Talk")).await;

    let suggestion = suggest_clusters(
        &state,
        SuggestClusters {
            min_topic_size: 3,
            ..SuggestClusters::default()
        },
    )
    .await
    .unwrap();
    assert!(
        suggestion.skipped_containers >= 2,
        "skipped={}",
        suggestion.skipped_containers
    );
    assert_eq!(suggestion.documents, 6);
}

#[tokio::test]
async fn prefers_attachment_text_over_abstract() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new();
    let state = AppState::builder(store).surface(Surface::Test).build();

    let id = add(
        &state,
        article(
            "Misleading title",
            None,
            "abstract about completely unrelated zebra taxonomy nonsense filler words",
        ),
    )
    .await;
    let path = dir.path().join("body.txt");
    std::fs::write(
        &path,
        "deep learning neural networks representation learning backpropagation layers",
    )
    .unwrap();
    let Outcome::AddAttachment(_) = state
        .execute(Command::AddAttachment(AddAttachment {
            resource_id: id,
            file_path: path,
        }))
        .await
        .unwrap()
    else {
        panic!("expected AddAttachment");
    };

    let Outcome::GetResourceDetail(detail) = state
        .execute(Command::GetResourceDetail(GetResourceDetail { id }))
        .await
        .unwrap()
    else {
        panic!("expected detail");
    };
    let text = extract_document_text(&detail);
    assert_eq!(text.source, TextSource::Attachment);
    assert!(text.body.contains("neural"));
    assert!(!text.body.contains("zebra"));
}

#[tokio::test]
async fn apply_nests_under_parent_collection() {
    let store = MemoryStore::new();
    let state = AppState::builder(store).surface(Surface::Test).build();
    seed_two_topics(&state).await;

    let Outcome::CreateCollection(parent) = state
        .execute(Command::CreateCollection(CreateCollection {
            name: NonEmptyStr::try_new("Auto topics").unwrap(),
            parent_id: None,
        }))
        .await
        .unwrap()
    else {
        panic!("expected parent");
    };

    let suggestion = suggest_clusters(
        &state,
        SuggestClusters {
            min_topic_size: 3,
            ..SuggestClusters::default()
        },
    )
    .await
    .unwrap();
    let applied = apply_clusters(&state, &suggestion, Some(parent.id))
        .await
        .unwrap();
    assert!(!applied.is_empty());

    let Outcome::ListCollectionForest(forest) = state
        .execute(Command::ListCollectionForest(ListCollectionForest))
        .await
        .unwrap()
    else {
        panic!("expected forest");
    };
    assert_eq!(forest.len(), 1);
    assert_eq!(forest[0].collection.id, parent.id);
    assert_eq!(forest[0].children.len(), applied.len());
}
