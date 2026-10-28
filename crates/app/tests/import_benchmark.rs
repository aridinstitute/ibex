#![cfg(feature = "testing")]

//! Task 07 micro-benchmarks (methodology recorded; not Criterion).
//!
//! Methodology:
//! - Backend: in-memory [`MemoryStore`] (no Surreal disk/network)
//! - Warmup: one discarded import of the same shape
//! - Measurement: wall time via `std::time::Instant` for parse + save
//! - Input: N books sharing one author (repeated creators) plus unique DOIs
//! - Query counts: `MemoryStore` counters after `reset_query_counts`
//! - Peak memory: not measured in this harness (RSS unavailable portably here)
//!
//! Run: `rustup run stable cargo test -p oeuvre-app --features testing --test import_benchmark -- --nocapture`

use std::time::Instant;

use oeuvre_app::command::ImportCslJson;
use oeuvre_app::import::parse_csl_json;
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, Outcome, Surface};

fn csl_library(n: usize) -> String {
    let mut items = Vec::with_capacity(n);
    for i in 0..n {
        items.push(format!(
            r#"{{"id":"b{i}","type":"book","title":"Title {i}","author":[{{"family":"Lovelace","given":"Ada"}}],"DOI":"10.1000/b{i}"}}"#
        ));
    }
    format!("[{}]", items.join(","))
}

#[tokio::test]
async fn record_import_timings_and_query_counts() {
    let n = 200;
    let source = csl_library(n);

    let parse_start = Instant::now();
    let draft = parse_csl_json(&source).expect("parse");
    let parse_ms = parse_start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(draft.entries.len(), n);

    // Warmup
    {
        let state = AppState::builder(MemoryStore::new())
            .surface(Surface::Test)
            .sink(MemorySink::new())
            .build();
        let _ = state
            .execute(Command::ImportCslJson(ImportCslJson {
                source_text: source.clone(),
                choices: Default::default(),
            }))
            .await
            .unwrap();
    }

    let store = MemoryStore::new();
    let state = AppState::builder(store.clone())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    store.reset_query_counts();
    let save_start = Instant::now();
    let Outcome::ImportCslJson(report) = state
        .execute(Command::ImportCslJson(ImportCslJson {
            source_text: source.clone(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("import");
    };
    let save_ms = save_start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(report.imported.len(), n);

    let finds = store.find_persons_by_name_count();
    let lists = store.list_persons_count();
    let gets = store.get_resource_count();
    let batches = store.list_resources_for_import_count();

    eprintln!("=== Task 07 import benchmark ===");
    eprintln!("input_entries={n}");
    eprintln!("input_bytes={}", source.len());
    eprintln!("parse_ms={parse_ms:.3}");
    eprintln!("save_ms={save_ms:.3}");
    eprintln!("find_persons_by_name={finds}");
    eprintln!("list_persons={lists}");
    eprintln!("get_resource={gets}");
    eprintln!("list_resources_for_import={batches}");
    eprintln!("methodology=MemoryStore; warmup=1; peak_rss=not_measured");

    // Batched person resolution: not one find per entry.
    assert!(finds + lists <= 2, "finds={finds} lists={lists}");
    assert!(batches >= 1, "expected batched identity seed");
}
