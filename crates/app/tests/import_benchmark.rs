#![cfg(feature = "testing")]

//! Task 07 follow-up: representative BibLaTeX / CSL-JSON import micro-benchmarks.
//!
//! ## Methodology
//!
//! - Backend: in-memory [`MemoryStore`] (no Surreal disk/network)
//! - Warmup: one discarded import of the same shape (skipped for conflict seed path)
//! - Measurement: wall time via `std::time::Instant` for parse + save
//! - Query counts: `MemoryStore` counters after `reset_query_counts` on the measured save
//! - Peak memory: Linux process `VmHWM` from `/proc/self/status` after the measured
//!   import (kilobytes). This is a process high-water mark, not an allocator-accurate
//!   per-import peak; it includes prior tests in the same process. Reported for
//!   relative comparison within a single `cargo test` invocation only.
//!
//! ## Scenarios
//!
//! - `shared_creators_containers`: N articles, shared journal/issue + two authors
//! - `unique_records`: N articles, distinct authors/journals/DOIs
//! - `conflicting_metadata`: seed N books, then import N books with same DOIs /
//!   different titles (publication_duplicate path)
//!
//! Run:
//! `rustup run stable cargo test -p ibex-app --features testing --test import_benchmark -- --nocapture`

use std::time::Instant;

use ibex_app::command::{ImportBiblatex, ImportCslJson};
use ibex_app::import::{parse_biblatex, parse_csl_json};
use ibex_app::testing::{MemorySink, MemoryStore};
use ibex_app::{AppState, Command, Outcome, Surface};
use ibex_core::import_codes;

#[derive(Clone, Copy)]
enum Format {
    Biblatex,
    CslJson,
}

impl Format {
    fn label(self) -> &'static str {
        match self {
            Format::Biblatex => "biblatex",
            Format::CslJson => "csl_json",
        }
    }
}

#[derive(Clone, Copy)]
enum Scenario {
    SharedCreatorsContainers,
    UniqueRecords,
    ConflictingMetadata,
}

impl Scenario {
    fn label(self) -> &'static str {
        match self {
            Scenario::SharedCreatorsContainers => "shared_creators_containers",
            Scenario::UniqueRecords => "unique_records",
            Scenario::ConflictingMetadata => "conflicting_metadata",
        }
    }
}

fn bib_shared(n: usize) -> String {
    let mut out = String::with_capacity(n * 180);
    for i in 0..n {
        out.push_str(&format!(
            "@article{{a{i}, title={{Title {i}}}, author={{Lovelace, Ada and Hopper, Grace}}, \
             journaltitle={{Nature}}, volume={{12}}, number={{3}}, year={{2020}}, \
             doi={{10.1000/shared-{i}}}}}\n"
        ));
    }
    out
}

fn bib_unique(n: usize) -> String {
    let mut out = String::with_capacity(n * 200);
    for i in 0..n {
        out.push_str(&format!(
            "@article{{u{i}, title={{Unique Title {i}}}, author={{Author{i}, First}}, \
             journaltitle={{Journal {i}}}, volume={{1}}, number={{1}}, year={{2020}}, \
             doi={{10.1000/unique-{i}}}}}\n"
        ));
    }
    out
}

fn bib_books_doi(prefix: &str, n: usize) -> String {
    let mut out = String::with_capacity(n * 140);
    for i in 0..n {
        out.push_str(&format!(
            "@book{{{prefix}{i}, title={{Book {prefix} {i}}}, author={{Lovelace, Ada}}, \
             year={{1843}}, doi={{10.1000/conflict-{i}}}}}\n"
        ));
    }
    out
}

fn csl_shared(n: usize) -> String {
    let mut items = Vec::with_capacity(n);
    for i in 0..n {
        items.push(format!(
            r#"{{"id":"a{i}","type":"article-journal","title":"Title {i}","author":[{{"family":"Lovelace","given":"Ada"}},{{"family":"Hopper","given":"Grace"}}],"container-title":"Nature","volume":"12","issue":"3","issued":{{"date-parts":[[2020]]}},"DOI":"10.1000/shared-{i}"}}"#
        ));
    }
    format!("[{}]", items.join(","))
}

