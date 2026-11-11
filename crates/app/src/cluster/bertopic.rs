//! BerTopic-style: TF-IDF embed → agglomerative cluster → c-TF-IDF labels.

use std::collections::{HashMap, HashSet};

use ibex_core::ResourceId;

use super::text::TextSource;
use super::{ClusterDocument, DEFAULT_KEYWORD_TERMS, DEFAULT_LABEL_TERMS, DEFAULT_MIN_TOPIC_SIZE};

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

/// Cluster documents into topics. Empty input yields an empty report.
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
        return Err(format!(
            "need at least {min_topic_size} resources with text to form a topic; found {}",
            documents.len()
        ));
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
    let mut cluster_ids: Vec<i32> = by_cluster.keys().copied().filter(|id| *id >= 0).collect();
    cluster_ids.sort_unstable();

    for cluster_id in cluster_ids {
        let members_idx = by_cluster.get(&cluster_id).cloned().unwrap_or_default();
        if members_idx.len() < min_topic_size {
            for idx in members_idx {
                outliers.push(member_from(&documents[idx]));
            }
            continue;
        }
        let cluster_tokens: Vec<&[String]> = members_idx
            .iter()
            .map(|&i| tokenized[i].as_slice())
            .collect();
        let member_set: HashSet<usize> = members_idx.iter().copied().collect();
        let other_tokens: Vec<&[String]> = tokenized
            .iter()
            .enumerate()
            .filter(|(i, _)| !member_set.contains(i))
            .map(|(_, t)| t.as_slice())
            .collect();
        let scored = c_tfidf_terms(
            &cluster_tokens,
            &other_tokens,
            &vocab,
            opts.keyword_terms.max(1),
        );
        let keywords: Vec<String> = scored.iter().map(|(t, _)| t.clone()).collect();
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

    if let Some(outlier_idxs) = by_cluster.get(&-1) {
        for &idx in outlier_idxs {
            outliers.push(member_from(&documents[idx]));
        }
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
        .map(str::to_lowercase)
        .filter(|w| !STOPWORDS.iter().any(|s| s == w))
        .collect()
}

fn tfidf_matrix(docs: &[Vec<String>]) -> (Vec<String>, Vec<Vec<f64>>) {
    let n = docs.len() as f64;
    let mut df: HashMap<String, usize> = HashMap::new();
    for doc in docs {
        let unique: HashSet<&str> = doc.iter().map(String::as_str).collect();
        for term in unique {
            *df.entry(term.to_owned()).or_default() += 1;
        }
    }

    let mut vocab: Vec<String> = df
        .iter()
        .filter(|(_, count)| **count >= 2 && (**count as f64) < n * 0.9)
        .map(|(term, _)| term.clone())
        .collect();
    if vocab.is_empty() {
        vocab = df.keys().cloned().collect();
    }
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
        let mut vec = vec![0.0; vocab.len()];
        let max_tf = tf.values().copied().fold(0.0_f64, f64::max).max(1.0);
        for (i, count) in tf {
            vec[i] = (count / max_tf) * idf[i];
        }
        l2_normalize(&mut vec);
        vectors.push(vec);
    }

    (vocab, vectors)
}

fn l2_normalize(v: &mut [f64]) {
    let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

fn cosine_distance(a: &[f64], b: &[f64]) -> f64 {
    let mut dot = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
    }
    (1.0 - dot).clamp(0.0, 2.0)
}

/// Cosine-distance ceiling: once clusters are this far apart and we are at or
/// below the max topic budget, stop merging so well-separated topics survive.
const MERGE_DISTANCE_CEILING: f64 = 0.55;

