#![cfg(feature = "testing")]

use ibex_app::command::{
    CreateResource, CreateTag, GetResourceDetail, ListResourcesQueryPage, ListTags,
};
use ibex_app::testing::MemoryStore;
use ibex_app::{AppState, Command, Outcome, Surface};
use ibex_core::{NewResource, NonEmptyStr, ResourceFilter, ResourceKind, ResourceQuery, Store};

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
        metadata: Default::default(),
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
    }
}

#[tokio::test]
async fn create_tag_enforces_exact_text_uniqueness() {
    let store = MemoryStore::new();
    let state = AppState::builder(store).surface(Surface::Test).build();

    let Outcome::CreateTag(first) = state
        .execute(Command::CreateTag(CreateTag {
            text: NonEmptyStr::try_new("epistemology").unwrap(),
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateTag");
    };
    assert_eq!(first.text.as_str(), "epistemology");

    let err = state
        .execute(Command::CreateTag(CreateTag {
            text: NonEmptyStr::try_new("epistemology").unwrap(),
        }))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already exists"));

    // Different casing is a different tag under exact uniqueness.
    let Outcome::CreateTag(second) = state
        .execute(Command::CreateTag(CreateTag {
            text: NonEmptyStr::try_new("Epistemology").unwrap(),
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateTag");
    };
    assert_ne!(first.id, second.id);

    let Outcome::ListTags(tags) = state.execute(Command::ListTags(ListTags)).await.unwrap() else {
        panic!("expected ListTags");
    };
    assert_eq!(tags.len(), 2);
}

#[tokio::test]
async fn resource_detail_and_filter_use_tag_refs() {
    let store = MemoryStore::new();
    let tag = store
        .create_tag(NonEmptyStr::try_new("methods").unwrap())
        .await
        .unwrap();
    let state = AppState::builder(store).surface(Surface::Test).build();

    let mut tagged = book("Tagged Work");
    tagged.tag_ids = vec![tag.id];
    let Outcome::CreateResource(resource) = state
        .execute(Command::CreateResource(CreateResource { new: tagged }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };
    state
        .execute(Command::CreateResource(CreateResource {
            new: book("Untagged Work"),
        }))
        .await
        .unwrap();

    let Outcome::GetResourceDetail(detail) = state
        .execute(Command::GetResourceDetail(GetResourceDetail {
            id: resource.id,
        }))
        .await
        .unwrap()
    else {
        panic!("expected detail");
    };
    assert_eq!(detail.tags.len(), 1);
    assert_eq!(detail.tags[0].text.as_str(), "methods");

    let Outcome::ListResourcesQueryPage(page) = state
        .execute(Command::ListResourcesQueryPage(ListResourcesQueryPage {
            query: ResourceQuery {
                sort: None,
                filter: ResourceFilter {
                    tag: Some(tag.id),
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
    assert_eq!(page.rows[0].id, resource.id);
}