fn csl_unique(n: usize) -> String {
    let mut items = Vec::with_capacity(n);
    for i in 0..n {
        items.push(format!(
            r#"{{"id":"u{i}","type":"article-journal","title":"Unique Title {i}","author":[{{"family":"Author{i}","given":"First"}}],"container-title":"Journal {i}","volume":"1","issue":"1","issued":{{"date-parts":[[2020]]}},"DOI":"10.1000/unique-{i}"}}"#
        ));
    }
    format!("[{}]", items.join(","))
}

fn csl_books_doi(prefix: &str, n: usize) -> String {
    let mut items = Vec::with_capacity(n);
    for i in 0..n {
        items.push(format!(
            r#"{{"id":"{prefix}{i}","type":"book","title":"Book {prefix} {i}","author":[{{"family":"Lovelace","given":"Ada"}}],"issued":{{"date-parts":[[1843]]}},"DOI":"10.1000/conflict-{i}"}}"#
        ));
    }
    format!("[{}]", items.join(","))
}

fn source(format: Format, scenario: Scenario, n: usize) -> String {
    match (format, scenario) {
        (Format::Biblatex, Scenario::SharedCreatorsContainers) => bib_shared(n),
        (Format::Biblatex, Scenario::UniqueRecords) => bib_unique(n),
        (Format::Biblatex, Scenario::ConflictingMetadata) => bib_books_doi("c", n),
        (Format::CslJson, Scenario::SharedCreatorsContainers) => csl_shared(n),
        (Format::CslJson, Scenario::UniqueRecords) => csl_unique(n),
        (Format::CslJson, Scenario::ConflictingMetadata) => csl_books_doi("c", n),
    }
}

fn seed_source(format: Format, n: usize) -> String {
    match format {
        Format::Biblatex => bib_books_doi("seed", n),
        Format::CslJson => csl_books_doi("seed", n),
    }
}

fn app(store: MemoryStore) -> AppState {
    AppState::builder(store)
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build()
}

fn peak_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        let Some(rest) = line.strip_prefix("VmHWM:") else {
            continue;
        };
        return rest.split_whitespace().next()?.parse().ok();
    }
    None
}

async fn import(format: Format, state: &AppState, text: &str) -> ibex_core::ImportReport {
    match format {
        Format::Biblatex => {
            let Outcome::ImportBiblatex(report) = state
                .execute(Command::ImportBiblatex(ImportBiblatex {
                    source_text: text.to_owned(),
                    choices: Default::default(),
                }))
                .await
                .expect("biblatex import")
            else {
                panic!("unexpected outcome");
            };
            report
        }
        Format::CslJson => {
            let Outcome::ImportCslJson(report) = state
                .execute(Command::ImportCslJson(ImportCslJson {
                    source_text: text.to_owned(),
                    choices: Default::default(),
                }))
                .await
                .expect("csl import")
            else {
                panic!("unexpected outcome");
            };
            report
        }
    }
}

fn parse_entries(format: Format, text: &str) -> usize {
    match format {
        Format::Biblatex => parse_biblatex(text).expect("parse").entries.len(),
        Format::CslJson => parse_csl_json(text).expect("parse").entries.len(),
    }
}

struct CaseResult {
    format: Format,
    scenario: Scenario,
    n: usize,
    input_bytes: usize,
    parse_ms: f64,
    save_ms: f64,
    imported: usize,
    conflicted: usize,
    find_persons_by_name: usize,
    list_persons: usize,
    get_resource: usize,
    list_resources_for_import: usize,
    peak_rss_kb: Option<u64>,
}

