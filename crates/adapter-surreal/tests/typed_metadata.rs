//! Task 03: typed dates, pages, names, and provenance through SurrealDB.

use ibex_adapter_surreal::SurrealStore;
use ibex_app::command::{GetResource, ImportBiblatex, ListPersons};
use ibex_app::{AppState, Command, Outcome, Surface};
use ibex_core::{
    BibliographicDate, DatePrecision, Identifier, Name, NewResource, NonEmptyStr, Pages,
    ResourceKind, Store, UtcMillis,
};

#[tokio::test]
async fn dates_pages_names_and_provenance_round_trip_through_real_adapter() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library");
    let store = SurrealStore::open(&path).await.unwrap();
    let state = AppState::builder(store).surface(Surface::Test).build();
    let source = r#"
        @book{typed,
          title = {Typed Metadata},
          author = {van Beethoven, Ludwig and {World Bank} and Marty, Michael and others},
          date = {2020-06/2021-08},
          urldate = {2025-01-02},
          origdate = {1960},
          pages = {10--20},
          isbn = {978-0-306-40615-7, 0-306-40615-2},
          note = {Keep this note},
          annotation = {Keep this annotation},
          customunk = {unknown value}
        }
        @article{pages2, title={More pages}, pages={S10-S20}}
        @article{pages3, title={Roman}, pages={iv-xii}}
        @article{pages4, title={List}, pages={1-3, 8-10}}
        @article{pages5, title={Odd}, pages={passim}}
        @book{accent, title={Accent}, author={García, José}}
    "#;
    let Outcome::ImportBiblatex(report) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: source.into(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("import");
    };
    assert!(report.imported.len() >= 6);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.message.contains("and others")),
        "incomplete creator list should warn"
    );

    let typed_id = report
        .imported
        .iter()
        .find(|e| e.entry_key == "typed")
        .unwrap()
        .resource_id;
    let Outcome::GetResource(book) = state
        .execute(Command::GetResource(GetResource { id: typed_id }))
        .await
        .unwrap()
    else {
        panic!("get");
    };

    let published = book.published.as_ref().expect("published");
    assert_eq!(published.precision(), DatePrecision::Month);
    assert!(published.end().is_some());
    let accessed = book.accessed.as_ref().expect("accessed");
    assert_eq!(accessed.precision(), DatePrecision::Day);
    assert!(!accessed.claims_exact_time());
    assert_eq!(
        book.original_date.as_ref().unwrap().precision(),
        DatePrecision::Year
    );

    let pages = book.pages.as_ref().unwrap();
    assert_eq!(pages.as_str(), "10–20");
    assert_eq!(pages.spans[0].start.number, Some(10));
    assert_eq!(pages.spans[0].end.as_ref().unwrap().number, Some(20));
    assert_eq!(book.isbns.len(), 2);
    assert_eq!(book.isbns[0].as_str(), "978-0-306-40615-7");
    assert!(book.isbns[0].canonical.is_some());
    assert_eq!(
        book.metadata.provenance.note.as_deref(),
        Some("Keep this note")
    );
    assert_eq!(
        book.metadata.provenance.annotation.as_deref(),
        Some("Keep this annotation")
    );
    assert!(
        book.metadata
            .provenance
            .original_fields
            .contains_key("customunk")
    );

    let by_key = |key: &str| {
        report
            .imported
            .iter()
            .find(|e| e.entry_key == key)
            .unwrap()
            .resource_id
    };
    async fn pages_of(state: &AppState, id: ibex_core::ResourceId) -> Pages {
        let Outcome::GetResource(r) = state
            .execute(Command::GetResource(GetResource { id }))
            .await
            .unwrap()
        else {
            panic!("get pages");
        };
        r.pages.expect("pages")
    }
    let p2 = pages_of(&state, by_key("pages2")).await;
    assert_eq!(
        p2.spans[0].start.prefix.as_ref().map(|s| s.as_str()),
        Some("S")
    );
    let p3 = pages_of(&state, by_key("pages3")).await;
    assert_eq!(p3.spans[0].start.literal.as_str(), "iv");
    let p4 = pages_of(&state, by_key("pages4")).await;
    assert_eq!(p4.spans.len(), 2);
    let p5 = pages_of(&state, by_key("pages5")).await;
    assert_eq!(p5.as_str(), "passim");

    let Outcome::ListPersons(people) = state
        .execute(Command::ListPersons(ListPersons))
        .await
        .unwrap()
    else {
        panic!("people");
    };
    let beethoven = people
        .iter()
        .find(|p| p.name.last() == Some("Beethoven"))
        .expect("particle preserved");
    assert_eq!(beethoven.name.particle(), Some("van"));
    assert!(
        people
            .iter()
            .any(|p| p.name.is_literal() && p.name.display() == "World Bank"),
        "corporate name"
    );
    assert!(
        people.iter().all(|p| p.name.last() != Some("others")),
        "and others must not create a person"
    );
    let jose = people
        .iter()
        .find(|p| p.name.first() == Some("José"))
        .expect("accented display preserved");
    assert_eq!(jose.name.display(), "José García");

    drop(state);
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
    .expect("lock released");
    let again = reopened.get_resource(typed_id).await.unwrap();
    assert_eq!(again.pages.as_ref().map(|p| p.as_str()), Some("10–20"));
    assert_eq!(
        again.metadata.provenance.note.as_deref(),
        Some("Keep this note")
    );
    assert!(!again.accessed.as_ref().unwrap().claims_exact_time());
}

