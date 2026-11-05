#![cfg(feature = "testing")]

//! Task 05: read-time inheritance — allowlist, overrides, parent updates,
//! non-inherited fields, and predictable failures.

use ibex_app::command::{
    CreateResource, GetResource, GetResourceDetail, ListResources, UpdateResource,
};
use ibex_app::testing::{MemorySink, MemoryStore};
use ibex_app::{AppState, Command, CommandError, Outcome, Surface};
use ibex_core::{
    BibliographicDate, CreatorRole, CreatorSpec, Identifier, Name, NewResource, NonEmptyStr, Pages,
    PersonId, ResourceId, ResourceKind, UtcMillis,
};

fn nes(s: &str) -> NonEmptyStr {
    NonEmptyStr::try_new(s).unwrap()
}

fn state() -> AppState {
    AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build()
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

async fn create(state: &AppState, new: NewResource) -> ibex_core::Resource {
    let Outcome::CreateResource(r) = state
        .execute(Command::CreateResource(CreateResource { new }))
        .await
        .unwrap()
    else {
        panic!("create");
    };
    r
}

async fn detail(state: &AppState, id: ResourceId) -> ibex_core::ResourceDetail {
    let Outcome::GetResourceDetail(d) = state
        .execute(Command::GetResourceDetail(GetResourceDetail { id }))
        .await
        .unwrap()
    else {
        panic!("detail");
    };
    d
}

async fn raw(state: &AppState, id: ResourceId) -> ibex_core::Resource {
    let Outcome::GetResource(r) = state
        .execute(Command::GetResource(GetResource { id }))
        .await
        .unwrap()
    else {
        panic!("get");
    };
    r
}

async fn list(state: &AppState) -> Vec<ibex_core::ResourceSummary> {
    let Outcome::ListResources(rows) = state
        .execute(Command::ListResources(ListResources))
        .await
        .unwrap()
    else {
        panic!("list");
    };
    rows
}

async fn person(state: &AppState, last: &str) -> PersonId {
    let name = Name::try_new(None::<&str>, Some(last), None::<&str>).unwrap();
    let Outcome::CreatePerson(p) = state
        .execute(Command::CreatePerson(ibex_app::command::CreatePerson {
            name,
            aliases: vec![],
        }))
        .await
        .unwrap()
    else {
        panic!("person");
    };
    p.id
}

#[tokio::test]
async fn article_inherits_issue_and_journal_fields_in_detail_and_list() {
    let state = state();
    let mut journal = bare(ResourceKind::Journal, "Nature", None);
    journal.publisher = Some("Springer Nature".into());
    journal.language = Some("en".into());
    journal.issns = vec![Identifier::new("0028-0836")];
    let journal = create(&state, journal).await;

    let mut issue = bare(ResourceKind::Issue, "Vol. 12, no. 3", Some(journal.id));
    issue.volume = Some("12".into());
    issue.issue = Some("3".into());
    issue.published = Some(
        BibliographicDate::try_new(UtcMillis::jan_1_utc(2020).unwrap(), Some(nes("2020"))).unwrap(),
    );
    let issue = create(&state, issue).await;

    let article = create(
        &state,
        bare(ResourceKind::JournalArticle, "Paper", Some(issue.id)),
    )
    .await;

    let d = detail(&state, article.id).await;
    assert_eq!(d.volume.as_deref(), Some("12"));
    assert_eq!(d.issue.as_deref(), Some("3"));
    assert_eq!(d.publisher.as_deref(), Some("Springer Nature"));
    assert_eq!(d.language.as_deref(), Some("en"));
    assert_eq!(d.issns[0].as_str(), "0028-0836");
    assert_eq!(
        d.container_title.as_ref().map(|s| s.as_str()),
        Some("Nature")
    );
    assert_eq!(d.published.as_ref().and_then(|p| p.display()), Some("2020"));

    // Stored child is unchanged.
    let stored = raw(&state, article.id).await;
    assert!(stored.publisher.is_none());
    assert!(stored.volume.is_none());
    assert!(stored.issns.is_empty());

    let row = list(&state)
        .await
        .into_iter()
        .find(|r| r.id == article.id)
        .unwrap();
    assert_eq!(row.publisher.as_deref(), Some("Springer Nature"));
    assert_eq!(
        row.container_title.as_ref().map(|s| s.as_str()),
        Some("Nature")
    );
    assert_eq!(
        row.published_label.as_ref().map(|s| s.as_str()),
        Some("2020")
    );
}

#[tokio::test]
async fn explicit_fields_override_defaults_but_linked_container_overrides_venue() {
    let state = state();
    let mut journal = bare(ResourceKind::Journal, "Nature", None);
    journal.publisher = Some("Springer Nature".into());
    let journal = create(&state, journal).await;

    let mut issue = bare(ResourceKind::Issue, "Vol. 1, no. 1", Some(journal.id));
    issue.volume = Some("1".into());
    issue.issue = Some("1".into());
    let issue = create(&state, issue).await;

    let mut article = bare(ResourceKind::JournalArticle, "Override", Some(issue.id));
    article.publisher = Some("Author Press".into());
    article.volume = Some("99".into());
    article.metadata.venue = Some("Custom Venue".into());
    let article = create(&state, article).await;

    let d = detail(&state, article.id).await;
    assert_eq!(d.publisher.as_deref(), Some("Author Press"));
    assert_eq!(d.volume.as_deref(), Some("99"));
    assert_eq!(d.issue.as_deref(), Some("1"));
    assert_eq!(
        d.container_title.as_ref().map(|s| s.as_str()),
        Some("Nature")
    );
}

#[tokio::test]
async fn journal_rename_updates_linked_article_container_despite_imported_venue() {
    let state = state();
    let journal = create(&state, bare(ResourceKind::Journal, "BSac", None)).await;
    let issue = create(
        &state,
        bare(ResourceKind::Issue, "Vol. 182, no. 1", Some(journal.id)),
    )
    .await;
    let mut article = bare(ResourceKind::JournalArticle, "Paper", Some(issue.id));
    article.metadata.venue = Some("BSac".into());
    let article = create(&state, article).await;

    assert_eq!(
        detail(&state, article.id)
            .await
            .container_title
            .as_ref()
            .map(NonEmptyStr::as_str),
        Some("BSac")
    );

    let mut edited = journal.to_new();
    edited.title = nes("Bibliotheca Sacra");
    state
        .execute(Command::UpdateResource(UpdateResource {
            id: journal.id,
            new: edited,
        }))
        .await
        .unwrap();

    let after = detail(&state, article.id).await;
    let row = list(&state)
        .await
        .into_iter()
        .find(|r| r.id == article.id)
        .unwrap();
    assert_eq!(
        (
            after.container_title.as_ref().map(NonEmptyStr::as_str),
            row.container_title.as_ref().map(NonEmptyStr::as_str),
        ),
        (Some("Bibliotheca Sacra"), Some("Bibliotheca Sacra")),
        "linked journal title must win over the article's imported venue on fresh reads"
    );

    // Resolving the current container title must not rewrite the stored article.
    assert_eq!(raw(&state, article.id).await, article);
}

#[tokio::test]
async fn parent_update_propagates_to_list_and_detail_without_child_write() {
    let state = state();
    let mut journal = bare(ResourceKind::Journal, "Nature", None);
    journal.publisher = Some("Old Pub".into());
    let journal = create(&state, journal).await;
    let issue = create(
        &state,
        bare(ResourceKind::Issue, "Vol. 1, no. 1", Some(journal.id)),
    )
    .await;
    let article = create(
        &state,
        bare(ResourceKind::JournalArticle, "Paper", Some(issue.id)),
    )
    .await;

    assert_eq!(
        detail(&state, article.id).await.publisher.as_deref(),
        Some("Old Pub")
    );

    let mut edited = journal.to_new();
    edited.publisher = Some("New Pub".into());
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

    let d = detail(&state, article.id).await;
    assert_eq!(d.publisher.as_deref(), Some("New Pub"));
    let row = list(&state)
        .await
        .into_iter()
        .find(|r| r.id == article.id)
        .unwrap();
    assert_eq!(row.publisher.as_deref(), Some("New Pub"));

    let stored = raw(&state, article.id).await;
    assert!(stored.publisher.is_none(), "read must not mutate child");
}

#[tokio::test]
async fn doi_pages_accessed_and_creators_are_not_inherited() {
    let state = state();
    let editor = person(&state, "Editor").await;
    let mut journal = bare(ResourceKind::Journal, "Nature", None);
    journal.doi = Some("10.journal/x".into());
    journal.pages = Some(Pages::parse("1--10").unwrap());
    journal.accessed =
        Some(BibliographicDate::try_new(UtcMillis::jan_1_utc(2019).unwrap(), None).unwrap());
    journal.creators = vec![CreatorSpec {
        person_id: editor,
        role: CreatorRole::Editor,
        credited_as: None,
    }];
    journal.publisher = Some("SN".into());
    let journal = create(&state, journal).await;

    let article = create(
        &state,
        bare(ResourceKind::JournalArticle, "Paper", Some(journal.id)),
    )
    .await;

    let d = detail(&state, article.id).await;
    assert!(d.doi.is_none());
    assert!(d.pages.is_none());
    assert!(d.accessed.is_none());
    assert!(d.creators.is_empty());
    assert_eq!(d.publisher.as_deref(), Some("SN"));

    let stored = raw(&state, article.id).await;
    assert!(stored.doi.is_none());
    assert!(stored.creators.is_empty());
}

#[tokio::test]
async fn missing_parent_and_invalid_parent_fail_predictably() {
    let state = state();
    let missing = ResourceId::generate();
    let err = state
        .execute(Command::CreateResource(CreateResource {
            new: bare(ResourceKind::JournalArticle, "X", Some(missing)),
        }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::NotFound(_)));

    let book = create(&state, bare(ResourceKind::Book, "Book", None)).await;
    // Create validates parent kind, so invalid parents never persist; detail
    // therefore never sees them. Assert create rejects the mismatch.
    let err = state
        .execute(Command::CreateResource(CreateResource {
            new: bare(ResourceKind::JournalArticle, "Bad", Some(book.id)),
        }))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Invalid(_)));
}

#[tokio::test]
async fn chapter_inherits_book_publisher_and_container() {
    let state = state();
    let mut book = bare(ResourceKind::Book, "Collected", None);
    book.publisher = Some("MIT Press".into());
    book.edition = Some("2nd".into());
    book.doi = Some("10.book/x".into());
    let book = create(&state, book).await;

    let chapter = create(
        &state,
        bare(ResourceKind::BookSection, "Ch. 1", Some(book.id)),
    )
    .await;

    let d = detail(&state, chapter.id).await;
    assert_eq!(d.publisher.as_deref(), Some("MIT Press"));
    assert_eq!(d.edition.as_deref(), Some("2nd"));
    assert_eq!(
        d.container_title.as_ref().map(|s| s.as_str()),
        Some("Collected")
    );
    assert!(d.doi.is_none());
}
