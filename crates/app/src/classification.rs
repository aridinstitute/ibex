//! Read-only collection suggestions and explicit membership application.

use std::collections::HashSet;

use ibex_core::{Collection, CollectionId, CollectionTreeNode, ResourceId, keyword_suggestions};

use crate::command::{GetResource, GetResourceDetail, ListCollectionForest, UpdateResource};
use crate::{AppState, Command, CommandError, Outcome};

pub fn flatten_collections(forest: &[CollectionTreeNode]) -> Vec<Collection> {
    fn visit(
        node: &CollectionTreeNode,
        seen: &mut HashSet<CollectionId>,
        rows: &mut Vec<Collection>,
    ) {
        if seen.insert(node.collection.id) {
            rows.push(node.collection.clone());
        }
        for child in &node.children {
            visit(child, seen, rows);
        }
    }
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    for root in forest {
        visit(root, &mut seen, &mut rows);
    }
    rows
}

/// Suggest keyword-matching collections the resource does not already belong to.
pub async fn suggest_keywords(
    state: &AppState,
    id: ResourceId,
) -> Result<Vec<Collection>, CommandError> {
    let Outcome::GetResourceDetail(detail) = state
        .execute(Command::GetResourceDetail(GetResourceDetail { id }))
        .await?
    else {
        return Err(CommandError::Internal(
            "unexpected outcome from get_resource_detail".into(),
        ));
    };
    let Outcome::ListCollectionForest(forest) = state
        .execute(Command::ListCollectionForest(ListCollectionForest))
        .await?
    else {
        return Err(CommandError::Internal(
            "unexpected outcome from list_collection_forest".into(),
        ));
    };
    let library = flatten_collections(&forest);
    let ids = keyword_suggestions(&detail, &library);
    Ok(library
        .into_iter()
        .filter(|c| {
            ids.contains(&c.id) && !detail.collections.iter().any(|member| member.id == c.id)
        })
        .collect())
}

/// Attach suggestions without replacing other memberships or bibliographic fields.
pub async fn apply_suggestions(
    state: &AppState,
    id: ResourceId,
    suggestions: &[Collection],
) -> Result<(), CommandError> {
    if suggestions.is_empty() {
        return Ok(());
    }
    let Outcome::GetResource(resource) = state
        .execute(Command::GetResource(GetResource { id }))
        .await?
    else {
        return Err(CommandError::Internal(
            "unexpected outcome from get_resource".into(),
        ));
    };
    let mut new = resource.to_new();
    for collection in suggestions {
        if !new.collection_ids.contains(&collection.id) {
            new.collection_ids.push(collection.id);
        }
    }
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

/// Rank unjoined, Jev-enabled collections using a caller-supplied endpoint and credentials.
pub async fn suggest_jev(
    state: &AppState,
    id: ResourceId,
    key: &str,
    model: &str,
    endpoint: &str,
) -> Result<Vec<(Collection, f32)>, String> {
    let Outcome::GetResourceDetail(detail) = state
        .execute(Command::GetResourceDetail(GetResourceDetail { id }))
        .await
        .map_err(|e| e.to_string())?
    else {
        return Err("unexpected outcome from get_resource_detail".into());
    };
    let Outcome::ListCollectionForest(forest) = state
        .execute(Command::ListCollectionForest(ListCollectionForest))
        .await
        .map_err(|e| e.to_string())?
    else {
        return Err("unexpected outcome from list_collection_forest".into());
    };
    let candidates: Vec<_> = flatten_collections(&forest)
        .into_iter()
        .filter(|c| c.jev_enabled && !detail.collections.iter().any(|member| member.id == c.id))
        .collect();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let picks = crate::jev::classify_at(endpoint, &detail, &candidates, key, model).await?;
    Ok(picks
        .into_iter()
        .filter_map(|(id, probability)| {
            candidates
                .iter()
                .find(|c| c.id == id)
                .map(|c| (c.clone(), probability))
        })
        .collect())
}
