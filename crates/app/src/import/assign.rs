//! Batch collection/tag targets applied to every imported or reused entry.
//!
//! Targets are validated without writes before the first entry, then named
//! targets that do not exist yet are created once, just before the first entry
//! that receives them. Exact-name lookup on retry finds those same targets, so
//! retries never create duplicates.

use ibex_core::{
    AssignedTarget, CollectionId, CollectionTreeNode, ExistingIdOrName, ImportAssignmentReport,
    ImportAssignments, NewResource, NonEmptyStr, ResourceId, TagId,
};

use crate::{AppState, CommandError};

#[derive(Debug, Clone)]
enum Planned<Id> {
    Existing { id: Id, name: String, created: bool },
    Create { name: NonEmptyStr },
}

impl<Id: Copy + PartialEq> Planned<Id> {
    fn id(&self) -> Option<Id> {
        match self {
            Planned::Existing { id, .. } => Some(*id),
            Planned::Create { .. } => None,
        }
    }
}

fn push_existing<Id: Copy + PartialEq>(out: &mut Vec<Planned<Id>>, id: Id, name: String) {
    if !out.iter().any(|p| p.id() == Some(id)) {
        out.push(Planned::Existing {
            id,
            name,
            created: false,
        });
    }
}

fn push_create<Id>(out: &mut Vec<Planned<Id>>, name: NonEmptyStr) {
    if !out
        .iter()
        .any(|p| matches!(p, Planned::Create { name: n } if *n == name))
    {
        out.push(Planned::Create { name });
    }
}

/// Re-validates a caller-supplied name with the shared trimming rule, since
/// deserialized [`NonEmptyStr`] values bypass [`NonEmptyStr::try_new`].
fn checked_name(name: &NonEmptyStr, what: &str) -> Result<NonEmptyStr, CommandError> {
    NonEmptyStr::try_new(name.as_str())
        .map_err(|_| CommandError::Invalid(format!("{what} name must not be empty")))
}

fn flatten_forest(nodes: &[CollectionTreeNode], out: &mut Vec<(CollectionId, String)>) {
    for node in nodes {
        let id = node.collection.id;
        if !out.iter().any(|(seen, _)| *seen == id) {
            out.push((id, node.collection.name.as_str().to_owned()));
        }
        flatten_forest(&node.children, out);
    }
}

fn not_found(what: &str, id: impl std::fmt::Display, err: ibex_core::StoreError) -> CommandError {
    if err.is_not_found() {
        CommandError::NotFound(format!("{what} {id}"))
    } else if err.is_invalid() {
        CommandError::Invalid(err.message.clone())
    } else {
        CommandError::Store(err)
    }
}

/// Resolved batch targets for one save.
#[derive(Debug, Default)]
pub(crate) struct AssignmentPlan {
    tags: Vec<Planned<TagId>>,
    collections: Vec<Planned<CollectionId>>,
    materialized: bool,
}

impl AssignmentPlan {
    /// Validate every requested target without writing. Unknown ids,
    /// empty names, and collection names shared by several collections are
    /// rejected here so a bad request leaves the library untouched.
    pub(crate) async fn resolve(
        state: &AppState,
        assignments: &ImportAssignments,
    ) -> Result<Self, CommandError> {
        let mut plan = Self::default();
        if assignments.is_empty() {
            plan.materialized = true;
            return Ok(plan);
        }

        if !assignments.tags.is_empty() {
            let library_tags = state.store().list_tags().await?;
            for target in &assignments.tags {
                match target {
                    ExistingIdOrName::Existing { id } => {
                        let tag = state
                            .store()
                            .get_tag(*id)
                            .await
                            .map_err(|e| not_found("tag", id, e))?;
                        push_existing(&mut plan.tags, tag.id, tag.text.as_str().to_owned());
                    }
                    ExistingIdOrName::Name { name } => {
                        let name = checked_name(name, "tag")?;
                        match library_tags.iter().find(|t| t.text == name) {
                            Some(tag) => {
                                push_existing(&mut plan.tags, tag.id, tag.text.as_str().to_owned())
                            }
                            None => push_create(&mut plan.tags, name),
                        }
                    }
                }
            }
        }

        if !assignments.collections.is_empty() {
            let forest = state.store().list_collection_forest().await?;
            let mut library_collections = Vec::new();
            flatten_forest(&forest, &mut library_collections);
            for target in &assignments.collections {
                match target {
                    ExistingIdOrName::Existing { id } => {
                        let collection = state
                            .store()
                            .get_collection(*id)
                            .await
                            .map_err(|e| not_found("collection", id, e))?;
                        push_existing(
                            &mut plan.collections,
                            collection.id,
                            collection.name.as_str().to_owned(),
                        );
                    }
                    ExistingIdOrName::Name { name } => {
                        let name = checked_name(name, "collection")?;
                        let matches: Vec<&(CollectionId, String)> = library_collections
                            .iter()
                            .filter(|(_, n)| n == name.as_str())
                            .collect();
                        match matches.as_slice() {
                            [] => push_create(&mut plan.collections, name),
                            [(id, n)] => push_existing(&mut plan.collections, *id, n.clone()),
                            many => {
                                let ids: Vec<String> =
                                    many.iter().map(|(id, _)| id.to_string()).collect();
                                return Err(CommandError::Invalid(format!(
                                    "ambiguous collection name \"{}\" matches {} collections ({}); select one by id",
                                    name.as_str(),
                                    many.len(),
                                    ids.join(", ")
                                )));
                            }
                        }
                    }
                }
            }
        }
        Ok(plan)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tags.is_empty() && self.collections.is_empty()
    }

