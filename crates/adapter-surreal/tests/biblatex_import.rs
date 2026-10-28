use oeuvre_adapter_surreal::SurrealStore;
use oeuvre_app::command::{
    GetResource, GetResourceDetail, ImportBiblatex, ListPersons, ListResources,
    ListResourcesQueryPage,
};
use oeuvre_app::{AppState, Command, Outcome, Surface};
use oeuvre_core::{
    CreatorRole, CreatorSpec, ImportedResourceWrite, Name, NewResource, NewSeries, NonEmptyStr,
    Person, PersonId, ResourceKind, ResourceQuery, ResourceSort, Series, SeriesId, SortDirection,
    Store,
};

#[tokio::test]
async fn imported_metadata_is_visible_in_library_rows_and_details() {
    let state = AppState::builder(SurrealStore::open_memory().await.unwrap())
        .surface(Surface::Test)
        .build();
    let Outcome::ImportBiblatex(report) = state.execute(Command::ImportBiblatex(ImportBiblatex {
        source_text: "@article{paper,title={An imported paper},author={Hopper, Grace},journaltitle={Example Journal},doi={10.1000/paper},keywords={history, computing},date={2024-03-15}}".into(),
    })).await.unwrap() else { panic!("import"); };
    assert_eq!(report.imported.len(), 1);
    let Outcome::GetResourceDetail(detail) = state
        .execute(Command::GetResourceDetail(GetResourceDetail {
            id: report.imported[0].resource_id,
        }))
        .await
        .unwrap()
    else {
        panic!("detail");
    };
    assert_eq!(detail.doi.as_deref(), Some("10.1000/paper"));
    assert_eq!(detail.keywords, ["history", "computing"]);
    assert_eq!(
        detail.container_title.as_ref().map(|s| s.as_str()),
        Some("Example Journal")
    );
    let Outcome::ListResources(rows) = state
        .execute(Command::ListResources(ListResources))
        .await
        .unwrap()
    else {
        panic!("rows");
    };
    assert_eq!(
        rows[0].container_title.as_ref().map(|s| s.as_str()),
        Some("Example Journal")
    );
    for sort in [
        ResourceSort::Title,
        ResourceSort::Published,
        ResourceSort::Publisher,
        ResourceSort::PrimaryCreator,
        ResourceSort::DateAdded,
        ResourceSort::DateEdited,
    ] {
        let Outcome::ListResourcesQueryPage(page) = state
            .execute(Command::ListResourcesQueryPage(ListResourcesQueryPage {
                query: ResourceQuery::sorted(sort, SortDirection::Ascending),
                offset: 0,
                limit: 10,
            }))
            .await
            .unwrap()
        else {
            panic!("page");
        };
        assert_eq!(
            page.rows[0].container_title.as_ref().map(|s| s.as_str()),
            Some("Example Journal")
        );
    }
}

#[tokio::test]
async fn import_persists_people_roles_order_and_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library");
    let store = SurrealStore::open(&path).await.unwrap();
    let state = AppState::builder(store).surface(Surface::Test).build();
    let source = r#"@book{work,
        title = {Linked Creators},
        author = {Lovelace, Ada and Hopper, Grace},
        editor = {Knuth, Donald},
        translator = {Turing, Alan},
        series = {Computing Classics},
        number = {3}
    }"#;
    let Outcome::ImportBiblatex(report) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: source.into(),
        }))
        .await
        .unwrap()
    else {
        panic!("import");
    };
    assert_eq!(report.imported.len(), 1);
    let resource_id = report.imported[0].resource_id;

    let Outcome::GetResource(book) = state
        .execute(Command::GetResource(GetResource { id: resource_id }))
        .await
        .unwrap()
    else {
        panic!("resource");
    };
    assert_eq!(book.creators.len(), 4);
    assert_eq!(book.creators[0].role, CreatorRole::Author);
    assert_eq!(book.creators[0].order, 0);
    assert_eq!(book.creators[1].role, CreatorRole::Author);
    assert_eq!(book.creators[1].order, 1);
    assert_eq!(book.creators[2].role, CreatorRole::Editor);
    assert_eq!(book.creators[2].order, 2);
    assert_eq!(book.creators[3].role, CreatorRole::Translator);
    assert_eq!(book.creators[3].order, 3);

    let Outcome::ListPersons(people) = state
        .execute(Command::ListPersons(ListPersons))
        .await
        .unwrap()
    else {
        panic!("people");
    };
    assert_eq!(people.len(), 4);
    let by_id: std::collections::HashMap<_, _> = people
        .iter()
        .map(|person| (person.id, person.name.display()))
        .collect();
    assert_eq!(by_id.get(&book.creators[0].person_id).map(String::as_str), Some("Ada Lovelace"));
    assert_eq!(by_id.get(&book.creators[1].person_id).map(String::as_str), Some("Grace Hopper"));
    assert_eq!(by_id.get(&book.creators[2].person_id).map(String::as_str), Some("Donald Knuth"));
    assert_eq!(by_id.get(&book.creators[3].person_id).map(String::as_str), Some("Alan Turing"));

    let series_id = book.series_id.expect("series linked");
    assert_eq!(book.series_number.as_ref().map(|s| s.as_str()), Some("3"));
    let Outcome::GetResourceDetail(detail) = state
        .execute(Command::GetResourceDetail(GetResourceDetail { id: resource_id }))
        .await
        .unwrap()
    else {
        panic!("detail");
    };
    assert_eq!(
        detail.series_title.as_ref().map(|s| s.as_str()),
        Some("Computing Classics")
    );

    let expected_creators = book.creators.clone();
    let expected_people: Vec<_> = people
        .iter()
        .map(|person| (person.id, person.name.display()))
        .collect();
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

    let book = reopened.get_resource(resource_id).await.unwrap();
    assert_eq!(book.creators, expected_creators);
    assert_eq!(book.series_id, Some(series_id));
    let people = reopened.list_persons().await.unwrap();
    assert_eq!(people.len(), 4);
    let by_id: std::collections::HashMap<_, _> = people
        .iter()
        .map(|person| (person.id, person.name.display()))
        .collect();
    for (id, name) in expected_people {
        assert_eq!(by_id.get(&id).map(String::as_str), Some(name.as_str()));
    }
    assert_eq!(
        reopened.get_series(series_id).await.unwrap().title.as_str(),
        "Computing Classics"
    );
}

