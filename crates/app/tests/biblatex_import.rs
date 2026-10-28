#![cfg(feature = "testing")]

use oeuvre_app::command::{GetResource, ImportBiblatex};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, Outcome, Surface};
use oeuvre_core::utc_year_from_millis;

#[tokio::test]
async fn imports_a_book_through_the_application_command() {
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    let source = r#"@book{lovelace1843,
        author = {Lovelace, Ada},
        title = {Notes on the Analytical Engine},
        publisher = {Scientific Memoirs},
        year = {1843},
        doi = {10.1000/notes},
        custom = {Retain me}
    }"#;

    let Outcome::ImportBiblatex(report) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: source.into(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("expected import report");
    };
    assert_eq!(report.imported.len(), 1);
    assert!(report.skipped.is_empty());
    assert!(report.warnings.is_empty());
    assert_eq!(report.imported[0].entry_key, "lovelace1843");

    let Outcome::GetResource(book) = state
        .execute(Command::GetResource(GetResource {
            id: report.imported[0].resource_id,
        }))
        .await
        .unwrap()
    else {
        panic!("expected imported resource");
    };
    assert_eq!(book.title.as_str(), "Notes on the Analytical Engine");
    assert_eq!(book.publisher.as_deref(), Some("Scientific Memoirs"));
    assert_eq!(book.creators.len(), 1);
    assert_eq!(book.metadata.citekey.as_deref(), Some("lovelace1843"));
    assert_eq!(book.doi.as_deref(), Some("10.1000/notes"));
    assert_eq!(
        book.metadata
            .source_fields
            .get("custom")
            .map(String::as_str),
        Some("Retain me")
    );
    assert_eq!(
        book.published
            .as_ref()
            .map(|d| utc_year_from_millis(d.instant().as_unix_ms())),
        Some(1843)
    );
}

