#![cfg(feature = "testing")]

use ibex_app::cluster::{SuggestClusters, apply_clusters, suggest_clusters};
use ibex_app::command::{CreateResource, GetResource, ListCollectionForest};
use ibex_app::testing::MemoryStore;
use ibex_app::{AppState, Command, Outcome, Surface};
use ibex_core::{NewResource, NonEmptyStr, ResourceKind};

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

async fn add(state: &AppState, new: NewResource) {
    let Outcome::CreateResource(_) = state
        .execute(Command::CreateResource(CreateResource { new }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };
}

#[tokio::test]
async fn suggest_and_apply_creates_topic_collections() {
    let store = MemoryStore::new();
    let state = AppState::builder(store).surface(Surface::Test).build();

    add(
        &state,
        article(
            "Neural nets",
            Some("Deep architectures"),
            "deep learning neural networks representation learning backpropagation layers",
        ),
    )
    .await;
    add(
        &state,
        article(
            "CNN survey",
            None,
            "convolutional neural networks deep learning image recognition layers",
        ),
    )
    .await;
    add(
        &state,
        article(
            "Transformers",
            None,
            "transformer attention neural networks deep learning language models",
        ),
    )
    .await;
    add(
        &state,
        article(
            "Carbon sinks",
            Some("Forest carbon"),
            "climate change carbon sequestration forests greenhouse emissions",
        ),
    )
    .await;
    add(
        &state,
        article(
            "Warming trends",
            None,
            "climate change global warming temperature greenhouse emissions",
        ),
    )
    .await;
    add(
        &state,
        article(
            "Ocean acid",
            None,
            "climate change ocean acidification carbon dioxide greenhouse emissions",
        ),
    )
    .await;

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
