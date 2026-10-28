#![cfg(feature = "testing")]

//! Task 04: linked journal/issue/book/series creation and matching.

use std::collections::HashMap;

use oeuvre_app::command::{GetResource, GetResourceDetail, ImportBiblatex, ListResources};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, Outcome, Surface};
use oeuvre_core::{ResourceId, ResourceKind};

async fn import(source: &str) -> (AppState, oeuvre_core::ImportReport) {
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
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
    (state, report)
}

async fn resource(state: &AppState, id: ResourceId) -> oeuvre_core::Resource {
    let Outcome::GetResource(r) = state
        .execute(Command::GetResource(GetResource { id }))
        .await
        .unwrap()
    else {
        panic!("get");
    };
    r
}

async fn all_resources(state: &AppState) -> Vec<oeuvre_core::ResourceSummary> {
    let Outcome::ListResources(rows) = state
        .execute(Command::ListResources(ListResources))
        .await
        .unwrap()
    else {
        panic!("list");
    };
    rows
}

#[tokio::test]
async fn multi_article_same_issue_shares_one_journal_and_one_issue() {
    let source = r#"
        @article{a1, title={First}, author={Ada, Lovelace},
          journaltitle={Nature}, volume={12}, number={3}, date={2020}}
        @article{a2, title={Second}, author={Hopper, Grace},
          journaltitle={Nature}, volume={12}, number={3}, date={2020}}
    "#;
    let (state, report) = import(source).await;
    assert_eq!(report.imported.len(), 2, "source entries only; inferred containers excluded");
    assert!(report.conflicted.is_empty());

    let rows = all_resources(&state).await;
    let journals: Vec<_> = rows
        .iter()
        .filter(|r| r.kind == ResourceKind::Journal)
        .collect();
    let issues: Vec<_> = rows
        .iter()
        .filter(|r| r.kind == ResourceKind::Issue)
        .collect();
    assert_eq!(journals.len(), 1);
    assert_eq!(journals[0].title.as_str(), "Nature");
    assert_eq!(issues.len(), 1);

    let issue_id = issues[0].id;
    let journal_id = journals[0].id;
    let issue = resource(&state, issue_id).await;
    assert_eq!(issue.parent_id, Some(journal_id));
    assert_eq!(issue.volume.as_deref(), Some("12"));
    assert_eq!(issue.issue.as_deref(), Some("3"));

    for key in ["a1", "a2"] {
        let id = report
            .imported
            .iter()
            .find(|e| e.entry_key == key)
            .unwrap()
            .resource_id;
        let article = resource(&state, id).await;
        assert_eq!(article.parent_id, Some(issue_id), "{key} should parent to the shared issue");
        assert_eq!(article.kind, ResourceKind::JournalArticle);
    }
}

#[tokio::test]
async fn distinct_volumes_create_distinct_issues_under_same_journal() {
    let source = r#"
        @article{v1, title={Vol one}, author={Ada, Lovelace},
          journaltitle={Science}, volume={1}, number={1}}
        @article{v2, title={Vol two}, author={Hopper, Grace},
          journaltitle={Science}, volume={2}, number={1}}
    "#;
    let (state, report) = import(source).await;
    assert_eq!(report.imported.len(), 2);

    let rows = all_resources(&state).await;
    assert_eq!(
        rows.iter()
            .filter(|r| r.kind == ResourceKind::Journal)
            .count(),
        1
    );
    let issues: Vec<_> = rows
        .iter()
        .filter(|r| r.kind == ResourceKind::Issue)
        .collect();
    assert_eq!(issues.len(), 2);

    let mut parents = HashMap::new();
    for key in ["v1", "v2"] {
        let id = report
            .imported
            .iter()
            .find(|e| e.entry_key == key)
            .unwrap()
            .resource_id;
        let article = resource(&state, id).await;
        parents.insert(key, article.parent_id.expect("parent"));
    }
    assert_ne!(parents["v1"], parents["v2"]);
}

