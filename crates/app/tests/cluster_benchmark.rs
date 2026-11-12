#![cfg(feature = "testing")]

//! Topic-clustering correctness and efficiency harness.
//!
//! ## Methodology
//!
//! - Algorithm cases use planted topic corpora (no store I/O)
//! - End-to-end cases use [`MemoryStore`] + `suggest_clusters` / `apply_clusters`
//! - Wall time via `std::time::Instant`
//! - Peak memory: Linux `VmHWM` from `/proc/self/status` (process high-water mark)
//! - Correctness gates: partition completeness, min topic size, purity, recovery
//!
//! Run:
//! `cargo test -p ibex-app --features testing --test cluster_benchmark -- --nocapture`
//!
//! Optional sizes: `IBEX_CLUSTER_BENCH_SIZES=24,96,192`

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use ibex_app::cluster::{
    ClusterDocument, ClusterOptions, DocumentText, SuggestClusters, TextSource, TopicReport,
    apply_clusters, cluster_documents, suggest_clusters,
};
use ibex_app::command::{CreateResource, ListCollectionForest};
use ibex_app::testing::{MemorySink, MemoryStore};
use ibex_app::{AppState, Command, Outcome, Surface};
use ibex_core::{NewResource, NonEmptyStr, ResourceId, ResourceKind};

const DEFAULT_SIZES: &[usize] = &[24, 96];
const PLANTED: &[&[&str]] = &[
    &[
        "neural",
        "network",
        "deep",
        "learning",
        "backpropagation",
        "transformer",
        "attention",
        "layers",
    ],
    &[
        "climate",
        "carbon",
        "emissions",
        "greenhouse",
        "warming",
        "ocean",
        "sequestration",
        "temperature",
    ],
    &[
        "quantum",
        "qubit",
        "entanglement",
        "superposition",
        "hamiltonian",
        "coherence",
        "circuit",
        "gates",
    ],
];

