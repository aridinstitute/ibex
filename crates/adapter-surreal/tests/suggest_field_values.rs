use ibex_core::{NewResource, NonEmptyStr, ResourceKind, ResourceTextField, Store};
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
        abstract_text: None,
        keywords: vec![],
    }
}

async fn contract(store: &impl Store) {
    use ResourceTextField::*;
    for field in [
        Publisher,
        Institution,
        Organization,
        DocumentType,
        PublicationMethod,
        Language,
    ] {
        assert!(
            store
                .suggest_field_values(field, "".into(), 10)
                .await
                .unwrap()
                .is_empty()
        );
    }
    store
        .create_resource(bare_book("Absent scalars"))
        .await
        .unwrap();
    let mut blank = bare_book("Blank scalars");
    blank.publisher = Some("  ".into());
    blank.institution = Some("  ".into());
    blank.organization = Some("  ".into());
    blank.document_type = Some("  ".into());
    blank.publication_method = Some("  ".into());
    blank.language = Some("  ".into());
    store.create_resource(blank).await.unwrap();
    for value in [
        "zeta Press",
        "Alpha Press",
        "alpha press",
        "Alpha Press",
        "",
        " \t ",
        "Beta Custom",
    ] {
        let mut new = bare_book("Title must not match");
        new.publisher = Some(value.into());
        new.institution = Some(format!("Institute {value}"));
        new.organization = Some(format!("Organization {value}"));
        new.document_type = Some(format!("Type {value}"));
        new.publication_method = Some(format!("Method {value}"));
        new.language = Some(format!("Language {value}"));
        store.create_resource(new).await.unwrap();
    }
    for (field, prefix) in [
        (Publisher, ""),
        (Institution, "Institute "),
        (Organization, "Organization "),
        (DocumentType, "Type "),
        (PublicationMethod, "Method "),
        (Language, "Language "),
    ] {
        assert!(
            store
                .suggest_field_values(field, "".into(), 100)
                .await
                .unwrap()
                .iter()
                .all(|value| !value.trim().is_empty())
        );
        assert_eq!(
            store
                .suggest_field_values(field, "pReS".into(), 10)
                .await
                .unwrap(),
            [
                format!("{prefix}Alpha Press"),
                format!("{prefix}zeta Press")
            ]
        );
        assert_eq!(
            store
                .suggest_field_values(field, "custom".into(), 10)
                .await
                .unwrap(),
            [format!("{prefix}Beta Custom")]
        );
        assert_eq!(
            store
                .suggest_field_values(field, "Press".into(), 1)
                .await
                .unwrap(),
            [format!("{prefix}Alpha Press")]
        );
        assert!(
            store
                .suggest_field_values(field, "".into(), 0)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .suggest_field_values(field, "not found".into(), 10)
                .await
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(
        store
            .suggest_field_values(Publisher, "".into(), 10)
            .await
            .unwrap(),
        ["Alpha Press", "Beta Custom", "zeta Press"]
    );
    assert!(
        store
            .suggest_field_values(Publisher, "Institute".into(), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .suggest_field_values(Publisher, "'; RETURN true; --".into(), 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn suggestions_project_distinct_sorted_stored_values() {
    let store = ibex_adapter_surreal::SurrealStore::open_memory()
        .await
        .unwrap();
    contract(&store).await;
}
