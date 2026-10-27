use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::NonEmptyStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CollectionId(ulid::Ulid);

impl CollectionId {
    pub fn generate() -> Self {
        Self(ulid::Ulid::generate())
    }

    pub fn parse(s: &str) -> Result<Self, ulid::DecodeError> {
        Ok(Self(ulid::Ulid::from_string(s)?))
    }

    pub fn as_ulid(self) -> ulid::Ulid {
        self.0
    }
}

impl fmt::Display for CollectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Named library collection. Nesting lives on `child_of` edges, not on this row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Collection {
    pub id: CollectionId,
    pub name: NonEmptyStr,
}

impl Collection {
    pub fn try_new(id: CollectionId, name: NonEmptyStr) -> Self {
        Self { id, name }
    }
}

/// One node in the sidebar forest. The same [`CollectionId`] may appear under
/// multiple parents when the nesting graph is a DAG.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CollectionTreeNode {
    pub collection: Collection,
    pub children: Vec<CollectionTreeNode>,
}

/// Build the sidebar forest from flat rows and `(child, parent)` edges.
///
/// Roots are collections with no parents. Children are ordered by name.
/// Multi-parent collections appear under each parent. Cycles are cut.
pub fn build_collection_forest(
    collections: impl IntoIterator<Item = Collection>,
    child_parent_edges: impl IntoIterator<Item = (CollectionId, CollectionId)>,
) -> Vec<CollectionTreeNode> {
    let by_id: HashMap<CollectionId, Collection> =
        collections.into_iter().map(|c| (c.id, c)).collect();
    let mut children_of: HashMap<CollectionId, Vec<CollectionId>> = HashMap::new();
    let mut has_parent: HashSet<CollectionId> = HashSet::new();
    for (child, parent) in child_parent_edges {
        if !by_id.contains_key(&child) || !by_id.contains_key(&parent) {
            continue;
        }
        children_of.entry(parent).or_default().push(child);
        has_parent.insert(child);
    }
    for kids in children_of.values_mut() {
        kids.sort_by(|a, b| {
            let an = by_id.get(a).map(|c| c.name.as_str()).unwrap_or("");
            let bn = by_id.get(b).map(|c| c.name.as_str()).unwrap_or("");
            an.cmp(bn).then_with(|| a.to_string().cmp(&b.to_string()))
        });
        kids.dedup();
    }
    let mut roots: Vec<CollectionId> = by_id
        .keys()
        .copied()
        .filter(|id| !has_parent.contains(id))
        .collect();
    roots.sort_by(|a, b| {
        let an = by_id.get(a).map(|c| c.name.as_str()).unwrap_or("");
        let bn = by_id.get(b).map(|c| c.name.as_str()).unwrap_or("");
        an.cmp(bn).then_with(|| a.to_string().cmp(&b.to_string()))
    });

    fn build_node(
        id: CollectionId,
        by_id: &HashMap<CollectionId, Collection>,
        children_of: &HashMap<CollectionId, Vec<CollectionId>>,
        ancestry: &mut HashSet<CollectionId>,
    ) -> Option<CollectionTreeNode> {
        if !ancestry.insert(id) {
            return None;
        }
        let collection = by_id.get(&id)?.clone();
        let mut children = Vec::new();
        if let Some(child_ids) = children_of.get(&id) {
            for child_id in child_ids {
                if let Some(node) = build_node(*child_id, by_id, children_of, ancestry) {
                    children.push(node);
                }
            }
        }
        ancestry.remove(&id);
        Some(CollectionTreeNode {
            collection,
            children,
        })
    }

    let mut ancestry = HashSet::new();
    roots
        .into_iter()
        .filter_map(|id| build_node(id, &by_id, &children_of, &mut ancestry))
        .collect()
}

/// `root` plus every transitive child reachable via `(child, parent)` edges.
pub fn collection_descendant_closure(
    root: CollectionId,
    child_parent_edges: impl IntoIterator<Item = (CollectionId, CollectionId)>,
) -> Vec<CollectionId> {
    let mut children_of: HashMap<CollectionId, Vec<CollectionId>> = HashMap::new();
    for (child, parent) in child_parent_edges {
        children_of.entry(parent).or_default().push(child);
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        out.push(id);
        if let Some(kids) = children_of.get(&id) {
            stack.extend(kids.iter().copied());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_id_parse_round_trip() {
        let id = CollectionId::generate();
        let parsed = CollectionId::parse(&id.to_string()).unwrap();
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_ulid(), id.as_ulid());
    }

    #[test]
    fn try_new_keeps_name() {
        let name = NonEmptyStr::try_new("Methods").unwrap();
        let collection = Collection::try_new(CollectionId::generate(), name.clone());
        assert_eq!(collection.name, name);
    }
}
