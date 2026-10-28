//! Real-adapter coverage for Task 05 read-time inheritance.

use oeuvre_adapter_surreal::SurrealStore;
use oeuvre_app::command::{
    CreateResource, GetResource, GetResourceDetail, ListResources, UpdateResource,
};
use oeuvre_app::{AppState, Command, Outcome, Surface};
use oeuvre_core::{
    BibliographicDate, NewResource, NonEmptyStr, ResourceId, ResourceKind, UtcMillis,
};

fn nes(s: &str) -> NonEmptyStr {
    NonEmptyStr::try_new(s).unwrap()
}

fn bare(kind: ResourceKind, title: &str, parent_id: Option<ResourceId>) -> NewResource {
    NewResource {
        kind,
        title: nes(title),
        creators: vec![],
        publisher: None,
        published: None,
        accessed: None,
        parent_id,
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
async fn parent_publisher_and_title_propagate_through_reopen_list_and_detail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library");
    let store = SurrealStore::open(&path).await.unwrap();
    let state = AppState::builder(store).surface(Surface::Test).build();

    let mut journal = bare(ResourceKind::Journal, "Nature", None);
    journal.publisher = Some("Old Pub".into());
    let Outcome::CreateResource(journal) = state
        .execute(Command::CreateResource(CreateResource { new: journal }))
        .await
        .unwrap()
    else {
        panic!("journal");
    };

    let mut issue = bare(ResourceKind::Issue, "Vol. 1, no. 1", Some(journal.id));
    issue.volume = Some("1".into());
    issue.issue = Some("1".into());
    issue.published = Some(
        BibliographicDate::try_new(UtcMillis::jan_1_utc(2021).unwrap(), Some(nes("2021")))
            .unwrap(),
    );
    let Outcome::CreateResource(issue) = state
        .execute(Command::CreateResource(CreateResource { new: issue }))
        .await
        .unwrap()
    else {
        panic!("issue");
    };

    let mut article = bare(ResourceKind::JournalArticle, "Paper", Some(issue.id));
    article.metadata.venue = Some("Nature".into());
    let Outcome::CreateResource(article) = state
        .execute(Command::CreateResource(CreateResource { new: article }))
        .await
        .unwrap()
    else {
        panic!("article");
    };

    let Outcome::GetResourceDetail(before) = state
        .execute(Command::GetResourceDetail(GetResourceDetail {
            id: article.id,
        }))
        .await
        .unwrap()
    else {
        panic!("detail");
    };
    assert_eq!(before.publisher.as_deref(), Some("Old Pub"));
    assert_eq!(before.volume.as_deref(), Some("1"));
    assert_eq!(
        before.container_title.as_ref().map(|s| s.as_str()),
        Some("Nature")
    );

    let mut edited = journal.to_new();
    edited.publisher = Some("New Pub".into());
    edited.title = nes("Renamed Journal");
    let Outcome::UpdateResource(_) = state
        .execute(Command::UpdateResource(UpdateResource {
            id: journal.id,
            new: edited,
        }))
        .await
        .unwrap()
    else {
        panic!("update");
    };

    drop(state);

    let reopened = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match SurrealStore::open(&path).await {
                Ok(store) => break store,
                Err(err) if err.message.contains("already locked") => {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Err(err) => panic!("reopen failed: {err}"),
            }
        }
    })
    .await
    .expect("database lock released");

    let state = AppState::builder(reopened).surface(Surface::Test).build();
    let Outcome::GetResourceDetail(after) = state
        .execute(Command::GetResourceDetail(GetResourceDetail {
            id: article.id,
        }))
        .await
        .unwrap()
    else {
        panic!("detail after");
    };
    assert_eq!(after.publisher.as_deref(), Some("New Pub"));
    assert_eq!(after.volume.as_deref(), Some("1"));
    assert_eq!(
        after.container_title.as_ref().map(NonEmptyStr::as_str),
        Some("Renamed Journal")
    );

    let Outcome::ListResources(rows) = state
        .execute(Command::ListResources(ListResources))
        .await
        .unwrap()
    else {
        panic!("list");
    };
    let row = rows.iter().find(|r| r.id == article.id).unwrap();
    assert_eq!(row.publisher.as_deref(), Some("New Pub"));
    assert_eq!(
        row.container_title.as_ref().map(|s| s.as_str()),
        Some("Renamed Journal")
    );

    let Outcome::GetResource(stored) = state
        .execute(Command::GetResource(GetResource { id: article.id }))
        .await
        .unwrap()
    else {
        panic!("raw");
    };
    assert!(stored.publisher.is_none());
    assert!(stored.volume.is_none());
    assert_eq!(stored.metadata.venue.as_deref(), Some("Nature"));
}
