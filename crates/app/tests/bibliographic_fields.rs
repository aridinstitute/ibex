#![cfg(feature = "testing")]

use ibex_app::command::{CreateResource, GetResource, GetResourceDetail, UpdateResource};
use ibex_app::testing::{MemorySink, MemoryStore};
use ibex_app::{AppState, Command, FieldValue, Outcome, Surface};
use ibex_core::NewResource;
use serde_json::json;

#[tokio::test]
async fn bibliographic_fields_are_returned_in_detail_and_preserved_on_update() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();
    let input = json!({
        "kind": "book", "title": "Example", "creators": [],
        "publisher": null, "published": null, "accessed": null,
        "parent_id": null, "series_id": null, "series_number": null,
        "tag_ids": [], "collection_ids": [],
        "subtitle": "A guide", "title_addon": "Revised", "short_title": "Guide",
        "place": "London", "volume": "II", "issue": "4", "pages": "10–20",
        "total_pages": "xii+250", "article_number": "e123", "edition": "Second",
        "doi": "10.1234/example", "url": "https://example.org", "eprint": "1234.5678",
        "isbns": ["978-0-306-40615-7"], "issns": ["1234-5678"], "isrns": ["EXAMPLE-1"],
        "original_date": {"instant": 0, "display": "1970"},
        "event_date": {"instant": 86400000, "display": null},
        "language": "en", "document_type": "Research report", "publication_method": "Online",
        "institution": "Example University", "organization": "Citation Society",
        "abstract_text": "A summary.", "keywords": ["history", "bibliography"]
    });
    let new: NewResource = serde_json::from_value(input.clone()).unwrap();
    let Outcome::CreateResource(created) = state
        .execute(Command::CreateResource(CreateResource { new: new.clone() }))
        .await
        .unwrap()
    else {
        panic!("create");
    };
    let Outcome::GetResource(fetched) = state
        .execute(Command::GetResource(GetResource { id: created.id }))
        .await
        .unwrap()
    else {
        panic!("get");
    };
    assert_eq!(fetched.to_new(), new);
    let Outcome::GetResourceDetail(detail) = state
        .execute(Command::GetResourceDetail(GetResourceDetail {
            id: created.id,
        }))
        .await
        .unwrap()
    else {
        panic!("detail");
    };
    let detail = serde_json::to_value(detail).unwrap();
    assert!(detail.get("note").is_none());
    assert!(detail.get("annotation").is_none());
    for field in [
        "subtitle",
        "title_addon",
        "short_title",
        "place",
        "volume",
        "issue",
        "total_pages",
        "article_number",
        "edition",
        "doi",
        "url",
        "eprint",
        "language",
        "document_type",
        "publication_method",
        "institution",
        "organization",
        "abstract_text",
        "keywords",
    ] {
        assert_eq!(detail[field], input[field], "detail lost {field}");
    }
    assert_eq!(detail["pages"]["literal"], "10–20");
    assert_eq!(detail["isbns"][0]["original"], "978-0-306-40615-7");
    assert_eq!(detail["issns"][0]["original"], "1234-5678");
    assert_eq!(detail["isrns"][0]["original"], "EXAMPLE-1");
    assert_eq!(detail["original_date"]["instant"], 0);
    assert_eq!(detail["original_date"]["display"], "1970");
    assert_eq!(detail["event_date"]["instant"], 86400000);
    let mut edit = fetched.to_new();
    let _ = sink.take();
    edit.doi = Some("10.1234/updated".into());
    edit.keywords = vec!["updated".into()];
    state
        .execute(Command::UpdateResource(UpdateResource {
            id: created.id,
            new: edit.clone(),
        }))
        .await
        .unwrap();
    let Outcome::GetResource(fetched) = state
        .execute(Command::GetResource(GetResource { id: created.id }))
        .await
        .unwrap()
    else {
        panic!("get updated");
    };
    assert_eq!(fetched.to_new(), edit);
    let events = sink.take();
    let updated = &events[0];
    assert_eq!(
        updated.get("resource.changed_fields"),
        Some(&FieldValue::from("doi,keywords"))
    );
}

#[test]
fn older_resource_payloads_default_new_metadata() {
    let new: NewResource = serde_json::from_value(json!({
        "kind": "book", "title": "Legacy", "creators": [],
        "tag_ids": [], "collection_ids": []
    }))
    .unwrap();
    assert!(new.subtitle.is_none());
    assert!(new.doi.is_none());
    assert!(new.original_date.is_none());
    assert!(new.abstract_text.is_none());
    assert!(new.isbns.is_empty());
    assert!(new.issns.is_empty());
    assert!(new.isrns.is_empty());
    assert!(new.keywords.is_empty());
}