#[tokio::test]
async fn create_resource_round_trips_typed_pages_and_dates() {
    let store = SurrealStore::open_memory().await.unwrap();
    let pages = Pages::parse("1-3, 8-10").unwrap();
    let accessed = BibliographicDate::try_new_precise(
        UtcMillis::jan_1_utc(2025).unwrap(),
        None,
        DatePrecision::Day,
        ibex_core::DateCertainty::Exact,
        Some(NonEmptyStr::try_new("2025-01-01").unwrap()),
    )
    .unwrap();
    assert!(!accessed.claims_exact_time());
    let new = NewResource {
        kind: ResourceKind::Book,
        title: NonEmptyStr::try_new("Direct").unwrap(),
        creators: vec![],
        publisher: None,
        published: Some(
            BibliographicDate::try_new_precise(
                UtcMillis::jan_1_utc(2020).unwrap(),
                None,
                DatePrecision::Year,
                ibex_core::DateCertainty::Exact,
                Some(NonEmptyStr::try_new("2020").unwrap()),
            )
            .unwrap(),
        ),
        accessed: Some(accessed),
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
        pages: Some(pages),
        total_pages: None,
        article_number: None,
        edition: None,
        doi: None,
        url: None,
        eprint: None,
        isbns: vec![Identifier::from("978-0-306-40615-7")],
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
    };
    let created = store.create_resource(new.clone()).await.unwrap();
    let fetched = store.get_resource(created.id).await.unwrap();
    assert_eq!(fetched.to_new().pages, new.pages);
    assert_eq!(fetched.to_new().accessed, new.accessed);
    assert_eq!(fetched.to_new().isbns[0].as_str(), "978-0-306-40615-7");
}

#[tokio::test]
async fn person_particle_and_corporate_survive_storage() {
    let store = SurrealStore::open_memory().await.unwrap();
    let particle =
        Name::try_new_parts(Some("Ludwig"), Some("van"), Some("Beethoven"), None::<&str>).unwrap();
    let corp = Name::corporate("World Health Organization").unwrap();
    let p1 = store.create_person(particle.clone(), vec![]).await.unwrap();
    let p2 = store.create_person(corp.clone(), vec![]).await.unwrap();
    assert_eq!(store.get_person(p1.id).await.unwrap().name, particle);
    assert_eq!(store.get_person(p2.id).await.unwrap().name, corp);
    let matches = store.find_persons_by_name(&particle).await.unwrap();
    assert_eq!(matches.len(), 1);
}
