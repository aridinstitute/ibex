#![cfg(feature = "testing")]

use ibex_app::metadata::{empty_resource, fetch_at};
use ibex_app::testing::{MemorySink, MemoryStore};
use ibex_app::{AppState, FieldValue, Surface};
use ibex_core::{Identifier, ResourceKind, Store};
use std::io::{Read, Write};

fn serve(body: &'static str) -> (String, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 4096];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    (url, handle)
}

const JOURNAL_WORK: &str = r#"{"message":{"title":["Paper"],"type":"journal-article","container-title":["The Journal"],"ISSN":["1234-5679"],"publisher":"Society"}}"#;

#[tokio::test]
async fn doi_fetch_reuses_journal_across_kind_filtered_pages() {
    let store = MemoryStore::new();
    let mut journal = empty_resource(" the JOURNAL ", ResourceKind::Journal).unwrap();
    journal.issns = vec![Identifier::new("12345679")];
    let existing = store.create_resource(journal).await.unwrap();
    for i in 0..105 {
        store
            .create_resource(
                empty_resource(&format!("Other journal {i}"), ResourceKind::Journal).unwrap(),
            )
            .await
            .unwrap();
    }
    store
        .create_resource(empty_resource("The Journal", ResourceKind::Book).unwrap())
        .await
        .unwrap();
    let sink = MemorySink::new();
    let state = AppState::builder(store)
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();
    let (url, handle) = serve(JOURNAL_WORK);
    let article = fetch_at(&state, "10.1234/paper", None, "http://unused.invalid", &url)
        .await
        .unwrap();
    handle.join().unwrap();
    assert_eq!(article.parent_id, Some(existing.id));
    let events = sink.take();
    assert!(!events.iter().any(|e| e.command() == "list_resources"));
    let pages: Vec<_> = events
        .iter()
        .filter(|e| e.command() == "list_resources_query_page")
        .collect();
    assert!(pages.len() >= 2);
    for page in pages {
        assert_eq!(
            page.get("resource.filter_kind"),
            Some(&FieldValue::from(ResourceKind::Journal.as_str()))
        );
        assert!(matches!(
            page.get("resource.page_limit"),
            Some(FieldValue::U64(1..=100))
        ));
    }
}

#[tokio::test]
async fn doi_fetch_reuses_title_match_and_enriches_missing_issn() {
    let store = MemoryStore::new();
    let existing = store
        .create_resource(empty_resource("the JOURNAL", ResourceKind::Journal).unwrap())
        .await
        .unwrap();
    let state = AppState::builder(store.clone())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    let (url, handle) = serve(JOURNAL_WORK);
    let article = fetch_at(&state, "10.1234/paper", None, "http://unused.invalid", &url)
        .await
        .unwrap();
    handle.join().unwrap();
    assert_eq!(article.parent_id, Some(existing.id));
    let journal = store.get_resource(existing.id).await.unwrap();
    assert_eq!(journal.issns[0].lookup_key(), "12345679");
    assert_eq!(journal.publisher.as_deref(), Some("Society"));
}

#[tokio::test]
async fn doi_fetch_creates_journal_when_absent() {
    let store = MemoryStore::new();
    let state = AppState::builder(store.clone())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    let (url, handle) = serve(JOURNAL_WORK);
    let article = fetch_at(&state, "10.1234/paper", None, "http://unused.invalid", &url)
        .await
        .unwrap();
    handle.join().unwrap();
    let journal = store
        .get_resource(article.parent_id.unwrap())
        .await
        .unwrap();
    assert_eq!(journal.kind, ResourceKind::Journal);
    assert_eq!(journal.title.as_str(), "The Journal");
    assert_eq!(journal.issns[0].lookup_key(), "12345679");
}

#[tokio::test]
async fn doi_fetch_rejects_issn_match_with_different_title() {
    let store = MemoryStore::new();
    let mut journal = empty_resource("Different title", ResourceKind::Journal).unwrap();
    journal.issns = vec![Identifier::new("12345679")];
    store.create_resource(journal).await.unwrap();
    let state = AppState::builder(store)
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    let (url, handle) = serve(JOURNAL_WORK);
    let error = fetch_at(&state, "10.1234/paper", None, "http://unused.invalid", &url)
        .await
        .unwrap_err();
    handle.join().unwrap();
    assert_eq!(
        error,
        "ISSN matches journal 'Different title' but Crossref calls it 'The Journal'"
    );
}

#[tokio::test]
async fn fetch_stores_new_metadata_and_fills_only_missing_fields_on_update() {
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    let (url, handle) = serve(
        r#"{"ISBN:9780306406157":{"title":"Original title","authors":[{"name":"Ada Lovelace"}],"publishers":[{"name":"Local Press"}]}}"#,
    );
    let created = fetch_at(&state, "9780306406157", None, &url, "http://unused.invalid")
        .await
        .unwrap();
    handle.join().unwrap();
    assert_eq!(created.title.as_str(), "Original title");
    assert_eq!(created.creators.len(), 1);
    assert_eq!(created.publisher.as_deref(), Some("Local Press"));

    let (url, handle) = serve(
        r#"{"ISBN:9780306406157":{"title":"Remote title","authors":[{"name":"Another Author"}],"publishers":[{"name":"Remote Press"}],"number_of_pages":42}}"#,
    );
    let updated = fetch_at(
        &state,
        "9780306406157",
        Some(created.id),
        &url,
        "http://unused.invalid",
    )
    .await
    .unwrap();
    handle.join().unwrap();
    assert_eq!(updated.id, created.id);
    assert_eq!(updated.title.as_str(), "Original title");
    assert_eq!(updated.publisher.as_deref(), Some("Local Press"));
    assert_eq!(updated.creators.len(), 1);
    assert_eq!(updated.total_pages.as_deref(), Some("42"));
}
