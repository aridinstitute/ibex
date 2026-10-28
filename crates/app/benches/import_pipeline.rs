//! BibLaTeX / CSL-JSON import pipeline benchmarks (Task 07 follow-up).
//!
//! Cases mirror the plan: repeated creators/containers, unique records, and
//! conflicting metadata (DOI collision against a seeded library). Backend is
//! in-memory [`MemoryStore`] so timings isolate parse + orchestration rather
//! than Surreal disk I/O.
//!
//! ```text
//! rustup run stable cargo bench -p oeuvre-app --features testing --bench import_pipeline
//! OEUVRE_IMPORT_BENCH_SIZES=50,200 rustup run stable cargo bench -p oeuvre-app --features testing --bench import_pipeline
//! ```
//!
//! Query counts and peak-RSS methodology live in the companion harness
//! `crates/app/tests/import_benchmark.rs` (one-shot, `--nocapture`).

use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use oeuvre_app::command::{ImportBiblatex, ImportCslJson};
use oeuvre_app::import::{parse_biblatex, parse_csl_json};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, Surface};
use tokio::runtime::Runtime;

const DEFAULT_SIZES: &[usize] = &[50, 200];

#[derive(Clone, Copy, Debug)]
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

#[derive(Clone, Copy, Debug)]
enum Scenario {
    /// Articles sharing one journal/issue and two authors; unique DOIs.
    SharedCreatorsContainers,
    /// Distinct authors, journals, and DOIs per entry.
    UniqueRecords,
    /// Seed half the library, then import the other half with colliding DOIs.
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

fn sizes() -> Vec<usize> {
    match std::env::var("OEUVRE_IMPORT_BENCH_SIZES") {
        Ok(raw) => raw
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .filter(|&n| n > 0)
            .collect(),
        Err(_) => DEFAULT_SIZES.to_vec(),
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

fn bib_books_doi(prefix: &str, start: usize, n: usize) -> String {
    let mut out = String::with_capacity(n * 140);
    for i in start..start + n {
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

fn csl_books_doi(prefix: &str, start: usize, n: usize) -> String {
    let mut items = Vec::with_capacity(n);
    for i in start..start + n {
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
        (Format::Biblatex, Scenario::ConflictingMetadata) => bib_books_doi("c", 0, n),
        (Format::CslJson, Scenario::SharedCreatorsContainers) => csl_shared(n),
        (Format::CslJson, Scenario::UniqueRecords) => csl_unique(n),
        (Format::CslJson, Scenario::ConflictingMetadata) => csl_books_doi("c", 0, n),
    }
}

fn seed_source(format: Format, n: usize) -> String {
    // Seed uses the same DOIs as the measured conflicting import, different titles/keys.
    match format {
        Format::Biblatex => bib_books_doi("seed", 0, n),
        Format::CslJson => csl_books_doi("seed", 0, n),
    }
}

fn app(store: MemoryStore) -> AppState {
    AppState::builder(store)
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build()
}

async fn import_once(format: Format, state: &AppState, text: &str) {
    match format {
        Format::Biblatex => {
            let _ = state
                .execute(Command::ImportBiblatex(ImportBiblatex {
                    source_text: text.to_owned(),
                    choices: Default::default(),
                }))
                .await
                .expect("biblatex import");
        }
        Format::CslJson => {
            let _ = state
                .execute(Command::ImportCslJson(ImportCslJson {
                    source_text: text.to_owned(),
                    choices: Default::default(),
                }))
                .await
                .expect("csl import");
        }
    }
}

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("import_parse");
    group.warm_up_time(Duration::from_millis(300));
    group.measurement_time(Duration::from_secs(2));
    group.sample_size(20);

    for &n in &sizes() {
        group.throughput(Throughput::Elements(n as u64));
        for format in [Format::Biblatex, Format::CslJson] {
            let text = source(format, Scenario::SharedCreatorsContainers, n);
            eprintln!(
                "import_parse/{}/{n}: input_bytes={}",
                format.label(),
                text.len()
            );
            group.bench_with_input(BenchmarkId::new(format.label(), n), &text, |b, text| {
                match format {
                    Format::Biblatex => b.iter(|| {
                        let draft = parse_biblatex(black_box(text)).expect("parse");
                        black_box(draft.entries.len());
                    }),
                    Format::CslJson => b.iter(|| {
                        let draft = parse_csl_json(black_box(text)).expect("parse");
                        black_box(draft.entries.len());
                    }),
                }
            });
        }
    }
    group.finish();
}

fn bench_import_save(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    let mut group = c.benchmark_group("import_save");
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(3));
    group.sample_size(10);

    let scenarios = [
        Scenario::SharedCreatorsContainers,
        Scenario::UniqueRecords,
        Scenario::ConflictingMetadata,
    ];

    for &n in &sizes() {
        group.throughput(Throughput::Elements(n as u64));
        for format in [Format::Biblatex, Format::CslJson] {
            for scenario in scenarios {
                let id = format!("{}/{}/{}", format.label(), scenario.label(), n);
                let measured = source(format, scenario, n);
                eprintln!("import_save/{id}: input_bytes={}", measured.len());

                group.bench_with_input(BenchmarkId::from_parameter(&id), &measured, |b, text| {
                    // Setup (fresh store, optional DOI seed) is outside the timed
                    // closure so conflicting_metadata measures only the conflict path.
                    b.iter_batched(
                        || {
                            rt.block_on(async {
                                let store = MemoryStore::new();
                                let state = app(store);
                                if matches!(scenario, Scenario::ConflictingMetadata) {
                                    let seed = seed_source(format, n);
                                    import_once(format, &state, &seed).await;
                                }
                                state
                            })
                        },
                        |state| {
                            rt.block_on(async {
                                import_once(format, &state, text).await;
                            });
                        },
                        BatchSize::LargeInput,
                    );
                });
            }
        }
    }
    group.finish();
}

criterion_group!(benches, bench_parse, bench_import_save);
criterion_main!(benches);