#[tokio::test]
async fn create_resource_propagates_creator_relation_statement_errors() {
    let store = SurrealStore::open_memory().await.unwrap();
    let missing = PersonId::generate();
    let result = store
        .create_resource(minimal_book(
            "Ghost authors",
            vec![CreatorSpec {
                person_id: missing,
                role: CreatorRole::Author,
                credited_as: None,
            }],
        ))
        .await;
    assert!(
        result.is_err(),
        "missing creator person must not report create success: {result:?}"
    );
    assert!(
        store.list_resources().await.unwrap().is_empty(),
        "failed create must not leave a resource row"
    );
}

#[tokio::test]
async fn imported_resource_write_rolls_back_new_deps_and_keeps_shared() {
    let store = SurrealStore::open_memory().await.unwrap();
    let shared = store
        .create_person(
            Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
            vec![],
        )
        .await
        .unwrap();
    let shared_resource = store
        .create_resource(minimal_book(
            "Already committed",
            vec![CreatorSpec {
                person_id: shared.id,
                role: CreatorRole::Author,
                credited_as: None,
            }],
        ))
        .await
        .unwrap();

    let new_person = Person::try_new(
        PersonId::generate(),
        Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap(),
        vec![],
    )
    .unwrap();
    let new_series = Series::try_new(
        SeriesId::generate(),
        NewSeries {
            title: NonEmptyStr::try_new("Should Not Persist").unwrap(),
        },
    );
    let missing = PersonId::generate();
    let err = store
        .create_imported_resource(ImportedResourceWrite {
            new_people: vec![new_person.clone()],
            new_series: Some(new_series.clone()),
            resource: NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new("Failed unit").unwrap(),
                creators: vec![
                    CreatorSpec {
                        person_id: new_person.id,
                        role: CreatorRole::Author,
                        credited_as: None,
                    },
                    CreatorSpec {
                        person_id: missing,
                        role: CreatorRole::Editor,
                        credited_as: None,
                    },
                ],
                publisher: None,
                published: None,
                accessed: None,
                parent_id: None,
                series_id: Some(new_series.id),
                series_number: Some(NonEmptyStr::try_new("1").unwrap()),
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
            },
        })
        .await
        .expect_err("missing creator must fail the imported write");

    assert!(
        !err.is_not_found(),
        "relation/statement failure should surface as a store error: {err}"
    );
    let people = store.list_persons().await.unwrap();
    assert_eq!(people.len(), 1);
    assert_eq!(people[0].id, shared.id);
    assert!(
        store.get_person(new_person.id).await.unwrap_err().is_not_found(),
        "rolled-back person must not remain"
    );
    assert!(
        store
            .get_series(new_series.id)
            .await
            .unwrap_err()
            .is_not_found(),
        "rolled-back series must not remain"
    );
    let resources = store.list_resources().await.unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].id, shared_resource.id);
    let kept = store.get_resource(shared_resource.id).await.unwrap();
    assert_eq!(kept.creators.len(), 1);
    assert_eq!(kept.creators[0].person_id, shared.id);
}

fn minimal_book(title: &str, creators: Vec<CreatorSpec>) -> NewResource {
    NewResource {
        kind: ResourceKind::Book,
        title: NonEmptyStr::try_new(title).unwrap(),
        creators,
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
