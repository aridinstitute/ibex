#![cfg(feature = "testing")]

use oeuvre_app::metadata::fetch_at;
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Surface};
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
