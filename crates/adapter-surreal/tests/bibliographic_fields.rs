use ibex_adapter_surreal::SurrealStore;
use ibex_core::{BibliographicDate, NewResource, NonEmptyStr, ResourceKind, Store, UtcMillis};

fn example() -> NewResource {
    NewResource {
        kind: ResourceKind::Book,
        title: NonEmptyStr::try_new("The Reference Companion").unwrap(),
        creators: vec![],
        publisher: Some("Loom Press".into()),
        published: None,
        accessed: None,
        parent_id: None,
        series_id: None,
        series_number: None,
        tag_ids: vec![],
        collection_ids: vec![],
        metadata: Default::default(),
        subtitle: Some(NonEmptyStr::try_new("A research guide").unwrap()),
        title_addon: Some(NonEmptyStr::try_new("Revised").unwrap()),
        short_title: Some(NonEmptyStr::try_new("Companion").unwrap()),
        place: Some("London".into()),
        volume: Some("II".into()),
        issue: Some("4".into()),
        pages: Some(ibex_core::Pages::parse("10–20").unwrap()),
        total_pages: Some("xii+250".into()),
        article_number: Some("e123".into()),
        edition: Some("Second".into()),
        doi: Some("10.1234/example".into()),
        url: Some("https://example.org/companion".into()),
        eprint: Some("arXiv:1234.5678".into()),
        isbns: vec![
            ibex_core::Identifier::from("978-0-306-40615-7"),
            ibex_core::Identifier::from("0-306-40615-2"),
        ],
        issns: vec![ibex_core::Identifier::from("1234-5678")],
        isrns: vec![ibex_core::Identifier::from("EXAMPLE-REPORT-1")],
        original_date: Some(
            BibliographicDate::try_new(
                UtcMillis::from_unix_ms(0),
                Some(NonEmptyStr::try_new("1970").unwrap()),
            )
            .unwrap(),
        ),
        event_date: Some(
            BibliographicDate::try_new(UtcMillis::from_unix_ms(86_400_000), None).unwrap(),
        ),
        language: Some("en".into()),
        document_type: Some("Research report".into()),
        publication_method: Some("Online".into()),
        institution: Some("Example University".into()),
        organization: Some("Citation Society".into()),
        school: Some("School of History".into()),
        abstract_text: Some("A guide to bibliographic research.".into()),
        keywords: vec!["history".into(), "bibliography".into()],
    }
}

#[tokio::test]
async fn bibliographic_fields_survive_storage_updates_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library");
    let store = SurrealStore::open(&path).await.unwrap();
    let expected = example();
    let resource = store.create_resource(expected.clone()).await.unwrap();
    assert_eq!(
        store.get_resource(resource.id).await.unwrap().to_new(),
        expected
    );

    let mut edited = store.get_resource(resource.id).await.unwrap().to_new();
    edited.title = NonEmptyStr::try_new("Updated title").unwrap();
    edited.keywords.push("cataloguing".into());
    store
        .update_resource(resource.id, edited.clone())
        .await
        .unwrap();
    assert_eq!(
        store
            .set_resource_favorite(resource.id, true)
            .await
            .unwrap()
            .to_new(),
        edited
    );
    drop(store);
    // SurrealKV releases its directory lock asynchronously after the last handle drops.
    let reopened = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match SurrealStore::open(&path).await {
                Ok(store) => break store,
                Err(err) if err.is_in_use() => {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Err(err) => panic!("reopen failed: {err}"),
            }
        }
    })
    .await
    .expect("database lock released");
    assert_eq!(
        reopened.get_resource(resource.id).await.unwrap().to_new(),
        edited
    );

    edited.subtitle = None;
    edited.doi = None;
    edited.keywords.clear();
    reopened
        .update_resource(resource.id, edited.clone())
        .await
        .unwrap();
    assert_eq!(
        reopened.get_resource(resource.id).await.unwrap().to_new(),
        edited
    );
}
