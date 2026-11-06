#![cfg(feature = "testing")]

use ibex_app::command::{CreateResource, InheritancePreview, PreviewInheritance};
use ibex_app::testing::{MemorySink, MemoryStore};
use ibex_app::{AppState, Command, FieldValue, Outcome, Status, Surface};
use ibex_core::{
    BibliographicDate, Effective, NewResource, NonEmptyStr, Resource, ResourceId, ResourceKind,
    UtcMillis, ValueOrigin,
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
        abstract_text: None,
        keywords: vec![],
    }
}

async fn create(state: &AppState, new: NewResource) -> Resource {
    let Outcome::CreateResource(r) = state
        .execute(Command::CreateResource(CreateResource { new }))
        .await
        .unwrap()
    else {
        panic!("create");
    };
    r
}

async fn preview(
    state: &AppState,
    kind: ResourceKind,
    parent_id: Option<ResourceId>,
) -> InheritancePreview {
    let Outcome::PreviewInheritance(preview) = state
        .execute(Command::PreviewInheritance(PreviewInheritance {
            kind,
            parent_id,
        }))
        .await
        .unwrap()
    else {
        panic!("preview outcome");
    };
    preview
}

/// Journal "Nature" (publisher) -> issue (volume 12, issue 3, 2020).
async fn journal_and_issue(state: &AppState) -> (Resource, Resource) {
    let mut journal = bare(ResourceKind::Journal, "Nature", None);
    journal.publisher = Some("Springer Nature".into());
    let journal = create(state, journal).await;
    let mut issue = bare(ResourceKind::Issue, "Vol. 12, no. 3", Some(journal.id));
    issue.volume = Some("12".into());
    issue.issue = Some("3".into());
    issue.published = Some(
        BibliographicDate::try_new(UtcMillis::jan_1_utc(2020).unwrap(), Some(nes("2020"))).unwrap(),
    );
    let issue = create(state, issue).await;
    (journal, issue)
}

fn all_absent(preview: &InheritancePreview) {
    let v = &preview.values;
    assert_eq!(v.publisher, Effective::absent());
    assert_eq!(v.published, Effective::absent());
    assert_eq!(v.volume, Effective::absent());
    assert_eq!(v.issue, Effective::absent());
    assert_eq!(v.container_title, Effective::absent());
    assert!(preview.sources.is_empty());
}

#[tokio::test]
async fn article_under_issue_previews_issue_and_journal_values() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();
    let (journal, issue) = journal_and_issue(&state).await;
    sink.take();

    let preview = preview(&state, ResourceKind::JournalArticle, Some(issue.id)).await;
    let v = &preview.values;
    assert_eq!(v.volume.value.as_deref(), Some("12"));
    assert_eq!(
        v.volume.origin,
        ValueOrigin::Inherited {
            parent_id: issue.id
        }
    );
    assert_eq!(v.issue.value.as_deref(), Some("3"));
    assert_eq!(v.published.value, issue.published);
    assert_eq!(v.publisher.value.as_deref(), Some("Springer Nature"));
    assert_eq!(
        v.publisher.origin,
        ValueOrigin::Inherited {
            parent_id: journal.id
        }
    );
    assert_eq!(preview.sources.get(&issue.id), Some(&issue.title));
    assert_eq!(preview.sources.get(&journal.id), Some(&journal.title));

    let events = sink.take();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].command(), "preview_inheritance");
    assert_eq!(events[0].status(), Status::Ok);
    assert_eq!(
        events[0].get("resource.kind"),
        Some(&FieldValue::from("journalArticle"))
    );
    assert_eq!(
        events[0].get("resource.id"),
        Some(&FieldValue::from(issue.id.to_string()))
    );
}

#[tokio::test]
async fn kind_that_cannot_link_to_parent_previews_nothing() {
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    let (_journal, issue) = journal_and_issue(&state).await;

    let preview = preview(&state, ResourceKind::Book, Some(issue.id)).await;
    all_absent(&preview);
}

#[tokio::test]
async fn no_parent_previews_nothing() {
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();

    let preview = preview(&state, ResourceKind::JournalArticle, None).await;
    all_absent(&preview);
}
