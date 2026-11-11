//! BerTopic-style clustering benchmarks.
//!
//! ```text
//! cargo bench -p ibex-app --features testing --bench topic_cluster
//! IBEX_CLUSTER_BENCH_SIZES=24,96 cargo bench -p ibex-app --features testing --bench topic_cluster
//! ```

use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ibex_app::cluster::{
    ClusterDocument, ClusterOptions, DocumentText, TextSource, cluster_documents,
};
use ibex_core::ResourceId;

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

fn planted_corpus(total: usize) -> Vec<ClusterDocument> {
    let per_topic = (total / PLANTED.len()).max(4);
    let mut docs = Vec::with_capacity(per_topic * PLANTED.len());
    for (topic_i, vocab) in PLANTED.iter().enumerate() {
        for doc_i in 0..per_topic {
            let body = (0..8)
                .map(|k| vocab[(doc_i + k) % vocab.len()])
                .collect::<Vec<_>>()
                .join(" ");
            docs.push(ClusterDocument {
                resource_id: ResourceId::generate(),
                title: format!("topic{topic_i}-doc{doc_i}"),
                text: DocumentText {
                    body,
                    source: TextSource::Metadata,
                },
            });
        }
    }
    docs
}

fn bench_cluster(c: &mut Criterion) {
    let mut group = c.benchmark_group("topic_cluster");
    group.warm_up_time(Duration::from_millis(300));
    group.measurement_time(Duration::from_secs(2));
    group.sample_size(20);

    for &n in &sizes() {
        let docs = planted_corpus(n);
        let actual = docs.len();
        group.throughput(Throughput::Elements(actual as u64));
        eprintln!("topic_cluster/planted/{actual}");
        group.bench_with_input(BenchmarkId::new("planted", actual), &docs, |b, docs| {
            b.iter(|| {
                let report = cluster_documents(
                    black_box(docs),
                    ClusterOptions {
                        min_topic_size: 4,
                        label_terms: 3,
                        keyword_terms: 6,
                    },
                )
                .expect("cluster");
                black_box(report.topics.len());
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_cluster);
criterion_main!(benches);
