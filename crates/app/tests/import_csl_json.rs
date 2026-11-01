#![cfg(feature = "testing")]

//! Task 07: CSL-JSON parser + shared pipeline equivalence + batching/cancel.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ibex_app::command::{GetResource, ImportBiblatex, ImportCslJson, ListPersons, ListResources};
use ibex_app::import::{ImportHooks, parse_biblatex, parse_csl_json, save_draft_with_hooks};
use ibex_app::testing::{MemorySink, MemoryStore};
use ibex_app::{AppState, Command, Outcome, Surface};
use ibex_core::{ImportFormat, ImportProgress, ResourceKind, import_codes};

fn app_with(store: MemoryStore) -> AppState {
    AppState::builder(store)
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build()
}

fn app() -> AppState {
    app_with(MemoryStore::new())
}

const BIB_FIXTURE: &str = r#"
@book{lovelace1843,
  author = {Lovelace, Ada},
  title = {Notes on the Analytical Engine},
  publisher = {Scientific Memoirs},
  year = {1843},
  doi = {10.1000/notes}
}
@article{hopper1952,
  author = {Hopper, Grace},
  title = {The Education of a Computer},
  journaltitle = {Proceedings of the ACM},
  volume = {1},
  number = {2},
  pages = {243--249},
  year = {1952},
  doi = {10.1145/609.123}
}
"#;

const CSL_FIXTURE: &str = r#"[
  {
    "id": "lovelace1843",
    "type": "book",
    "title": "Notes on the Analytical Engine",
    "author": [{"family": "Lovelace", "given": "Ada"}],
    "publisher": "Scientific Memoirs",
    "issued": {"date-parts": [[1843]]},
    "DOI": "10.1000/notes"
  },
  {
    "id": "hopper1952",
    "type": "article-journal",
    "title": "The Education of a Computer",
    "author": [{"family": "Hopper", "given": "Grace"}],
    "container-title": "Proceedings of the ACM",
    "volume": "1",
    "issue": "2",
    "page": "243-249",
    "issued": {"date-parts": [[1952]]},
    "DOI": "10.1145/609.123"
  }
]"#;

#[test]
fn parse_csl_json_maps_book_and_article_without_store() {
    let draft = parse_csl_json(CSL_FIXTURE).expect("parse");
    assert_eq!(draft.provenance.format, ImportFormat::CslJson);
    assert_eq!(draft.entries.len(), 2);
    assert!(draft.source_fingerprint.is_some());

    let book = draft
        .entries
        .iter()
        .find(|e| e.source_key() == "lovelace1843")
        .expect("book");
    assert_eq!(book.kind(), ResourceKind::Book);
    assert_eq!(book.title().as_str(), "Notes on the Analytical Engine");
    assert_eq!(book.unresolved_creators.len(), 1);
    assert_eq!(book.unresolved_creators[0].name.display(), "Ada Lovelace");
    assert_eq!(book.resource.doi.as_deref(), Some("10.1000/notes"));
    assert!(book.resource.creators.is_empty());
    assert!(book.resource.series_id.is_none());

    let article = draft
        .entries
        .iter()
        .find(|e| e.source_key() == "hopper1952")
        .expect("article");
    assert_eq!(article.kind(), ResourceKind::JournalArticle);
    assert_eq!(
        article.resource.metadata.venue.as_deref(),
        Some("Proceedings of the ACM")
    );
    assert_eq!(article.resource.volume.as_deref(), Some("1"));
    assert_eq!(article.resource.issue.as_deref(), Some("2"));
}

#[test]
fn parse_csl_json_rejects_oversized_sources_and_bad_json() {
    let huge = format!("[{}]", " ".repeat(ibex_core::IMPORT_MAX_SOURCE_BYTES));
    let err = parse_csl_json(&huge).expect_err("too large");
    assert_eq!(err.code, import_codes::SOURCE_TOO_LARGE);

    let err = parse_csl_json("{not json").expect_err("syntax");
    assert_eq!(err.code, import_codes::SYNTAX);
}

#[test]
fn parse_csl_json_has_no_person_matching_or_db_writes() {
    // Parser returns unresolved creators only; resource.creators stays empty.
    let draft = parse_csl_json(
        r#"[{"id":"a","type":"book","title":"T","author":[{"family":"Ada","given":"L"}]}]"#,
    )
    .expect("parse");
    assert!(draft.entries[0].resource.creators.is_empty());
    assert_eq!(draft.entries[0].unresolved_creators.len(), 1);
}

