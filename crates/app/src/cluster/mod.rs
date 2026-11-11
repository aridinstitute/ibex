//! BerTopic-style topic clustering for automatic collections.
//!
//! Pipeline follows the BerTopic recipe: embed documents → cluster → describe
//! each cluster with class-based TF-IDF. Embeddings here are TF-IDF vectors
//! (offline, no model download); the shape of the pipeline matches BerTopic so
//! a denser embedder can replace the vectorizer later.
//!
//! Document text prefers extractable file attachments; otherwise title,
//! subtitle, and related bibliographic fields are used.

mod bertopic;
mod text;

use std::collections::HashSet;

use ibex_core::{
    Collection, CollectionId, NonEmptyStr, ResourceDetail, ResourceId, ResourceSummary,
};

use crate::command::{
    CreateCollection, GetResource, GetResourceDetail, ListResources, SetCollectionClassification,
    UpdateResource,
};
use crate::{AppState, Command, CommandError, Outcome};

pub use bertopic::{ClusterOptions, TopicCluster, TopicMember, TopicReport, cluster_documents};
pub use text::{DocumentText, TextSource, extract_attachment_text, extract_document_text};

/// Default minimum resources per topic (BerTopic defaults higher; libraries are smaller).
pub const DEFAULT_MIN_TOPIC_SIZE: usize = 3;
/// How many c-TF-IDF terms become the collection name.
pub const DEFAULT_LABEL_TERMS: usize = 3;
/// Cap on terms stored as collection keyword rules after apply.
pub const DEFAULT_KEYWORD_TERMS: usize = 8;

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

/// Suggested topic collections plus unclustered outliers.
#[derive(Debug, Clone)]
pub struct ClusterSuggestion {
    pub report: TopicReport,
    pub documents: usize,
    pub skipped_containers: usize,
}

/// Created collection plus the resources that were (or would be) attached.
#[derive(Debug, Clone)]
pub struct AppliedTopic {
    pub collection: Collection,
    pub resource_ids: Vec<ResourceId>,
    pub keywords: Vec<String>,
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
    let mut documents = Vec::new();
    for summary in summaries {
        if summary.kind.is_container() {
            skipped_containers += 1;
            continue;
        }
        let detail = load_detail(state, summary.id).await?;
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

    let report = cluster_documents(
        &documents,
        ClusterOptions {
            min_topic_size: opts.min_topic_size,
            label_terms: opts.label_terms,
            keyword_terms: opts.keyword_terms,
        },
    )
    .map_err(CommandError::Invalid)?;

    Ok(ClusterSuggestion {
        documents: documents.len(),
        skipped_containers,
        report,
    })
}

/// Create a collection per topic, seed keyword rules, and attach members.
pub async fn apply_clusters(
    state: &AppState,
    suggestion: &ClusterSuggestion,
    parent_id: Option<CollectionId>,
) -> Result<Vec<AppliedTopic>, CommandError> {
    let mut applied = Vec::with_capacity(suggestion.report.topics.len());
    let mut used_names = HashSet::new();

    for topic in &suggestion.report.topics {
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

        let keywords = topic.keywords.clone();
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

        let mut resource_ids = Vec::with_capacity(topic.members.len());
        for member in &topic.members {
            attach_to_collection(state, member.resource_id, collection.id).await?;
            resource_ids.push(member.resource_id);
        }

        applied.push(AppliedTopic {
            collection,
            resource_ids,
            keywords,
        });
    }

    Ok(applied)
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

/// Summaries that would be clustered (non-containers).
pub fn clusterable_summaries(summaries: &[ResourceSummary]) -> Vec<&ResourceSummary> {
    summaries
        .iter()
        .filter(|s| !s.kind.is_container())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ibex_core::{ResourceKind, UtcMillis};

    #[test]
    fn unique_names_suffix_collisions() {
        let mut used = HashSet::new();
        let a = unique_collection_name("climate", &mut used).unwrap();
        let b = unique_collection_name("Climate", &mut used).unwrap();
        assert_eq!(a.as_str(), "climate");
        assert_eq!(b.as_str(), "Climate (2)");
    }

    #[test]
    fn clusterable_skips_containers() {
        let stamp = UtcMillis::from_unix_ms(0);
        let rows = [
            ResourceSummary {
                id: ResourceId::generate(),
                kind: ResourceKind::JournalArticle,
                title: NonEmptyStr::try_new("Paper").unwrap(),
                creators_label: String::new(),
                published_label: None,
                publisher: None,
                container_title: None,
                date_added: stamp,
                date_edited: stamp,
            },
            ResourceSummary {
                id: ResourceId::generate(),
                kind: ResourceKind::Journal,
                title: NonEmptyStr::try_new("Nature").unwrap(),
                creators_label: String::new(),
                published_label: None,
                publisher: None,
                container_title: None,
                date_added: stamp,
                date_edited: stamp,
            },
        ];
        assert_eq!(clusterable_summaries(&rows).len(), 1);
    }
}
