//! BerTopic-style: TF-IDF embed → agglomerative cluster → c-TF-IDF labels.

use std::collections::{HashMap, HashSet};

use ibex_core::ResourceId;

use super::text::TextSource;
use super::{ClusterDocument, DEFAULT_KEYWORD_TERMS, DEFAULT_LABEL_TERMS, DEFAULT_MIN_TOPIC_SIZE};

/// Cap on TF-IDF features (BerTopic-style `max_features`).
pub const MAX_FEATURES: usize = 5_000;
/// Never merge clusters farther apart than this cosine distance.
pub const MERGE_DISTANCE_CEILING: f64 = 0.55;

/// Tunables for the topic model.
#[derive(Debug, Clone)]
pub struct ClusterOptions {
    pub min_topic_size: usize,
    pub label_terms: usize,
    pub keyword_terms: usize,
}

impl Default for ClusterOptions {
    fn default() -> Self {
        Self {
            min_topic_size: DEFAULT_MIN_TOPIC_SIZE,
            label_terms: DEFAULT_LABEL_TERMS,
            keyword_terms: DEFAULT_KEYWORD_TERMS,
        }
    }
}

/// One resource inside a topic (or outlier set).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicMember {
    pub resource_id: ResourceId,
    pub title: String,
    pub source: TextSource,
}

/// A discovered topic with c-TF-IDF keywords and members.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicCluster {
    pub label: String,
    pub keywords: Vec<String>,
    pub members: Vec<TopicMember>,
}

/// Full clustering result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicReport {
    pub topics: Vec<TopicCluster>,
    pub outliers: Vec<TopicMember>,
}

#[derive(Clone, Debug)]
struct SparseVec {
    /// Sorted feature indices.
    indices: Vec<usize>,
    values: Vec<f64>,
}

impl SparseVec {
    fn empty() -> Self {
        Self {
            indices: Vec::new(),
            values: Vec::new(),
        }
    }

    fn from_map(map: &HashMap<usize, f64>) -> Self {
        let mut pairs: Vec<(usize, f64)> = map.iter().map(|(&i, &v)| (i, v)).collect();
        pairs.sort_by_key(|(i, _)| *i);
        let mut indices = Vec::with_capacity(pairs.len());
        let mut values = Vec::with_capacity(pairs.len());
        for (i, v) in pairs {
            indices.push(i);
            values.push(v);
        }
        let mut v = Self { indices, values };
        v.l2_normalize();
        v
    }

    fn l2_normalize(&mut self) {
        let norm = self.values.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm > 0.0 {
            for x in &mut self.values {
                *x /= norm;
            }
        }
    }

    fn cosine_distance(&self, other: &Self) -> f64 {
        let mut i = 0usize;
        let mut j = 0usize;
        let mut dot = 0.0;
        while i < self.indices.len() && j < other.indices.len() {
            match self.indices[i].cmp(&other.indices[j]) {
                std::cmp::Ordering::Equal => {
                    dot += self.values[i] * other.values[j];
                    i += 1;
                    j += 1;
                }
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
            }
        }
        (1.0 - dot).clamp(0.0, 2.0)
    }

    /// Size-weighted mean of two L2-normalized sparse vectors, then re-normalize.
    fn merge_centroid(&self, other: &Self, self_n: f64, other_n: f64) -> Self {
        let mut map: HashMap<usize, f64> = HashMap::new();
        for (&i, &v) in self.indices.iter().zip(self.values.iter()) {
            *map.entry(i).or_default() += v * self_n;
        }
        for (&i, &v) in other.indices.iter().zip(other.values.iter()) {
            *map.entry(i).or_default() += v * other_n;
        }
        let total = self_n + other_n;
        if total > 0.0 {
            for v in map.values_mut() {
                *v /= total;
            }
        }
        Self::from_map(&map)
    }
}