#[tokio::test]
async fn equivalent_biblatex_and_csl_json_produce_equivalent_domain_graphs() {
    let bib_draft = parse_biblatex(BIB_FIXTURE).expect("bib");
    let csl_draft = parse_csl_json(CSL_FIXTURE).expect("csl");

    let bib_state = app();
    let csl_state = app();
    let bib_report = ibex_app::import::save_draft(&bib_state, bib_draft)
        .await
        .expect("save bib");
    let csl_report = ibex_app::import::save_draft(&csl_state, csl_draft)
        .await
        .expect("save csl");
    assert_eq!(bib_report.imported.len(), 2);
    assert_eq!(csl_report.imported.len(), 2);

    async fn graph_signature(state: &AppState) -> BTreeSet<String> {
        let Outcome::ListResources(rows) = state
            .execute(Command::ListResources(ListResources))
            .await
            .unwrap()
        else {
            panic!("list");
        };
        let mut sig = BTreeSet::new();
        for row in rows {
            let Outcome::GetResource(r) = state
                .execute(Command::GetResource(GetResource { id: row.id }))
                .await
                .unwrap()
            else {
                panic!("get");
            };
            let creators: Vec<_> = {
                let Outcome::ListPersons(people) = state
                    .execute(Command::ListPersons(ListPersons))
                    .await
                    .unwrap()
                else {
                    panic!("people");
                };
                r.creators
                    .iter()
                    .map(|c| {
                        let name = people
                            .iter()
                            .find(|p| p.id == c.person_id)
                            .map(|p| p.name.display().to_owned())
                            .unwrap_or_default();
                        format!("{:?}:{name}", c.role)
                    })
                    .collect()
            };
            let parent_kind = if let Some(pid) = r.parent_id {
                let Outcome::GetResource(p) = state
                    .execute(Command::GetResource(GetResource { id: pid }))
                    .await
                    .unwrap()
                else {
                    panic!("parent");
                };
                format!("{:?}", p.kind)
            } else {
                "none".into()
            };
            // Ignore generated ids; compare kind/title/ids/venue/volume/issue/parent kind/creators.
            sig.insert(format!(
                "{:?}|{}|doi={}|vol={}|iss={}|venue={}|parent={}|creators={}",
                r.kind,
                r.title.as_str(),
                r.doi.as_deref().unwrap_or(""),
                r.volume.as_deref().unwrap_or(""),
                r.issue.as_deref().unwrap_or(""),
                r.metadata.venue.as_deref().unwrap_or(""),
                parent_kind,
                creators.join(";"),
            ));
        }
        sig
    }

    let bib_sig = graph_signature(&bib_state).await;
    let csl_sig = graph_signature(&csl_state).await;
    assert_eq!(
        bib_sig, csl_sig,
        "domain graphs should match\nbib={bib_sig:?}\ncsl={csl_sig:?}"
    );
}

