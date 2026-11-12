//! BerTopic-style topic clustering for automatic collections.
//!
//! Pipeline follows the BerTopic recipe: embed documents → cluster → describe
//! each cluster with class-based TF-IDF. Embeddings here are sparse TF-IDF
//! vectors (offline, no model download); the shape of the pipeline matches
//! BerTopic so a denser embedder can replace the vectorizer later.
//!
//! Document text prefers extractable file attachments (plus bibliographic
//! fields); otherwise title, subtitle, abstract, and keywords are used.

mod bertopic;
mod text;

use std::collections::{HashMap, HashSet};

use ibex_core::{
    Collection, CollectionId, CollectionTreeNode, NonEmptyStr, ResourceDetail, ResourceId,
};

use crate::classification::flatten_collections;
use crate::command::{
    CreateCollection, GetResource, GetResourceDetail, ListCollectionForest, ListResources,
    SetCollectionClassification, UpdateResource,
};
use crate::{AppState, Command, CommandError, Outcome};

pub use bertopic::{
    ClusterOptions, MAX_FEATURES, MERGE_DISTANCE_CEILING, TopicCluster, TopicMember, TopicReport,
    cluster_documents,
};
pub use text::{
    DocumentText, MAX_ATTACHMENT_BYTES, MAX_DOCUMENT_CHARS, TextSource, extract_attachment_text,
    extract_document_text,
};

/// Default minimum resources per topic (BerTopic defaults higher; libraries are smaller).
pub const DEFAULT_MIN_TOPIC_SIZE: usize = 3;
/// How many c-TF-IDF terms become the collection name.
pub const DEFAULT_LABEL_TERMS: usize = 3;
/// Cap on terms stored as collection keyword rules after apply.
pub const DEFAULT_KEYWORD_TERMS: usize = 5;

/// One clustered document ready for the topic model.
#[derive(Debug, Clone)]
pub struct ClusterDocument {
    pub resource_id: ResourceId,
    pub title: String,
    pub text: DocumentText,
}

/// Options for library-wide clustering.
#[derive(Debug, Clone)]
pub struct SuggestClusters {
    pub min_topic_size: usize,
    pub label_terms: usize,
    pub keyword_terms: usize,
}

impl Default for SuggestClusters {
    fn default() -> Self {
        Self {
            min_topic_size: DEFAULT_MIN_TOPIC_SIZE,
            label_terms: DEFAULT_LABEL_TERMS,
            keyword_terms: DEFAULT_KEYWORD_TERMS,
        }
    }
}

impl From<SuggestClusters> for ClusterOptions {
    fn from(opts: SuggestClusters) -> Self {
        Self {
            min_topic_size: opts.min_topic_size,
            label_terms: opts.label_terms,
            keyword_terms: opts.keyword_terms,
        }
    }
}

/// Suggested topic collections plus unclustered outliers.
#[derive(Debug, Clone)]
pub struct ClusterSuggestion {
    pub report: TopicReport,
    pub documents: usize,
    pub skipped_containers: usize,
}

/// Created or reused collection plus the resources attached in this apply.
#[derive(Debug, Clone)]
pub struct AppliedTopic {
    pub collection: Collection,
    pub resource_ids: Vec<ResourceId>,
    pub keywords: Vec<String>,
    /// True when an existing same-named collection was reused.
    pub reused: bool,
}

/// Gather library documents and run BerTopic-style clustering (read-only).
pub async fn suggest_clusters(
    state: &AppState,
    opts: SuggestClusters,
) -> Result<ClusterSuggestion, CommandError> {
    let Outcome::ListResources(summaries) =
        state.execute(Command::ListResources(ListResources)).await?
    else {
        return Err(CommandError::Internal(
            "unexpected outcome from list_resources".into(),
        ));
    };

    let mut skipped_containers = 0usize;
    let mut details = Vec::new();
    for summary in summaries {
        if summary.kind.is_container() {
            skipped_containers += 1;
            continue;
        }
        details.push(load_detail(state, summary.id).await?);
    }

    // Attachment/PDF extraction is sync and may panic inside pdf-extract; keep
    // it off the async runtime and isolate panics in extract_document_text.
    let documents = tokio::task::spawn_blocking(move || {
        let mut documents = Vec::with_capacity(details.len());
        for detail in details {
            let text = extract_document_text(&detail);
            if text.body.trim().is_empty() {
                continue;
            }
            documents.push(ClusterDocument {
                resource_id: detail.id,
                title: detail.title.as_str().to_owned(),
                text,
            });
        }
        documents
    })
    .await
    .map_err(|e| CommandError::Internal(format!("cluster text extraction: {e}")))?;

    let document_count = documents.len();
    let report = cluster_documents(&documents, opts.into()).map_err(CommandError::Invalid)?;

    Ok(ClusterSuggestion {
        documents: document_count,
        skipped_containers,
        report,
    })
}