async fn run_case(format: Format, scenario: Scenario, n: usize) -> CaseResult {
    let text = source(format, scenario, n);

    let parse_start = Instant::now();
    let entry_count = parse_entries(format, &text);
    let parse_ms = parse_start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(entry_count, n);

    if !matches!(scenario, Scenario::ConflictingMetadata) {
        let warm = app(MemoryStore::new());
        let _ = import(format, &warm, &text).await;
    }

    let store = MemoryStore::new();
    let state = app(store.clone());
    if matches!(scenario, Scenario::ConflictingMetadata) {
        let seed = seed_source(format, n);
        let seeded = import(format, &state, &seed).await;
        assert_eq!(seeded.imported.len(), n, "seed import");
    }

    store.reset_query_counts();
    let save_start = Instant::now();
    let report = import(format, &state, &text).await;
    let save_ms = save_start.elapsed().as_secs_f64() * 1000.0;

    match scenario {
        Scenario::SharedCreatorsContainers | Scenario::UniqueRecords => {
            assert_eq!(
                report.imported.len(),
                n,
                "{} {} imported",
                format.label(),
                scenario.label()
            );
            assert!(
                report.conflicted.is_empty(),
                "unexpected conflicts: {:?}",
                report.conflicted
            );
        }
        Scenario::ConflictingMetadata => {
            assert!(
                report.imported.is_empty(),
                "conflicts must not import: {:?}",
                report.imported
            );
            assert_eq!(report.conflicted.len(), n);
            assert!(
                report
                    .conflicted
                    .iter()
                    .all(|i| { i.code.as_deref() == Some(import_codes::PUBLICATION_DUPLICATE) }),
                "expected publication_duplicate: {:?}",
                report.conflicted
            );
        }
    }

    // Shared creators should not issue one find-per-entry.
    if matches!(scenario, Scenario::SharedCreatorsContainers) {
        let finds = store.find_persons_by_name_count();
        let lists = store.list_persons_count();
        assert!(
            finds + lists <= 2,
            "shared creators should batch person lookups: finds={finds} lists={lists}"
        );
        assert!(
            store.list_resources_for_import_count() >= 1,
            "expected batched identity seed"
        );
    }

    CaseResult {
        format,
        scenario,
        n,
        input_bytes: text.len(),
        parse_ms,
        save_ms,
        imported: report.imported.len(),
        conflicted: report.conflicted.len(),
        find_persons_by_name: store.find_persons_by_name_count(),
        list_persons: store.list_persons_count(),
        get_resource: store.get_resource_count(),
        list_resources_for_import: store.list_resources_for_import_count(),
        peak_rss_kb: peak_rss_kb(),
    }
}

fn emit(r: &CaseResult) {
    eprintln!("=== import benchmark ===");
    eprintln!("format={}", r.format.label());
    eprintln!("scenario={}", r.scenario.label());
    eprintln!("input_entries={}", r.n);
    eprintln!("input_bytes={}", r.input_bytes);
    eprintln!("parse_ms={:.3}", r.parse_ms);
    eprintln!("save_ms={:.3}", r.save_ms);
    eprintln!("imported={}", r.imported);
    eprintln!("conflicted={}", r.conflicted);
    eprintln!("find_persons_by_name={}", r.find_persons_by_name);
    eprintln!("list_persons={}", r.list_persons);
    eprintln!("get_resource={}", r.get_resource);
    eprintln!("list_resources_for_import={}", r.list_resources_for_import);
    match r.peak_rss_kb {
        Some(kb) => eprintln!("peak_rss_kb={kb}"),
        None => eprintln!("peak_rss_kb=unavailable"),
    }
    eprintln!(
        "methodology=MemoryStore; warmup=1_except_conflict_seed; peak_rss=VmHWM_proc_self_status"
    );
}

#[tokio::test]
async fn record_import_timings_and_query_counts() {
    let n = 200;
    let formats = [Format::Biblatex, Format::CslJson];
    let scenarios = [
        Scenario::SharedCreatorsContainers,
        Scenario::UniqueRecords,
        Scenario::ConflictingMetadata,
    ];

    for format in formats {
        for scenario in scenarios {
            let result = run_case(format, scenario, n).await;
            emit(&result);
        }
    }
}