#[tokio::test]
async fn missing_issue_info_links_article_directly_to_journal() {
    let source = r#"
        @article{only, title={No issue fields}, author={Ada, Lovelace},
          journaltitle={Direct Journal}}
    "#;
    let (state, report) = import(source).await;
    assert_eq!(report.imported.len(), 1);
    let rows = all_resources(&state).await;
    assert_eq!(
        rows.iter()
            .filter(|r| r.kind == ResourceKind::Journal)
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r.kind == ResourceKind::Issue)
            .count(),
        0,
        "no speculative issue without volume/number"
    );
    let article = resource(&state, report.imported[0].resource_id).await;
    let journal = rows
        .iter()
        .find(|r| r.kind == ResourceKind::Journal)
        .unwrap();
    assert_eq!(article.parent_id, Some(journal.id));
}

#[tokio::test]
async fn weak_journal_identity_does_not_fabricate_a_match() {
    let source = r#"
        @article{orphan, title={No venue}, author={Ada, Lovelace}, pages={1--2}}
    "#;
    let (state, report) = import(source).await;
    assert_eq!(report.imported.len(), 1);
    let article = resource(&state, report.imported[0].resource_id).await;
    assert!(article.parent_id.is_none());
    let rows = all_resources(&state).await;
    assert!(
        rows.iter()
            .all(|r| r.kind != ResourceKind::Journal && r.kind != ResourceKind::Issue)
    );
}

#[tokio::test]
async fn parent_before_child_and_child_before_parent_are_equivalent() {
    let parent_first = r#"
        @periodical{nat, title={Nature Letters}, issn={1234-5678}}
        @article{paper, title={A paper}, author={Ada, Lovelace},
          crossref={nat}, volume={5}, number={2}}
    "#;
    let child_first = r#"
        @article{paper, title={A paper}, author={Ada, Lovelace},
          crossref={nat}, volume={5}, number={2}}
        @periodical{nat, title={Nature Letters}, issn={1234-5678}}
    "#;

    let (state_a, report_a) = import(parent_first).await;
    let (state_b, report_b) = import(child_first).await;
    // Source entries are accounted for as imported or reused (child-first may
    // infer the journal, then reuse it when the @periodical entry arrives).
    assert_eq!(report_a.imported.len() + report_a.reused.len(), 2);
    assert_eq!(report_b.imported.len() + report_b.reused.len(), 2);

    async fn shape(state: &AppState, report: &oeuvre_core::ImportReport) -> (ResourceKind, Option<(Option<String>, Option<String>)>) {
        let paper_id = report
            .imported
            .iter()
            .find(|e| e.entry_key == "paper")
            .unwrap()
            .resource_id;
        let paper = resource(state, paper_id).await;
        let parent = resource(state, paper.parent_id.expect("linked")).await;
        (
            parent.kind,
            Some((parent.volume.clone(), parent.issue.clone())),
        )
    }

    assert_eq!(shape(&state_a, &report_a).await, shape(&state_b, &report_b).await);

    for state in [&state_a, &state_b] {
        let rows = all_resources(state).await;
        assert_eq!(
            rows.iter()
                .filter(|r| r.kind == ResourceKind::Journal)
                .count(),
            1
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.kind == ResourceKind::Issue)
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn chapter_links_to_imported_book_and_series_survives() {
    let source = r#"
        @book{parent, title={Collected Essays}, editor={Hopper, Grace},
          isbn={978-0-306-40615-7}, series={Essay Series}, number={1}}
        @incollection{chapter, title={A chapter}, author={Lovelace, Ada},
          crossref={parent}, pages={10--20}}
    "#;
    let (state, report) = import(source).await;
    assert_eq!(report.imported.len(), 2);

    let book_id = report
        .imported
        .iter()
        .find(|e| e.entry_key == "parent")
        .unwrap()
        .resource_id;
    let chapter_id = report
        .imported
        .iter()
        .find(|e| e.entry_key == "chapter")
        .unwrap()
        .resource_id;
    let book = resource(&state, book_id).await;
    let chapter = resource(&state, chapter_id).await;
    assert_eq!(book.kind, ResourceKind::Book);
    assert_eq!(chapter.kind, ResourceKind::BookSection);
    assert_eq!(chapter.parent_id, Some(book_id));
    assert_eq!(book.series_id, chapter.series_id);
    assert!(book.series_id.is_some());
    let Outcome::GetResourceDetail(detail) = state
        .execute(Command::GetResourceDetail(GetResourceDetail { id: book_id }))
        .await
        .unwrap()
    else {
        panic!("detail");
    };
    assert_eq!(
        detail.series_title.as_ref().map(|s| s.as_str()),
        Some("Essay Series")
    );
    assert!(
        !report
            .warnings
            .iter()
            .any(|w| w.code.as_deref() == Some("import.reference_no_parent_link")),
        "crossref to a publication entry should create a parent link"
    );
}

#[tokio::test]
async fn xdata_is_metadata_only_not_a_parent_link() {
    let source = r#"
        @xdata{shared, publisher={Shared Press}, location={London}}
        @book{work, title={Standalone}, author={Ada, Lovelace}, xdata={shared}}
    "#;
    let (state, report) = import(source).await;
    assert_eq!(report.imported.len(), 1);
    let book = resource(&state, report.imported[0].resource_id).await;
    assert!(book.parent_id.is_none());
    assert_eq!(book.publisher.as_deref(), Some("Shared Press"));
    assert_eq!(book.place.as_deref(), Some("London"));
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code.as_deref() == Some("import.reference_no_parent_link"))
    );
}