/// Create or reuse a collection per topic, seed keyword rules, and attach members.
///
/// Re-applying with the same topic labels under the same parent reuses existing
/// collections (case-insensitive name match among siblings) instead of
/// duplicating them.
pub async fn apply_clusters(
    state: &AppState,
    suggestion: &ClusterSuggestion,
    parent_id: Option<CollectionId>,
) -> Result<Vec<AppliedTopic>, CommandError> {
    let Outcome::ListCollectionForest(forest) = state
        .execute(Command::ListCollectionForest(ListCollectionForest))
        .await?
    else {
        return Err(CommandError::Internal(
            "unexpected outcome from list_collection_forest".into(),
        ));
    };

    let mut used_names: HashSet<String> = flatten_collections(&forest)
        .into_iter()
        .map(|c| c.name.as_str().to_lowercase())
        .collect();
    let mut siblings_by_name = sibling_collections_by_name(&forest, parent_id);

    let mut applied = Vec::with_capacity(suggestion.report.topics.len());

    for topic in &suggestion.report.topics {
        let key = topic.label.trim().to_lowercase();
        let keywords = topic.keywords.clone();
        let (collection, reused) = if let Some(existing) = siblings_by_name.remove(&key) {
            used_names.insert(key);
            let Outcome::SetCollectionClassification(collection) = state
                .execute(Command::SetCollectionClassification(
                    SetCollectionClassification {
                        id: existing.id,
                        keywords: keywords.clone(),
                        jev_enabled: existing.jev_enabled,
                    },
                ))
                .await?
            else {
                return Err(CommandError::Internal(
                    "unexpected outcome from set_collection_classification".into(),
                ));
            };
            (collection, true)
        } else {
            let name = unique_collection_name(&topic.label, &mut used_names)?;
            let Outcome::CreateCollection(collection) = state
                .execute(Command::CreateCollection(CreateCollection {
                    name,
                    parent_id,
                }))
                .await?
            else {
                return Err(CommandError::Internal(
                    "unexpected outcome from create_collection".into(),
                ));
            };
            let Outcome::SetCollectionClassification(collection) = state
                .execute(Command::SetCollectionClassification(
                    SetCollectionClassification {
                        id: collection.id,
                        keywords: keywords.clone(),
                        jev_enabled: false,
                    },
                ))
                .await?
            else {
                return Err(CommandError::Internal(
                    "unexpected outcome from set_collection_classification".into(),
                ));
            };
            (collection, false)
        };

        let mut resource_ids = Vec::with_capacity(topic.members.len());
        for member in &topic.members {
            attach_to_collection(state, member.resource_id, collection.id).await?;
            resource_ids.push(member.resource_id);
        }

        applied.push(AppliedTopic {
            collection,
            resource_ids,
            keywords,
            reused,
        });
    }

    Ok(applied)
}

fn sibling_collections_by_name(
    forest: &[CollectionTreeNode],
    parent_id: Option<CollectionId>,
) -> HashMap<String, Collection> {
    let siblings: Vec<Collection> = match parent_id {
        None => forest.iter().map(|n| n.collection.clone()).collect(),
        Some(parent) => find_children(forest, parent).unwrap_or_default(),
    };
    let mut map = HashMap::new();
    for collection in siblings {
        map.entry(collection.name.as_str().to_lowercase())
            .or_insert(collection);
    }
    map
}

fn find_children(nodes: &[CollectionTreeNode], parent: CollectionId) -> Option<Vec<Collection>> {
    for node in nodes {
        if node.collection.id == parent {
            return Some(node.children.iter().map(|c| c.collection.clone()).collect());
        }
        if let Some(found) = find_children(&node.children, parent) {
            return Some(found);
        }
    }
    None
}

async fn load_detail(state: &AppState, id: ResourceId) -> Result<ResourceDetail, CommandError> {
    let Outcome::GetResourceDetail(detail) = state
        .execute(Command::GetResourceDetail(GetResourceDetail { id }))
        .await?
    else {
        return Err(CommandError::Internal(
            "unexpected outcome from get_resource_detail".into(),
        ));
    };
    Ok(detail)
}

async fn attach_to_collection(
    state: &AppState,
    id: ResourceId,
    collection_id: CollectionId,
) -> Result<(), CommandError> {
    let Outcome::GetResource(resource) = state
        .execute(Command::GetResource(GetResource { id }))
        .await?
    else {
        return Err(CommandError::Internal(
            "unexpected outcome from get_resource".into(),
        ));
    };
    if resource.collection_ids.contains(&collection_id) {
        return Ok(());
    }
    let mut new = resource.to_new();
    new.collection_ids.push(collection_id);
    let Outcome::UpdateResource(_) = state
        .execute(Command::UpdateResource(UpdateResource { id, new }))
        .await?
    else {
        return Err(CommandError::Internal(
            "unexpected outcome from update_resource".into(),
        ));
    };
    Ok(())
}

fn unique_collection_name(
    label: &str,
    used: &mut HashSet<String>,
) -> Result<NonEmptyStr, CommandError> {
    let base = if label.trim().is_empty() {
        "Topic".to_owned()
    } else {
        label.trim().to_owned()
    };
    let mut candidate = base.clone();
    let mut n = 2u32;
    while !used.insert(candidate.to_lowercase()) {
        candidate = format!("{base} ({n})");
        n = n.saturating_add(1);
        if n > 10_000 {
            return Err(CommandError::Internal(
                "could not allocate a unique collection name".into(),
            ));
        }
    }
    NonEmptyStr::try_new(&candidate).map_err(|e| CommandError::Invalid(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_names_suffix_collisions() {
        let mut used = HashSet::new();
        let a = unique_collection_name("climate", &mut used).unwrap();
        let b = unique_collection_name("Climate", &mut used).unwrap();
        assert_eq!(a.as_str(), "climate");
        assert_eq!(b.as_str(), "Climate (2)");
    }
}
