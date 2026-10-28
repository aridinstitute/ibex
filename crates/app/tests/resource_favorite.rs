#![cfg(feature = "testing")]

use oeuvre_app::command::{
    CountResources, CreateResource, GetResource, ListResourcesQueryPage, SetResourceFavorite,
};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, CommandError, FieldValue, Outcome, Status, Surface};
use oeuvre_core::{
    NewResource, NonEmptyStr, ResourceFilter, ResourceId, ResourceKind, ResourceQuery,
};

fn bare_book(title: &str) -> NewResource {
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
async fn favorite_toggles_filters_and_counts_with_one_event() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();

    let Outcome::CreateResource(a) = state
        .execute(Command::CreateResource(CreateResource {
            new: bare_book("Starred"),
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };
    let Outcome::CreateResource(b) = state
        .execute(Command::CreateResource(CreateResource {
            new: bare_book("Plain"),
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };
    assert!(!a.favorite);
    assert!(!b.favorite);
    let _ = sink.take();

    let Outcome::SetResourceFavorite(starred) = state
        .execute(Command::SetResourceFavorite(SetResourceFavorite {
            id: a.id,
            favorite: true,
        }))
        .await
        .unwrap()
    else {
        panic!("expected SetResourceFavorite");
    };
    assert!(starred.favorite);
    assert_eq!(starred.date_edited, a.date_edited);

    let Outcome::GetResource(got) = state
        .execute(Command::GetResource(GetResource { id: a.id }))
        .await
        .unwrap()
    else {
        panic!("expected GetResource");
    };
    assert!(got.favorite);

    let Outcome::ListResourcesQueryPage(page) = state
        .execute(Command::ListResourcesQueryPage(ListResourcesQueryPage {
            query: ResourceQuery {
                sort: None,
                filter: ResourceFilter {
                    favorite: Some(true),
                    tag: None,
                    collection: None,
                    ..ResourceFilter::none()
                },
            },
            offset: 0,
            limit: 10,
        }))
        .await
        .unwrap()
    else {
        panic!("expected ListResourcesQueryPage");
    };
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].id, a.id);

    let Outcome::CountResources(counts) = state
        .execute(Command::CountResources(CountResources))
        .await
        .unwrap()
    else {
        panic!("expected CountResources");
    };
    assert_eq!(counts.all_items, 2);
    assert_eq!(counts.favorites, 1);

    // Bibliographic update must keep the star.
    let mut edit = got.to_new();
    edit.title = NonEmptyStr::try_new("Still starred").unwrap();
    let Outcome::UpdateResource(updated) = state
        .execute(Command::UpdateResource(
            oeuvre_app::command::UpdateResource {
                id: a.id,
                new: edit,
            },
        ))
        .await
        .unwrap()
    else {
        panic!("expected UpdateResource");
    };
    assert!(updated.favorite);

    let events = sink.take();
    let favorite_event = events
        .iter()
        .find(|e| e.command() == "set_resource_favorite")
        .expect("favorite event");
    assert_eq!(favorite_event.status(), Status::Ok);
    assert_eq!(
        favorite_event.get("resource.favorite"),
        Some(&FieldValue::from(true))
    );
}

#[tokio::test]
async fn favorite_missing_resource_is_not_found() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();
    let id = ResourceId::generate();
    let err = state
        .execute(Command::SetResourceFavorite(SetResourceFavorite {
            id,
            favorite: true,
        }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::NotFound(_)));
    let events = sink.take();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].command(), "set_resource_favorite");
    assert_eq!(events[0].status(), Status::Error);
}