/// Cluster documents into topics.
///
/// Libraries smaller than `min_topic_size` yield every document as an outlier
/// (no error). Empty input yields an empty report.
pub fn cluster_documents(
    documents: &[ClusterDocument],
    opts: ClusterOptions,
) -> Result<TopicReport, String> {
    let min_topic_size = opts.min_topic_size.max(2);
    if documents.is_empty() {
        return Ok(TopicReport {
            topics: Vec::new(),
            outliers: Vec::new(),
        });
    }
    if documents.len() < min_topic_size {
        return Ok(TopicReport {
            topics: Vec::new(),
            outliers: documents.iter().map(member_from).collect(),
        });
    }

    let tokenized: Vec<Vec<String>> = documents.iter().map(|d| tokenize(&d.text.body)).collect();
    if tokenized.iter().all(|t| t.is_empty()) {
        return Err("documents produced no usable tokens for clustering".into());
    }

    let (vocab, vectors) = tfidf_matrix(&tokenized);
    let assignments = agglomerative_cluster(&vectors, min_topic_size);

    let mut by_cluster: HashMap<i32, Vec<usize>> = HashMap::new();
    for (idx, &label) in assignments.iter().enumerate() {
        by_cluster.entry(label).or_default().push(idx);
    }

    let mut topics = Vec::new();
    let mut outliers = Vec::new();

    // Build class bags only for clusters that meet min size (real c-TF-IDF).
    let mut class_members: Vec<Vec<usize>> = Vec::new();
    let mut cluster_ids: Vec<i32> = by_cluster.keys().copied().filter(|id| *id >= 0).collect();
    cluster_ids.sort_unstable();
    for cluster_id in cluster_ids {
        let members_idx = by_cluster.remove(&cluster_id).unwrap_or_default();
        if members_idx.len() < min_topic_size {
            for idx in members_idx {
                outliers.push(member_from(&documents[idx]));
            }
            continue;
        }
        class_members.push(members_idx);
    }
    if let Some(outlier_idxs) = by_cluster.remove(&-1) {
        for idx in outlier_idxs {
            outliers.push(member_from(&documents[idx]));
        }
    }

    let class_tokens: Vec<Vec<&[String]>> = class_members
        .iter()
        .map(|members| members.iter().map(|&i| tokenized[i].as_slice()).collect())
        .collect();
    let keyword_lists = c_tfidf_all_classes(&class_tokens, &vocab, opts.keyword_terms.max(1));

    for (members_idx, keywords) in class_members.into_iter().zip(keyword_lists) {
        let label = label_from_keywords(&keywords, opts.label_terms.max(1));
        let members = members_idx
            .into_iter()
            .map(|idx| member_from(&documents[idx]))
            .collect();
        topics.push(TopicCluster {
            label,
            keywords,
            members,
        });
    }

    topics.sort_by(|a, b| {
        b.members
            .len()
            .cmp(&a.members.len())
            .then_with(|| a.label.cmp(&b.label))
    });

    Ok(TopicReport { topics, outliers })
}

fn member_from(doc: &ClusterDocument) -> TopicMember {
    TopicMember {
        resource_id: doc.resource_id,
        title: doc.title.clone(),
        source: doc.text.source.clone(),
    }
}

fn label_from_keywords(keywords: &[String], n: usize) -> String {
    let terms: Vec<&str> = keywords.iter().take(n).map(String::as_str).collect();
    if terms.is_empty() {
        "Topic".to_owned()
    } else {
        terms.join(" · ")
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .filter(|w| !w.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_lowercase)
        .filter(|w| !is_stopword(w))
        .collect()
}

fn is_stopword(w: &str) -> bool {
    STOPWORDS.binary_search(&w).is_ok()
}

fn tfidf_matrix(docs: &[Vec<String>]) -> (Vec<String>, Vec<SparseVec>) {
    let n = docs.len() as f64;
    let mut df: HashMap<String, usize> = HashMap::new();
    let mut tf_raw: HashMap<String, usize> = HashMap::new();
    for doc in docs {
        let unique: HashSet<&str> = doc.iter().map(String::as_str).collect();
        for term in unique {
            *df.entry(term.to_owned()).or_default() += 1;
        }
        for term in doc {
            *tf_raw.entry(term.clone()).or_default() += 1;
        }
    }

    let mut candidates: Vec<(String, usize, usize)> = df
        .iter()
        .filter(|(_, count)| **count >= 2 && (**count as f64) < n * 0.85)
        .map(|(term, &count)| {
            let raw = *tf_raw.get(term).unwrap_or(&0);
            (term.clone(), count, raw)
        })
        .collect();
    if candidates.is_empty() {
        candidates = df
            .iter()
            .map(|(term, &count)| {
                let raw = *tf_raw.get(term).unwrap_or(&0);
                (term.clone(), count, raw)
            })
            .collect();
    }
    // Prefer rarer-but-present terms; break ties by corpus frequency then name.
    candidates.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.0.cmp(&b.0))
    });
    candidates.truncate(MAX_FEATURES);
    let mut vocab: Vec<String> = candidates.into_iter().map(|(t, _, _)| t).collect();
    vocab.sort();

    let index: HashMap<&str, usize> = vocab
        .iter()
        .enumerate()
        .map(|(i, t)| (t.as_str(), i))
        .collect();

    let idf: Vec<f64> = vocab
        .iter()
        .map(|term| {
            let count = *df.get(term).unwrap_or(&1) as f64;
            (1.0 + n / count).ln()
        })
        .collect();

    let mut vectors = Vec::with_capacity(docs.len());
    for doc in docs {
        let mut tf: HashMap<usize, f64> = HashMap::new();
        for term in doc {
            if let Some(&i) = index.get(term.as_str()) {
                *tf.entry(i).or_default() += 1.0;
            }
        }
        let max_tf = tf.values().copied().fold(0.0_f64, f64::max).max(1.0);
        for (i, count) in tf.iter_mut() {
            *count = (*count / max_tf) * idf[*i];
        }
        vectors.push(SparseVec::from_map(&tf));
    }

    (vocab, vectors)
}