#[tokio::test]
async fn import_csl_json_command_uses_shared_save() {
    let state = app();
    let Outcome::ImportCslJson(report) = state
        .execute(Command::ImportCslJson(ImportCslJson {
            source_text: CSL_FIXTURE.into(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("import csl");
    };
    assert_eq!(report.imported.len(), 2);
    assert!(report.failed.is_empty());
}

#[tokio::test]
async fn repeated_creators_use_bounded_person_lookups() {
    let store = MemoryStore::new();
    let state = app_with(store.clone());

    // Seed one shared author so lookups hit the store.
    let first = r#"[{"id":"seed","type":"book","title":"Seed","author":[{"family":"Lovelace","given":"Ada"}]}]"#;
    let Outcome::ImportCslJson(_) = state
        .execute(Command::ImportCslJson(ImportCslJson {
            source_text: first.into(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("seed");
    };

    store.reset_query_counts();
    let mut items = Vec::new();
    for i in 0..40 {
        items.push(format!(
            r#"{{"id":"b{i}","type":"book","title":"Book {i}","author":[{{"family":"Lovelace","given":"Ada"}}]}}"#
        ));
    }
    let source = format!("[{}]", items.join(","));
    let Outcome::ImportCslJson(report) = state
        .execute(Command::ImportCslJson(ImportCslJson {
            source_text: source,
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("import");
    };
    assert_eq!(report.imported.len(), 40);

    let finds = store.find_persons_by_name_count();
    let lists = store.list_persons_count();
    // Batched: at most one list_persons (or a small constant of finds), not one per entry.
    assert!(
        finds + lists <= 2,
        "expected batched person resolution, finds={finds} lists={lists}"
    );
}

#[tokio::test]
async fn identity_seed_uses_batched_resource_reads() {
    let store = MemoryStore::new();
    let state = app_with(store.clone());

    // Pre-populate library so seed_identity_index has work to do.
    for i in 0..25 {
        let source = format!(
            r#"[{{"id":"pre{i}","type":"book","title":"Pre {i}","DOI":"10.1000/pre{i}"}}]"#
        );
        let Outcome::ImportCslJson(_) = state
            .execute(Command::ImportCslJson(ImportCslJson {
                source_text: source,
                choices: Default::default(),
            }))
            .await
            .unwrap()
        else {
            panic!("pre");
        };
    }

    store.reset_query_counts();
    let source = r#"[{"id":"new","type":"book","title":"New","DOI":"10.1000/new"}]"#;
    let Outcome::ImportCslJson(report) = state
        .execute(Command::ImportCslJson(ImportCslJson {
            source_text: source.into(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("import");
    };
    assert_eq!(report.imported.len(), 1);

    let gets = store.get_resource_count();
    let batches = store.list_resources_for_import_count();
    // Batched seed should not N+1 get_resource for every library row.
    assert!(
        batches >= 1,
        "expected list_resources_for_import usage, batches={batches}"
    );
    assert!(
        gets < 25,
        "expected bounded get_resource during seed/import, gets={gets}"
    );
}

#[tokio::test]
async fn cancellation_between_commits_leaves_complete_units_and_truthful_report() {
    let state = app();
    let cancel = Arc::new(AtomicBool::new(false));
    let progress_ticks = Arc::new(AtomicUsize::new(0));
    let ticks = progress_ticks.clone();
    let cancel_flag = cancel.clone();

    let mut items = Vec::new();
    for i in 0..8 {
        items.push(format!(
            r#"{{"id":"c{i}","type":"book","title":"Cancel {i}","author":[{{"family":"Ada","given":"L"}}]}}"#
        ));
    }
    let source = format!("[{}]", items.join(","));
    let draft = parse_csl_json(&source).expect("parse");

    let hooks = ImportHooks {
        cancel: Some(cancel.clone()),
        on_progress: Some(Arc::new(move |p: ImportProgress| {
            ticks.fetch_add(1, Ordering::SeqCst);
            if p.completed_entries >= 3 {
                cancel_flag.store(true, Ordering::SeqCst);
            }
        })),
    };

    let report = save_draft_with_hooks(&state, draft, Default::default(), hooks)
        .await
        .expect("save");

    assert!(
        report.imported.len() >= 3,
        "at least the committed units before cancel: {report:?}"
    );
    assert!(
        !report.not_attempted.is_empty(),
        "remaining entries marked not-attempted: {report:?}"
    );
    assert!(
        report.not_attempted.iter().any(|i| {
            i.code.as_deref() == Some(import_codes::CANCELLED)
                || i.code.as_deref() == Some(import_codes::ENTRY_NOT_ATTEMPTED)
        }),
        "{:?}",
        report.not_attempted
    );
    assert!(
        progress_ticks.load(Ordering::SeqCst) >= 3,
        "progress should fire between commits"
    );

    // Retry resumes safely without duplicating committed entries.
    let Outcome::ImportCslJson(retry) = state
        .execute(Command::ImportCslJson(ImportCslJson {
            source_text: source,
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("retry");
    };
    assert_eq!(
        retry.reused.len(),
        report.imported.len(),
        "committed units reused on retry: {retry:?}"
    );
    assert_eq!(
        retry.imported.len() + retry.reused.len(),
        8,
        "full set accounted for: {retry:?}"
    );
}

#[tokio::test]
async fn biblatex_still_imports_through_shared_path() {
    let state = app();
    let Outcome::ImportBiblatex(report) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: BIB_FIXTURE.into(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("bib");
    };
    assert_eq!(report.imported.len(), 2);
}
