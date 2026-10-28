//! Build an import preview: proposed creates/reuses and unresolved conflicts.
//!
//! Preview never writes. Callers apply [`oeuvre_core::ImportChoices`] at save;
//! save revalidates every decision.

use oeuvre_core::{
    ContainerPreview, CreatorPreview, EntryPreview, EntryProposal, ImportConflict, ImportDraft,
    ImportPreview, PersonCandidate, PersonMatchPreview, SeriesPreview, import_codes,
    source_fingerprint,
};

use crate::{AppState, CommandError};

use super::link::{
    ContainerDecision, ImportEntityIndex, plan_parent_for_entry, resolve_explicit_container,
};
use super::save::{seed_identity_index, seed_index_from_store};

/// Inspect `draft` against the library without persisting.
pub async fn preview_draft(
    state: &AppState,
    draft: ImportDraft,
) -> Result<ImportPreview, CommandError> {
    let fingerprint = draft.source_fingerprint.clone().unwrap_or_else(|| {
        source_fingerprint(draft.provenance.format, "")
    });

    let identity = seed_identity_index(state).await?;
    let mut index = ImportEntityIndex::default();
    seed_index_from_store(state, &mut index).await?;

    let mut series_seen = std::collections::HashMap::<String, bool>::new();
    let mut entries = Vec::with_capacity(draft.entries.len());

    for entry in &draft.entries {
        let entry_key = entry.source_key().to_owned();
        let mut conflicts = Vec::new();

        let proposal = if let Some(id) = identity
            .by_source_entry
            .get(&(fingerprint.clone(), entry_key.clone()))
            .copied()
        {
            EntryProposal::AlreadyCommitted {
                resource_id: id,
                reason: "same source fingerprint and citekey already committed".into(),
            }
        } else if let Some((id, reason)) = identity.find_publication_duplicate(entry) {
            conflicts.push(ImportConflict {
                code: import_codes::PUBLICATION_DUPLICATE.to_owned(),
                message: reason.clone(),
                field: None,
            });
            EntryProposal::PublicationDuplicate {
                resource_id: id,
                reason,
            }
        } else {
            EntryProposal::Create
        };

        let mut people = Vec::new();
        for (index_i, creator) in entry.unresolved_creators.iter().enumerate() {
            let found = state.store().find_persons_by_name(&creator.name).await?;
            let match_ = match found.as_slice() {
                [] => PersonMatchPreview::NoMatch,
                [person] => PersonMatchPreview::Unique {
                    person_id: person.id,
                    reason: "unique name match".into(),
                },
                many => PersonMatchPreview::Ambiguous {
                    candidates: many
                        .iter()
                        .map(|p| PersonCandidate {
                            person_id: p.id,
                            display: p.name.display().to_owned(),
                            name_key: p.name.key().as_str().to_owned(),
                        })
                        .collect(),
                },
            };
            if matches!(match_, PersonMatchPreview::Ambiguous { .. }) {
                conflicts.push(ImportConflict {
                    code: import_codes::AMBIGUOUS_CREATOR.to_owned(),
                    message: format!(
                        "ambiguous creator name: {} — choose an existing person or create new",
                        creator.name.display()
                    ),
                    field: Some(format!("creator[{index_i}]")),
                });
            }
            people.push(CreatorPreview {
                index: index_i,
                role: creator.role,
                name: creator.name.clone(),
                match_,
            });
        }

        let container = preview_container(entry, &mut index, &mut conflicts)?;

        let series = entry.series_title.as_ref().map(|title| {
            let reuse = series_seen.contains_key(title);
            series_seen.insert(title.clone(), true);
            SeriesPreview {
                title: title.clone(),
                reuse,
                reason: if reuse {
                    "series title already planned in this import".into()
                } else {
                    "new series from title".into()
                },
            }
        });

        // Explicit containers that resolve to reuse/conflict still surface.
        if let Some(decision) = resolve_explicit_container(entry, &mut index)? {
            match decision {
                ContainerDecision::Conflict(issue) => {
                    conflicts.push(ImportConflict {
                        code: issue
                            .code
                            .unwrap_or_else(|| import_codes::CONFLICTING_CONTAINER.to_owned()),
                        message: issue.message,
                        field: None,
                    });
                }
                ContainerDecision::Reuse(_) | ContainerDecision::Create(_) => {}
            }
        }

        entries.push(EntryPreview {
            temp_id: entry.temp_id,
            entry_key,
            title: entry.title().as_str().to_owned(),
            kind: entry.kind(),
            proposal,
            people,
            container,
            series,
            conflicts,
        });
    }

    Ok(ImportPreview {
        source_fingerprint: fingerprint,
        entries,
        diagnostics: draft.diagnostics,
    })
}

fn preview_container(
    entry: &oeuvre_core::ImportEntryDraft,
    index: &mut ImportEntityIndex,
    conflicts: &mut Vec<ImportConflict>,
) -> Result<ContainerPreview, CommandError> {
    let plan = plan_parent_for_entry(entry, index)?;
    for issue in &plan.conflicts {
        conflicts.push(ImportConflict {
            code: issue
                .code
                .clone()
                .unwrap_or_else(|| import_codes::AMBIGUOUS_CONTAINER.to_owned()),
            message: issue.message.clone(),
            field: None,
        });
    }
    if let Some(issue) = plan.conflicts.first() {
        return Ok(ContainerPreview::Conflict {
            code: issue
                .code
                .clone()
                .unwrap_or_else(|| import_codes::AMBIGUOUS_CONTAINER.to_owned()),
            message: issue.message.clone(),
        });
    }
    if let Some(dep) = plan.new_dependencies.last() {
        return Ok(ContainerPreview::Create {
            resource_kind: dep.kind,
            title: dep.title.as_str().to_owned(),
            reason: "inferred or explicit container will be created".into(),
        });
    }
    if let Some(parent_id) = plan.parent_id {
        return Ok(ContainerPreview::Reuse {
            resource_id: parent_id,
            reason: "matched existing or in-import container".into(),
        });
    }
    Ok(ContainerPreview::Absent)
}