#[tokio::test]
async fn imports_a_mixed_batch_and_reports_skips_without_losing_fields() {
    let sink = MemorySink::new();
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();
    let source = r#"
        @book{parent, title={Collected Essays}, editor={Hopper, Grace}, year={2020}}
        @incollection{chapter, title={A chapter}, author={Lovelace, Ada}, crossref={parent},
          subtitle={A subtitle}, titleaddon={Revised}, shorttitle={Chapter},
          location={London}, volume={II}, number={3}, pages={10--20}, pagetotal={xii+250},
          eid={e123}, edition={Second}, doi={10.1000/chapter}, url={https://example.org},
          eprint={1234.5678}, isbn={978-0-306-40615-7}, issn={1234-5678}, isrn={EXAMPLE-1},
          date={2024-03-15}, origdate={1960}, eventdate={2023-06-01}, urldate={2025-01-02},
          language={en}, type={Essay}, howpublished={Online},
          institution={Institute}, organization={Society}, school={History},
          abstract={A summary}, keywords={history, research}, note={Ignore note}, annotation={Ignore annotation}}
        @article{paper, title={A paper}, journaltitle={Journal}, date={2022},
          author={Lovelace, Ada}, translator={Turing, Alan}, volume={12}, number={4}}
        @online{web, title={A website}, url={https://example.org/web}}
        @thesis{broken, author={No, Title}}
        @patent{unsupported, title={A patent}}
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
    assert_eq!(report.imported.len(), 4);
    assert_eq!(report.skipped.len(), 2);
    assert!(
        report
            .skipped
            .iter()
            .any(|i| i.entry_key.as_deref() == Some("broken"))
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|i| i.entry_key.as_deref() == Some("unsupported"))
    );
    let id = report
        .imported
        .iter()
        .find(|e| e.entry_key == "chapter")
        .unwrap()
        .resource_id;
    let Outcome::GetResource(chapter) = state
        .execute(Command::GetResource(GetResource { id }))
        .await
        .unwrap()
    else {
        panic!("get");
    };
    assert_eq!(chapter.kind, oeuvre_core::ResourceKind::BookSection);
    assert_eq!(chapter.metadata.venue.as_deref(), Some("Collected Essays"));
    let data = serde_json::to_value(chapter.to_new()).unwrap();
    for (key, expected) in [
        ("subtitle", "A subtitle"),
        ("title_addon", "Revised"),
        ("short_title", "Chapter"),
        ("place", "London"),
        ("volume", "II"),
        ("issue", "3"),
        ("total_pages", "xii+250"),
        ("article_number", "e123"),
        ("edition", "Second"),
        ("doi", "10.1000/chapter"),
        ("url", "https://example.org"),
        ("eprint", "1234.5678"),
        ("language", "en"),
        ("document_type", "Essay"),
        ("publication_method", "Online"),
        ("institution", "Institute"),
        ("organization", "Society"),
        ("school", "History"),
        ("abstract_text", "A summary"),
    ] {
        assert_eq!(data[key], expected, "mapping {key}");
    }
    assert_eq!(chapter.pages.as_ref().map(|p| p.as_str()), Some("10–20"));
    assert!(chapter.pages.as_ref().unwrap().is_structured());
    assert_eq!(chapter.isbns[0].as_str(), "978-0-306-40615-7");
    assert_eq!(chapter.isbns[0].canonical.as_deref(), Some("9780306406157"));
    assert_eq!(chapter.issns[0].as_str(), "1234-5678");
    assert_eq!(chapter.isrns[0].as_str(), "EXAMPLE-1");
    assert_eq!(chapter.keywords, ["history", "research"]);
    assert_eq!(
        chapter.published.as_ref().unwrap().instant().as_unix_ms(),
        1_710_460_800_000
    );
    assert_eq!(
        chapter.published.as_ref().unwrap().precision(),
        oeuvre_core::DatePrecision::Day
    );
    assert!(chapter.original_date.is_some());
    assert_eq!(
        chapter.original_date.as_ref().unwrap().precision(),
        oeuvre_core::DatePrecision::Year
    );
    assert!(chapter.event_date.is_some());
    assert!(chapter.accessed.is_some());
    assert!(
        !chapter.accessed.as_ref().unwrap().claims_exact_time(),
        "date-only urldate must not claim an exact time"
    );
    assert!(chapter.tag_ids.is_empty());
    assert!(!chapter.metadata.source_fields.contains_key("note"));
    assert!(!chapter.metadata.source_fields.contains_key("annotation"));
    assert_eq!(
        chapter.metadata.provenance.note.as_deref(),
        Some("Ignore note")
    );
    assert_eq!(
        chapter.metadata.provenance.annotation.as_deref(),
        Some("Ignore annotation")
    );
    assert!(
        chapter
            .metadata
            .provenance
            .original_fields
            .contains_key("note")
    );
    let events = sink.take();
    let imports: Vec<_> = events
        .iter()
        .filter(|e| e.command() == "import_biblatex")
        .collect();
    assert_eq!(imports.len(), 1);
    assert_eq!(
        imports[0].get("import.imported_count"),
        Some(&oeuvre_app::FieldValue::from(4u64))
    );
}

#[tokio::test]
async fn invalid_syntax_duplicate_keys_and_reference_cycles_write_nothing() {
    for source in [
        "@book{valid,title={Valid}} @book{broken,title={Unclosed",
        "@book{same,title={First}} @book{same,title={Second}}",
        "@book{a,title={A},crossref={b}} @book{b,title={B},crossref={a}}",
    ] {
        let sink = MemorySink::new();
        let state = AppState::builder(MemoryStore::new())
            .surface(Surface::Test)
            .sink(sink.clone())
            .build();
        assert!(matches!(
            state
                .execute(Command::ImportBiblatex(ImportBiblatex {
                    source_text: source.into(),
                    choices: Default::default(),
                }))
                .await,
            Err(oeuvre_app::CommandError::Invalid(_))
        ));
        let Outcome::CountResources(counts) = state
            .execute(Command::CountResources(oeuvre_app::command::CountResources))
            .await
            .unwrap()
        else {
            panic!("counts");
        };
        assert_eq!(counts.all_items, 0);
        let events = sink.take();
        assert_eq!(events[0].command(), "import_biblatex");
        assert_eq!(events[0].status(), oeuvre_app::Status::Error);
    }
}

#[tokio::test]
async fn rejects_deep_reference_chains_even_when_parents_appear_first() {
    let mut source = "@book{e0,title={First}}\n".to_owned();
    for n in 1..70 {
        source.push_str(&format!(
            "@book{{e{n},title={{Entry}},crossref={{e{}}}}}\n",
            n - 1
        ));
    }
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    assert!(matches!(
        state
            .execute(Command::ImportBiblatex(ImportBiblatex {
                source_text: source,
                choices: Default::default(),
            }))
            .await,
        Err(oeuvre_app::CommandError::Invalid(_))
    ));
}

#[tokio::test]
async fn preserves_custom_fields_named_like_internal_reference_markers() {
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    let Outcome::ImportBiblatex(report) = state.execute(Command::ImportBiblatex(ImportBiblatex {
        source_text: "@book{one,title={Book},oeuvre_crossref={custom},oeuvre_import_crossref={also custom}}".into(),
        choices: Default::default(),
    })).await.unwrap() else { panic!("import"); };
    let Outcome::GetResource(resource) = state
        .execute(Command::GetResource(GetResource {
            id: report.imported[0].resource_id,
        }))
        .await
        .unwrap()
    else {
        panic!("resource");
    };
    assert_eq!(
        resource
            .metadata
            .source_fields
            .get("oeuvre_crossref")
            .map(String::as_str),
        Some("custom")
    );
    assert_eq!(
        resource
            .metadata
            .source_fields
            .get("oeuvre_import_crossref")
            .map(String::as_str),
        Some("also custom")
    );
    assert!(report.warnings.is_empty());
}

#[tokio::test]
async fn inherits_nearest_reference_date_and_does_not_abort_on_bad_entry_dates() {
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    let Outcome::ImportBiblatex(report) = state.execute(Command::ImportBiblatex(ImportBiblatex {
        source_text: "@book{grand,title={Grand},year={2020}} @book{parent,title={Parent},year={2021},crossref={grand}} @inbook{child,title={Child},crossref={parent}} @book{bad,title={Bad},year={bad}} @inbook{badchild,title={Bad child},crossref={bad}}".into(),
        choices: Default::default(),
    })).await.unwrap() else { panic!("import"); };
    // Invalid optional dates warn and retain source values; entries still import.
    assert_eq!(report.imported.len(), 5);
    assert!(report.skipped.is_empty());
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.entry_key.as_deref() == Some("bad"))
    );
    let id = report
        .imported
        .iter()
        .find(|e| e.entry_key == "child")
        .unwrap()
        .resource_id;
    let Outcome::GetResource(child) = state
        .execute(Command::GetResource(GetResource { id }))
        .await
        .unwrap()
    else {
        panic!("child");
    };
    assert_eq!(
        child
            .published
            .as_ref()
            .map(|d| utc_year_from_millis(d.instant().as_unix_ms())),
        Some(2021)
    );
}