    /// Create named targets that do not exist yet. Idempotent.
    pub(crate) async fn materialize(&mut self, state: &AppState) -> Result<(), CommandError> {
        if self.materialized {
            return Ok(());
        }
        for planned in &mut self.tags {
            if let Planned::Create { name } = planned {
                let tag = match state.store().create_tag(name.clone()).await {
                    Ok(tag) => (tag, true),
                    // Tag text is unique: another writer may have created it.
                    Err(err) if err.is_invalid() => {
                        match state.store().find_tag_by_text(name).await? {
                            Some(tag) => (tag, false),
                            None => return Err(CommandError::Invalid(err.message)),
                        }
                    }
                    Err(err) => return Err(CommandError::Store(err)),
                };
                *planned = Planned::Existing {
                    id: tag.0.id,
                    name: tag.0.text.as_str().to_owned(),
                    created: tag.1,
                };
            }
        }
        for planned in &mut self.collections {
            if let Planned::Create { name } = planned {
                let collection = state
                    .store()
                    .create_collection(name.clone(), None)
                    .await
                    .map_err(CommandError::Store)?;
                *planned = Planned::Existing {
                    id: collection.id,
                    name: collection.name.as_str().to_owned(),
                    created: true,
                };
            }
        }
        dedup_by_id(&mut self.tags);
        dedup_by_id(&mut self.collections);
        self.materialized = true;
        Ok(())
    }

    fn tag_ids(&self) -> Vec<TagId> {
        self.tags.iter().filter_map(Planned::id).collect()
    }

    fn collection_ids(&self) -> Vec<CollectionId> {
        self.collections.iter().filter_map(Planned::id).collect()
    }

    /// Add targets to a resource about to be created. Call after
    /// [`Self::materialize`].
    pub(crate) fn apply_to_new(&self, new: &mut NewResource) {
        for id in self.tag_ids() {
            if !new.tag_ids.contains(&id) {
                new.tag_ids.push(id);
            }
        }
        for id in self.collection_ids() {
            if !new.collection_ids.contains(&id) {
                new.collection_ids.push(id);
            }
        }
    }

    /// Add targets to an existing resource, changing nothing else.
    pub(crate) async fn attach(
        &mut self,
        state: &AppState,
        resource_id: ResourceId,
    ) -> Result<(), CommandError> {
        if self.is_empty() {
            return Ok(());
        }
        self.materialize(state).await?;
        state
            .store()
            .add_resource_labels(resource_id, &self.tag_ids(), &self.collection_ids())
            .await
            .map_err(|err| {
                if err.is_not_found() {
                    CommandError::Invalid(format!(
                        "stale preview: resource {resource_id} no longer exists"
                    ))
                } else {
                    CommandError::Store(err)
                }
            })
    }

    /// Targets this save applied; empty until the first entry received them.
    pub(crate) fn report(&self) -> ImportAssignmentReport {
        if !self.materialized {
            return ImportAssignmentReport::default();
        }
        ImportAssignmentReport {
            tags: self.tags.iter().filter_map(assigned).collect(),
            collections: self.collections.iter().filter_map(assigned).collect(),
        }
    }

    pub(crate) fn tag_count(&self) -> usize {
        self.tags.len()
    }

    pub(crate) fn collection_count(&self) -> usize {
        self.collections.len()
    }
}

fn assigned<Id: Copy>(planned: &Planned<Id>) -> Option<AssignedTarget<Id>> {
    match planned {
        Planned::Existing { id, name, created } => Some(AssignedTarget {
            id: *id,
            name: name.clone(),
            created: *created,
        }),
        Planned::Create { .. } => None,
    }
}

fn dedup_by_id<Id: Copy + PartialEq>(items: &mut Vec<Planned<Id>>) {
    let mut seen: Vec<Id> = Vec::new();
    items.retain(|p| match p.id() {
        Some(id) if seen.contains(&id) => false,
        Some(id) => {
            seen.push(id);
            true
        }
        None => true,
    });
}