#[tokio::test]
async fn ambiguous_journal_titles_conflict_rather_than_merge() {
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    // Seed two journals with the same title but different ISSNs.
    let Outcome::ImportBiblatex(seed) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: r#"
                @periodical{j1, title={Common Name}, issn={1111-1111}}
                @periodical{j2, title={Common Name}, issn={2222-2222}}
            "#
            .into(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("seed");
    };
    assert_eq!(seed.imported.len(), 2);

    let Outcome::ImportBiblatex(report) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: r#"
                @article{ambig, title={Which journal?}, author={Ada, Lovelace},
                  journaltitle={Common Name}, volume={1}, number={1}}
            "#
            .into(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("import");
    };

    assert!(
        !report.conflicted.is_empty()
            || report
                .skipped
                .iter()
                .any(|i| i.entry_key.as_deref() == Some("ambig")),
        "ambiguous title must not silently attach to one journal"
    );
    if let Some(entry) = report.imported.iter().find(|e| e.entry_key == "ambig") {
        let article = resource(&state, entry.resource_id).await;
        assert!(
            article.parent_id.is_none(),
            "must not silently pick a parent among ambiguous matches"
        );
    }
}

#[tokio::test]
async fn conflicting_issn_does_not_overwrite_existing_journal() {
    let state = AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    let Outcome::ImportBiblatex(_) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: "@periodical{j1, title={Original Title}, issn={9999-9999}}".into(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("seed");
    };

    let Outcome::ImportBiblatex(report) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: r#"
                @article{clash, title={Paper}, author={Ada, Lovelace},
                  journaltitle={Different Title}, issn={9999-9999}, volume={1}, number={1}}
            "#
            .into(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("import");
    };

    assert!(
        !report.conflicted.is_empty()
            || report.warnings.iter().any(|w| {
                w.entry_key.as_deref() == Some("clash")
                    && w.message.to_lowercase().contains("conflict")
            })
            || report
                .skipped
                .iter()
                .any(|i| i.entry_key.as_deref() == Some("clash")),
        "ISSN identity with conflicting title must surface a conflict"
    );
    let rows = all_resources(&state).await;
    let journals: Vec<_> = rows
        .iter()
        .filter(|r| r.kind == ResourceKind::Journal)
        .collect();
    assert!(
        journals
            .iter()
            .any(|j| j.title.as_str() == "Original Title"),
        "existing journal title must not be overwritten"
    );
}