fn sizes() -> Vec<usize> {
    match std::env::var("IBEX_CLUSTER_BENCH_SIZES") {
        Ok(raw) => raw
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .filter(|&n| n >= PLANTED.len() * 4)
            .collect(),
        Err(_) => DEFAULT_SIZES.to_vec(),
    }
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

fn planted_corpus(total: usize) -> (Vec<ClusterDocument>, HashMap<ResourceId, usize>) {
    let per_topic = (total / PLANTED.len()).max(4);
    let mut docs = Vec::with_capacity(per_topic * PLANTED.len());
    let mut labels = HashMap::new();
    for (topic_i, vocab) in PLANTED.iter().enumerate() {
        for doc_i in 0..per_topic {
            let id = ResourceId::generate();
            let body = (0..8)
                .map(|k| vocab[(doc_i + k) % vocab.len()])
                .collect::<Vec<_>>()
                .join(" ");
            docs.push(ClusterDocument {
                resource_id: id,
                title: format!("topic{topic_i}-doc{doc_i}"),
                text: DocumentText {
                    body,
                    source: TextSource::Metadata,
                },
            });
            labels.insert(id, topic_i);
        }
    }
    (docs, labels)
}

fn partition_ok(report: &TopicReport, n: usize) {
    let mut seen = HashSet::new();
    for topic in &report.topics {
        for member in &topic.members {
            assert!(
                seen.insert(member.resource_id),
                "duplicate topic membership"
            );
        }
    }
    for member in &report.outliers {
        assert!(seen.insert(member.resource_id), "duplicate outlier");
    }
    assert_eq!(seen.len(), n, "incomplete partition");
}

fn purity(report: &TopicReport, planted: &HashMap<ResourceId, usize>) -> f64 {
    let mut weighted = 0.0;
    let mut total = 0.0;
    for topic in &report.topics {
        let mut counts: HashMap<usize, usize> = HashMap::new();
        for member in &topic.members {
            if let Some(&label) = planted.get(&member.resource_id) {
                *counts.entry(label).or_default() += 1;
            }
        }
        weighted += counts.values().copied().max().unwrap_or(0) as f64;
        total += topic.members.len() as f64;
    }
    if total == 0.0 { 0.0 } else { weighted / total }
}

fn recovery(report: &TopicReport, planted: &HashMap<ResourceId, usize>) -> f64 {
    let mut by_planted: HashMap<usize, Vec<ResourceId>> = HashMap::new();
    for (id, label) in planted {
        by_planted.entry(*label).or_default().push(*id);
    }
    if by_planted.is_empty() {
        return 0.0;
    }
    let mut scores = Vec::new();
    for ids in by_planted.values() {
        let best = report
            .topics
            .iter()
            .map(|topic| {
                topic
                    .members
                    .iter()
                    .filter(|m| ids.contains(&m.resource_id))
                    .count()
            })
            .max()
            .unwrap_or(0);
        scores.push(best as f64 / ids.len() as f64);
    }
    scores.iter().sum::<f64>() / scores.len() as f64
}

struct AlgoResult {
    n: usize,
    topics: usize,
    outliers: usize,
    purity: f64,
    recovery: f64,
    cluster_ms: f64,
    docs_per_sec: f64,
    peak_rss_kb: Option<u64>,
}

fn run_algo(n: usize) -> AlgoResult {
    let (docs, planted) = planted_corpus(n);
    let actual_n = docs.len();
    let min_topic_size = 4;

    // Warmup
    let _ = cluster_documents(
        &docs,
        ClusterOptions {
            min_topic_size,
            label_terms: 3,
            keyword_terms: 6,
        },
    );

    let start = Instant::now();
    let report = cluster_documents(
        &docs,
        ClusterOptions {
            min_topic_size,
            label_terms: 3,
            keyword_terms: 6,
        },
    )
    .expect("cluster");
    let cluster_ms = start.elapsed().as_secs_f64() * 1000.0;

    partition_ok(&report, actual_n);
    for topic in &report.topics {
        assert!(topic.members.len() >= min_topic_size);
        assert!(!topic.keywords.is_empty());
    }
    let p = purity(&report, &planted);
    let r = recovery(&report, &planted);
    assert!(
        p >= 0.85,
        "purity {p:.3} too low for n={actual_n}; topics={}",
        report.topics.len()
    );
    assert!(
        r >= 0.70,
        "recovery {r:.3} too low for n={actual_n}; topics={}",
        report.topics.len()
    );
    // Soft efficiency gate: catch pathological regressions without flake.
    assert!(
        cluster_ms < 30_000.0,
        "clustering {actual_n} docs took {cluster_ms:.1}ms (>30s)"
    );

    AlgoResult {
        n: actual_n,
        topics: report.topics.len(),
        outliers: report.outliers.len(),
        purity: p,
        recovery: r,
        cluster_ms,
        docs_per_sec: actual_n as f64 / (cluster_ms / 1000.0).max(1e-9),
        peak_rss_kb: peak_rss_kb(),
    }
}

fn emit_algo(r: &AlgoResult) {
    eprintln!("=== cluster algorithm benchmark ===");
    eprintln!("documents={}", r.n);
    eprintln!("topics={}", r.topics);
    eprintln!("outliers={}", r.outliers);
    eprintln!("purity={:.4}", r.purity);
    eprintln!("recovery={:.4}", r.recovery);
    eprintln!("cluster_ms={:.3}", r.cluster_ms);
    eprintln!("docs_per_sec={:.1}", r.docs_per_sec);
    match r.peak_rss_kb {
        Some(kb) => eprintln!("peak_rss_kb={kb}"),
        None => eprintln!("peak_rss_kb=unavailable"),
    }
    eprintln!("methodology=planted_topics; warmup=1; peak_rss=VmHWM_proc_self_status");
}

fn article(title: &str, body: &str) -> NewResource {
    NewResource {
        kind: ResourceKind::JournalArticle,
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
        school: None,
        abstract_text: Some(body.into()),
        keywords: vec![],
    }
}

async fn seed_library(state: &AppState, total: usize) -> usize {
    let (docs, _) = planted_corpus(total);
    for doc in &docs {
        let Outcome::CreateResource(_) = state
            .execute(Command::CreateResource(CreateResource {
                new: article(&doc.title, &doc.text.body),
            }))
            .await
            .expect("create")
        else {
            panic!("unexpected create outcome");
        };
    }
    docs.len()
}

struct E2eResult {
    n: usize,
    topics: usize,
    suggest_ms: f64,
    apply_ms: f64,
    get_resource: usize,
    peak_rss_kb: Option<u64>,
}

async fn run_e2e(n: usize) -> E2eResult {
    let warm_store = MemoryStore::new();
    let warm = AppState::builder(warm_store)
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    let _ = seed_library(&warm, n).await;
    let _ = suggest_clusters(
        &warm,
        SuggestClusters {
            min_topic_size: 4,
            ..SuggestClusters::default()
        },
    )
    .await;

    let store = MemoryStore::new();
    let state = AppState::builder(store.clone())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    let actual_n = seed_library(&state, n).await;

    store.reset_query_counts();
    let suggest_start = Instant::now();
    let suggestion = suggest_clusters(
        &state,
        SuggestClusters {
            min_topic_size: 4,
            ..SuggestClusters::default()
        },
    )
    .await
    .expect("suggest");
    let suggest_ms = suggest_start.elapsed().as_secs_f64() * 1000.0;
    let get_resource = store.get_resource_count();

    assert!(
        suggestion.report.topics.len() >= 2,
        "e2e topics={}",
        suggestion.report.topics.len()
    );
    assert!(
        suggest_ms < 60_000.0,
        "suggest_clusters for {actual_n} took {suggest_ms:.1}ms"
    );

    let apply_start = Instant::now();
    let applied = apply_clusters(&state, &suggestion, None)
        .await
        .expect("apply");
    let apply_ms = apply_start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(applied.len(), suggestion.report.topics.len());

    let Outcome::ListCollectionForest(forest) = state
        .execute(Command::ListCollectionForest(ListCollectionForest))
        .await
        .unwrap()
    else {
        panic!("forest");
    };
    assert_eq!(forest.len(), applied.len());

    E2eResult {
        n: actual_n,
        topics: suggestion.report.topics.len(),
        suggest_ms,
        apply_ms,
        get_resource,
        peak_rss_kb: peak_rss_kb(),
    }
}

fn emit_e2e(r: &E2eResult) {
    eprintln!("=== cluster e2e benchmark ===");
    eprintln!("documents={}", r.n);
    eprintln!("topics={}", r.topics);
    eprintln!("suggest_ms={:.3}", r.suggest_ms);
    eprintln!("apply_ms={:.3}", r.apply_ms);
    eprintln!("get_resource={}", r.get_resource);
    match r.peak_rss_kb {
        Some(kb) => eprintln!("peak_rss_kb={kb}"),
        None => eprintln!("peak_rss_kb=unavailable"),
    }
    eprintln!("methodology=MemoryStore; warmup=1; peak_rss=VmHWM_proc_self_status");
}

#[test]
#[ignore = "opt-in timing harness; run with --ignored --nocapture"]
fn record_algorithm_correctness_and_timings() {
    for n in sizes() {
        let result = run_algo(n);
        emit_algo(&result);
    }
}

#[tokio::test]
#[ignore = "opt-in timing harness; run with --ignored --nocapture"]
async fn record_e2e_suggest_apply_timings() {
    for n in sizes() {
        let result = run_e2e(n).await;
        emit_e2e(&result);
    }
}