/// Centroid-linkage agglomerative clustering on cosine distance.
///
/// Merges the nearest pair while that pair is within [`MERGE_DISTANCE_CEILING`].
/// Distant pairs are never forced together; undersized leftovers become
/// outliers (`-1`).
fn agglomerative_cluster(vectors: &[SparseVec], min_topic_size: usize) -> Vec<i32> {
    let n = vectors.len();
    if n == 0 {
        return Vec::new();
    }

    let mut clusters: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let mut centroids: Vec<SparseVec> = vectors.to_vec();
    // Flat upper-triangle distance cache: index(i,j) for i < j.
    let mut dist = vec![0.0f64; n * n];
    for i in 0..n {
        for j in (i + 1)..n {
            let d = centroids[i].cosine_distance(&centroids[j]);
            dist[i * n + j] = d;
            dist[j * n + i] = d;
        }
    }
    let mut active: Vec<bool> = vec![true; n];
    let mut next_id = n;
    // Grow storage as merges create new cluster nodes.
    let mut sizes: Vec<usize> = vec![1; n];

    loop {
        let mut best = f64::INFINITY;
        let mut best_pair = None;
        let live: Vec<usize> = (0..next_id)
            .filter(|&i| active.get(i) == Some(&true))
            .collect();
        for a in 0..live.len() {
            for b in (a + 1)..live.len() {
                let i = live[a];
                let j = live[b];
                let d = if i < centroids.len() && j < centroids.len() {
                    // Prefer cache when both indices fit the original square; for
                    // newly allocated nodes recompute against live centroids.
                    if i < n && j < n {
                        dist[i * n + j]
                    } else {
                        centroids[i].cosine_distance(&centroids[j])
                    }
                } else {
                    f64::INFINITY
                };
                if d < best {
                    best = d;
                    best_pair = Some((i, j));
                }
            }
        }
        let Some((i, j)) = best_pair else {
            break;
        };
        if best > MERGE_DISTANCE_CEILING {
            break;
        }

        let merged_members = {
            let mut m = clusters[i].clone();
            m.extend(clusters[j].iter().copied());
            m
        };
        let merged_centroid =
            centroids[i].merge_centroid(&centroids[j], sizes[i] as f64, sizes[j] as f64);
        let merged_size = sizes[i] + sizes[j];

        // Ensure capacity for the new node.
        while clusters.len() <= next_id {
            clusters.push(Vec::new());
            centroids.push(SparseVec::empty());
            sizes.push(0);
            active.push(false);
        }
        clusters[next_id] = merged_members;
        centroids[next_id] = merged_centroid;
        sizes[next_id] = merged_size;
        active[next_id] = true;
        active[i] = false;
        active[j] = false;
        next_id += 1;
    }

    let mut assignments = vec![-1i32; n];
    let mut next_label = 0i32;
    for (id, members) in clusters.iter().enumerate().take(next_id) {
        if active.get(id) != Some(&true) {
            continue;
        }
        if members.len() < min_topic_size {
            continue;
        }
        for &idx in members {
            assignments[idx] = next_label;
        }
        next_label += 1;
    }
    assignments
}