/// Average-linkage agglomerative clustering on cosine distance.
///
/// Starts with one cluster per document and merges nearest pairs while either
/// more than `⌊n / min_topic_size⌋` clusters remain, or the nearest pair is
/// still closer than [`MERGE_DISTANCE_CEILING`]. Clusters smaller than
/// `min_topic_size` are labeled `-1` (outliers).
fn agglomerative_cluster(vectors: &[Vec<f64>], min_topic_size: usize) -> Vec<i32> {
    let n = vectors.len();
    if n == 0 {
        return Vec::new();
    }

    let max_topics = (n / min_topic_size).clamp(1, n);
    let dim = vectors.first().map(|v| v.len()).unwrap_or(0);

    // Each entry is the member document indices for an active cluster.
    let mut clusters: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let mut centroids: Vec<Vec<f64>> = vectors.to_vec();

    while clusters.len() > 1 {
        let mut best = f64::INFINITY;
        let mut best_pair = None;
        for i in 0..clusters.len() {
            for j in (i + 1)..clusters.len() {
                let dist = cosine_distance(&centroids[i], &centroids[j]);
                if dist < best {
                    best = dist;
                    best_pair = Some((i, j));
                }
            }
        }
        let Some((i, j)) = best_pair else {
            break;
        };
        // Past the budget, only keep merging tight pairs so distinct topics
        // are not forced together.
        if clusters.len() <= max_topics && best > MERGE_DISTANCE_CEILING {
            break;
        }

        let mut merged_members = clusters[i].clone();
        merged_members.extend(clusters[j].iter().copied());
        let sa = clusters[i].len() as f64;
        let sb = clusters[j].len() as f64;
        let mut merged_centroid = vec![0.0; dim];
        for d in 0..dim {
            merged_centroid[d] = (centroids[i][d] * sa + centroids[j][d] * sb) / (sa + sb);
        }
        l2_normalize(&mut merged_centroid);

        // Remove the higher index first so the lower index stays valid.
        let (keep, drop) = if i < j { (i, j) } else { (j, i) };
        clusters[keep] = merged_members;
        centroids[keep] = merged_centroid;
        clusters.remove(drop);
        centroids.remove(drop);
    }

    let mut assignments = vec![-1i32; n];
    let mut next_label = 0i32;
    for members in &clusters {
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

fn c_tfidf_terms(
    cluster_docs: &[&[String]],
    other_docs: &[&[String]],
    vocab: &[String],
    top_k: usize,
) -> Vec<(String, f64)> {
    let mut tf: HashMap<&str, f64> = HashMap::new();
    let mut cluster_words = 0.0;
    for doc in cluster_docs {
        for term in *doc {
            *tf.entry(term.as_str()).or_default() += 1.0;
            cluster_words += 1.0;
        }
    }
    if cluster_words == 0.0 {
        return Vec::new();
    }

    let mut other_df: HashMap<&str, f64> = HashMap::new();
    for doc in other_docs {
        let unique: HashSet<&str> = doc.iter().map(String::as_str).collect();
        for term in unique {
            *other_df.entry(term).or_default() += 1.0;
        }
    }
    let other_n = other_docs.len().max(1) as f64;
    let avg_words = cluster_words;

    let mut scored: Vec<(String, f64)> = vocab
        .iter()
        .filter_map(|term| {
            let freq = *tf.get(term.as_str())?;
            if freq <= 0.0 {
                return None;
            }
            let tf_norm = freq / cluster_words;
            let clusters_with = 1.0 + other_df.get(term.as_str()).copied().unwrap_or(0.0);
            let score = tf_norm * (1.0 + avg_words / clusters_with).ln();
            let other_share = other_df.get(term.as_str()).copied().unwrap_or(0.0) / other_n;
            let specificity = 1.0 - other_share.min(1.0);
            Some((term.clone(), score * (0.5 + 0.5 * specificity)))
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);
    scored
}

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "all", "can", "had", "her", "was", "one",
    "our", "out", "has", "have", "been", "were", "they", "this", "that", "with", "from", "their",
    "what", "which", "when", "who", "will", "each", "she", "how", "its", "may", "him", "his",
    "than", "them", "these", "then", "into", "some", "could", "other", "also", "over", "such",
    "only", "same", "both", "those", "after", "most", "through", "before", "between", "under",
    "while", "where", "about", "would", "there", "being", "because", "should", "does", "doing",
    "just", "more", "very", "your", "using", "used", "use", "based", "paper", "study", "results",
    "method", "methods", "approach", "data", "analysis", "research",
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

    /// Planted topic vocabularies used for purity / recovery checks.
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
                // Rotate terms so docs share topic vocabulary without being identical.
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

    /// Weighted purity: majority planted label share inside each discovered topic.
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

    /// Average, over planted labels, of the largest fraction recovered in one topic.
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
        assert!(
            report.topics.len() >= 2,
            "expected at least two topics, got {:?}",
            report.topics.iter().map(|t| &t.label).collect::<Vec<_>>()
        );
        let labels = report
            .topics
            .iter()
            .map(|t| t.label.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            labels.contains("neural")
                || labels.contains("learning")
                || labels.contains("climate")
                || labels.contains("carbon")
                || labels.contains("greenhouse"),
            "labels should reflect topic words: {labels}"
        );
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
        assert_eq!(
            topic_ids.len() + outlier_ids.len(),
            docs.len(),
            "every document must be assigned exactly once"
        );
        assert!(
            report.topics.len() >= 2,
            "expected multiple topics, got {}",
            report.topics.len()
        );
        for topic in &report.topics {
            assert!(
                topic.members.len() >= 4,
                "min_topic_size violated: {}",
                topic.members.len()
            );
            assert!(
                !topic.keywords.is_empty(),
                "topic {} missing keywords",
                topic.label
            );
        }
        let p = purity(&report, &planted);
        let r = recovery(&report, &planted);
        assert!(
            p >= 0.85,
            "purity {p:.3} below threshold; topics={:?}",
            report
                .topics
                .iter()
                .map(|t| (&t.label, t.members.len()))
                .collect::<Vec<_>>()
        );
        assert!(
            r >= 0.70,
            "recovery {r:.3} below threshold; topics={:?}",
            report
                .topics
                .iter()
                .map(|t| (&t.label, t.members.len()))
                .collect::<Vec<_>>()
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
        // Unique singleton noise should not form its own topic at min_size=3.
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
        for topic in &report.topics {
            assert!(topic.members.len() >= 3, "undersized topic {}", topic.label);
        }
        let noise = docs.last().unwrap().resource_id;
        assert!(
            outlier_ids.contains(&noise),
            "singleton noise should be an outlier, topics={:?}",
            report
                .topics
                .iter()
                .map(|t| (&t.label, t.members.len()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn rejects_too_few_documents() {
        let docs = vec![doc("Only", "lonely document about nothing much here")];
        let err = cluster_documents(&docs, ClusterOptions::default()).unwrap_err();
        assert!(err.contains("need at least"));
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
    fn empty_corpus_ok() {
        let report = cluster_documents(&[], ClusterOptions::default()).unwrap();
        assert!(report.topics.is_empty());
        assert!(report.outliers.is_empty());
    }
}