/// Class-based TF-IDF across all topic classes:
/// `W(t,c) = tf(t,c) · log(1 + A / f(t))` where `A` is average words per class
/// and `f(t)` is the number of classes containing `t`.
fn c_tfidf_all_classes(
    classes: &[Vec<&[String]>],
    vocab: &[String],
    top_k: usize,
) -> Vec<Vec<String>> {
    if classes.is_empty() {
        return Vec::new();
    }

    let mut class_tf: Vec<HashMap<&str, f64>> = Vec::with_capacity(classes.len());
    let mut class_words: Vec<f64> = Vec::with_capacity(classes.len());
    for docs in classes {
        let mut tf: HashMap<&str, f64> = HashMap::new();
        let mut words = 0.0;
        for doc in docs {
            for term in *doc {
                *tf.entry(term.as_str()).or_default() += 1.0;
                words += 1.0;
            }
        }
        class_tf.push(tf);
        class_words.push(words);
    }

    let a = class_words.iter().sum::<f64>() / classes.len() as f64;
    let mut class_freq: HashMap<&str, f64> = HashMap::new();
    for tf in &class_tf {
        for term in tf.keys() {
            *class_freq.entry(*term).or_default() += 1.0;
        }
    }

    let mut out = Vec::with_capacity(classes.len());
    for (ci, tf) in class_tf.iter().enumerate() {
        let words = class_words[ci].max(1.0);
        let mut scored: Vec<(String, f64)> = vocab
            .iter()
            .filter_map(|term| {
                let freq = *tf.get(term.as_str())?;
                if freq <= 0.0 {
                    return None;
                }
                let tf_norm = freq / words;
                let f_t = class_freq
                    .get(term.as_str())
                    .copied()
                    .unwrap_or(1.0)
                    .max(1.0);
                let score = tf_norm * (1.0 + a / f_t).ln();
                Some((term.clone(), score))
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        out.push(scored.into_iter().map(|(t, _)| t).collect());
    }
    out
}

/// Sorted for binary search.
const STOPWORDS: &[&str] = &[
    "about", "after", "all", "also", "analysis", "and", "approach", "are", "based", "because",
    "been", "before", "being", "between", "both", "but", "can", "could", "data", "does", "doing",
    "each", "for", "from", "had", "has", "have", "her", "him", "his", "how", "into", "its", "just",
    "may", "method", "methods", "more", "most", "not", "one", "only", "other", "our", "out",
    "over", "paper", "research", "results", "same", "she", "should", "some", "study", "such",
    "than", "that", "the", "their", "them", "then", "there", "these", "they", "this", "those",
    "through", "under", "use", "used", "using", "very", "was", "were", "what", "when", "where",
    "which", "while", "who", "will", "with", "would", "you", "your",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::text::{DocumentText, TextSource};

    fn doc(title: &str, body: &str) -> ClusterDocument {
        ClusterDocument {
            resource_id: ResourceId::generate(),
            title: title.into(),
            text: DocumentText {
                body: body.into(),
                source: TextSource::Metadata,
            },
        }
    }

    fn doc_with_id(id: ResourceId, title: &str, body: &str) -> ClusterDocument {
        ClusterDocument {
            resource_id: id,
            title: title.into(),
            text: DocumentText {
                body: body.into(),
                source: TextSource::Metadata,
            },
        }
    }

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

    fn planted_corpus(per_topic: usize) -> (Vec<ClusterDocument>, HashMap<ResourceId, usize>) {
        let mut docs = Vec::with_capacity(per_topic * PLANTED.len());
        let mut labels = HashMap::new();
        for (topic_i, vocab) in PLANTED.iter().enumerate() {
            for doc_i in 0..per_topic {
                let id = ResourceId::generate();
                let body = (0..6)
                    .map(|k| vocab[(doc_i + k) % vocab.len()])
                    .collect::<Vec<_>>()
                    .join(" ");
                docs.push(doc_with_id(
                    id,
                    &format!("topic{topic_i}-doc{doc_i}"),
                    &body,
                ));
                labels.insert(id, topic_i);
            }
        }
        (docs, labels)
    }

    fn partition_ids(report: &TopicReport) -> (HashSet<ResourceId>, HashSet<ResourceId>) {
        let mut topic_ids = HashSet::new();
        for topic in &report.topics {
            for member in &topic.members {
                assert!(
                    topic_ids.insert(member.resource_id),
                    "resource appears in multiple topics"
                );
            }
        }
        let mut outlier_ids = HashSet::new();
        for member in &report.outliers {
            assert!(
                !topic_ids.contains(&member.resource_id),
                "resource is both topic member and outlier"
            );
            assert!(outlier_ids.insert(member.resource_id), "duplicate outlier");
        }
        (topic_ids, outlier_ids)
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
            let majority = counts.values().copied().max().unwrap_or(0);
            weighted += majority as f64;
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
            let mut best = 0usize;
            for topic in &report.topics {
                let hit = topic
                    .members
                    .iter()
                    .filter(|m| ids.contains(&m.resource_id))
                    .count();
                best = best.max(hit);
            }
            scores.push(best as f64 / ids.len() as f64);
        }
        scores.iter().sum::<f64>() / scores.len() as f64
    }

    #[test]
    fn separates_two_distinct_topics() {
        let docs = vec![
            doc(
                "Neural nets",
                "deep learning neural networks representation learning backpropagation layers",
            ),
            doc(
                "CNN survey",
                "convolutional neural networks deep learning image recognition layers",
            ),
            doc(
                "Transformers",
                "transformer attention neural networks deep learning language models",
            ),
            doc(
                "Carbon sinks",
                "climate change carbon sequestration forests greenhouse emissions",
            ),
            doc(
                "Warming trends",
                "climate change global warming temperature greenhouse emissions",
            ),
            doc(
                "Ocean acid",
                "climate change ocean acidification carbon dioxide greenhouse emissions",
            ),
        ];
        let report = cluster_documents(
            &docs,
            ClusterOptions {
                min_topic_size: 3,
                label_terms: 3,
                keyword_terms: 5,
            },
        )
        .unwrap();
        assert_eq!(report.topics.len(), 2, "topics={:?}", report.topics);
        let mut saw_ml = false;
        let mut saw_climate = false;
        for topic in &report.topics {
            let label = topic.label.to_lowercase();
            if label.contains("neural") || label.contains("learning") || label.contains("network") {
                saw_ml = true;
            }
            if label.contains("climate")
                || label.contains("carbon")
                || label.contains("greenhouse")
                || label.contains("emissions")
            {
                saw_climate = true;
            }
        }
        assert!(saw_ml && saw_climate, "labels={:?}", report.topics);
    }

    #[test]
    fn planted_topics_have_high_purity_and_recovery() {
        let (docs, planted) = planted_corpus(8);
        let report = cluster_documents(
            &docs,
            ClusterOptions {
                min_topic_size: 4,
                label_terms: 3,
                keyword_terms: 6,
            },
        )
        .unwrap();
        let (topic_ids, outlier_ids) = partition_ids(&report);
        assert_eq!(topic_ids.len() + outlier_ids.len(), docs.len());
        assert!(
            report.topics.len() >= 3,
            "expected three planted topics, got {:?}",
            report
                .topics
                .iter()
                .map(|t| (&t.label, t.members.len()))
                .collect::<Vec<_>>()
        );
        for topic in &report.topics {
            assert!(topic.members.len() >= 4);
            assert!(!topic.keywords.is_empty());
        }
        let p = purity(&report, &planted);
        let r = recovery(&report, &planted);
        assert!(p >= 0.85, "purity {p:.3}");
        assert!(r >= 0.70, "recovery {r:.3}");
    }

    #[test]
    fn unrelated_subjects_are_not_forced_together() {
        let docs = vec![
            doc("Opera A", "opera soprano aria libretto bel canto orchestra"),
            doc("Opera B", "opera tenor aria libretto bel canto orchestra"),
            doc(
                "Opera C",
                "opera baritone aria libretto bel canto orchestra",
            ),
            doc(
                "Volcano A",
                "volcano magma eruption lava basalt seismology tephra",
            ),
            doc(
                "Volcano B",
                "volcano magma eruption lava basalt seismology tephra",
            ),
            doc(
                "Volcano C",
                "volcano magma eruption lava basalt seismology tephra",
            ),
            doc(
                "Tax A",
                "taxation fiscal revenue tariff subsidy levy budget",
            ),
            doc(
                "Tax B",
                "taxation fiscal revenue tariff subsidy levy budget",
            ),
            doc(
                "Tax C",
                "taxation fiscal revenue tariff subsidy levy budget",
            ),
            doc("Noise", "xylophone zebra quokka wallaby platypus wombat"),
        ];
        let report = cluster_documents(
            &docs,
            ClusterOptions {
                min_topic_size: 3,
                label_terms: 3,
                keyword_terms: 5,
            },
        )
        .unwrap();
        assert!(
            report.topics.len() >= 3,
            "unrelated subjects must stay separate: {:?}",
            report
                .topics
                .iter()
                .map(|t| (&t.label, t.members.len()))
                .collect::<Vec<_>>()
        );
        for topic in &report.topics {
            assert!(topic.members.len() >= 3);
            assert!(
                topic.members.len() <= 3,
                "topic unexpectedly merged: {} ({})",
                topic.label,
                topic.members.len()
            );
        }
        let noise = docs.last().unwrap().resource_id;
        assert!(
            report.outliers.iter().any(|m| m.resource_id == noise),
            "noise should be outlier"
        );
    }

    #[test]
    fn clustering_is_deterministic_for_fixed_ids() {
        let (docs, _) = planted_corpus(5);
        let opts = ClusterOptions {
            min_topic_size: 3,
            label_terms: 3,
            keyword_terms: 5,
        };
        let a = cluster_documents(&docs, opts.clone()).unwrap();
        let b = cluster_documents(&docs, opts).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn respects_min_topic_size_and_marks_noise_outliers() {
        let mut docs = planted_corpus(3).0;
        docs.push(doc(
            "Noise",
            "xylophone zebra quokka wallaby platypus wombat",
        ));
        let report = cluster_documents(
            &docs,
            ClusterOptions {
                min_topic_size: 3,
                label_terms: 3,
                keyword_terms: 5,
            },
        )
        .unwrap();
        let (topic_ids, outlier_ids) = partition_ids(&report);
        assert_eq!(topic_ids.len() + outlier_ids.len(), docs.len());
        assert!(
            report.topics.len() >= 3,
            "planted topics must not merge: {:?}",
            report
                .topics
                .iter()
                .map(|t| (&t.label, t.members.len()))
                .collect::<Vec<_>>()
        );
        for topic in &report.topics {
            assert!(topic.members.len() >= 3, "undersized topic {}", topic.label);
        }
        let noise = docs.last().unwrap().resource_id;
        assert!(outlier_ids.contains(&noise));
    }

    #[test]
    fn small_library_returns_outliers_not_error() {
        let docs = vec![doc("Only", "lonely document about nothing much here")];
        let report = cluster_documents(&docs, ClusterOptions::default()).unwrap();
        assert!(report.topics.is_empty());
        assert_eq!(report.outliers.len(), 1);
    }

    #[test]
    fn rejects_documents_with_no_tokens() {
        let docs = vec![
            doc("A", "!!! !!!"),
            doc("B", "??? ???"),
            doc("C", "... ..."),
        ];
        let err = cluster_documents(
            &docs,
            ClusterOptions {
                min_topic_size: 2,
                ..ClusterOptions::default()
            },
        )
        .unwrap_err();
        assert!(err.contains("no usable tokens"));
    }

    #[test]
    fn drops_numeric_tokens() {
        let tokens = tokenize("model 2019 university climate carbon emissions");
        assert!(!tokens.iter().any(|t| t == "2019"));
        assert!(tokens.iter().any(|t| t == "climate"));
    }

    #[test]
    fn empty_corpus_ok() {
        let report = cluster_documents(&[], ClusterOptions::default()).unwrap();
        assert!(report.topics.is_empty());
        assert!(report.outliers.is_empty());
    }

    #[test]
    fn stopwords_are_sorted_for_binary_search() {
        let mut sorted = STOPWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(STOPWORDS, sorted.as_slice());
    }
}
