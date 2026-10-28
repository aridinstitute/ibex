//! Shared import orchestration: resolve people/series and persist drafts.

use std::collections::HashMap;

use oeuvre_core::{
    CreatorRole, CreatorSpec, ImportedEntry, ImportedResourceWrite, ImportDraft, ImportIssue,
    ImportReport, Name, NewSeries, NonEmptyStr, Person, PersonId, Series, SeriesId, import_codes,
};

use crate::command::resource::validate_new_resource_refs;
use crate::event::{self, Event};
use crate::{AppState, CommandError};

/// Resolve entities in `draft` and persist through the shared store path.
pub async fn save_draft(
    state: &AppState,
    draft: ImportDraft,
) -> Result<ImportReport, CommandError> {
    save_draft_inner(state, draft, None).await
}

pub(crate) async fn save_draft_inner(
    state: &AppState,
    draft: ImportDraft,
    mut ev: Option<&mut Event>,
) -> Result<ImportReport, CommandError> {
    if let Some(ev) = ev.as_mut() {
        ev.set(event::IMPORT_IMPORTED_COUNT, 0u64);
        ev.set(event::IMPORT_SKIPPED_COUNT, 0u64);
        ev.set(event::IMPORT_WARNING_COUNT, 0u64);
    }

    let mut report = ImportReport::default();
    for diagnostic in draft.diagnostics {
        let issue = ImportIssue {
            entry_key: diagnostic.entry_key.clone(),
            code: Some(diagnostic.code.clone()),
            message: diagnostic.message,
        };
        match diagnostic.severity {
            oeuvre_core::DiagnosticSeverity::Error => report.skipped.push(issue),
            oeuvre_core::DiagnosticSeverity::Warning | oeuvre_core::DiagnosticSeverity::Info => {
                report.warnings.push(issue);
            }
        }
    }

    let mut series = HashMap::<String, SeriesId>::new();
    for entry in draft.entries {
        let entry_key = entry.provenance.source_key.clone();
        let had_reference_fields = entry.had_reference_fields;
        if let Some(ev) = ev.as_mut() {
            ev.set(event::IMPORT_ENTRY_KEY, entry_key.clone());
        }
        let names: Vec<_> = entry
            .unresolved_creators
            .into_iter()
            .map(|c| (c.role, c.name))
            .collect();
        let result = async {
            let (new_people, creators) = plan_creators(state, names).await?;
            let mut new = entry.resource;
            new.creators = creators;
            let mut new_series = None;
            let mut planned_series_title = None;
            if let Some(title) = entry.series_title {
                let id = if let Some(id) = series.get(&title) {
                    *id
                } else {
                    let planned = Series::try_new(
                        SeriesId::generate(),
                        NewSeries {
                            title: NonEmptyStr::try_new(&title)
                                .map_err(|e| CommandError::Invalid(e.to_string()))?,
                        },
                    );
                    let id = planned.id;
                    planned_series_title = Some(title);
                    new_series = Some(planned);
                    id
                };
                new.series_id = Some(id);
                new.series_number = entry.series_number;
            }
            let pending_people: Vec<PersonId> =
                new_people.iter().map(|person| person.id).collect();
            let pending_series = new_series.as_ref().map(|s| s.id);
            validate_new_resource_refs(state, &new, &pending_people, pending_series).await?;
            let resource = state
                .store()
                .create_imported_resource(ImportedResourceWrite {
                    new_people,
                    new_series,
                    resource: new,
                })
                .await?;
            if let (Some(title), Some(series_id)) = (planned_series_title, resource.series_id) {
                series.insert(title, series_id);
            }
            Ok::<_, CommandError>(resource)
        }
        .await;
        match result {
            Ok(resource) => {
                report.imported.push(ImportedEntry {
                    entry_key: entry_key.clone(),
                    resource_id: resource.id,
                });
                if had_reference_fields {
                    report.warnings.push(
                        ImportIssue::new(
                            Some(entry_key),
                            "Reference metadata was resolved/preserved; no parent-resource link was created",
                        )
                        .with_code(import_codes::REFERENCE_NO_PARENT_LINK),
                    );
                }
            }
            Err(CommandError::Invalid(message)) => {
                let code = if message.starts_with("ambiguous creator") {
                    Some(import_codes::AMBIGUOUS_CREATOR.to_owned())
                } else {
                    Some(import_codes::ENTRY_SKIPPED.to_owned())
                };
                report.skipped.push(ImportIssue {
                    entry_key: Some(entry_key),
                    code,
                    message,
                });
            }
            Err(error) => return Err(error),
        }
        if let Some(ev) = ev.as_mut() {
            ev.set(event::IMPORT_IMPORTED_COUNT, report.imported.len());
            ev.set(event::IMPORT_SKIPPED_COUNT, report.skipped.len());
            ev.set(event::IMPORT_WARNING_COUNT, report.warnings.len());
        }
    }
    Ok(report)
}

/// Resolve creator names to existing people or newly planned [`Person`] rows.
/// Does not write: new people are persisted with the resource atomically.
async fn plan_creators(
    state: &AppState,
    names: Vec<(CreatorRole, Name)>,
) -> Result<(Vec<Person>, Vec<CreatorSpec>), CommandError> {
    // Check all ambiguities before allocating any people for this entry.
    let mut resolved = Vec::new();
    for (role, name) in names {
        let people = state.store().find_persons_by_name(&name).await?;
        let id = match people.as_slice() {
            [person] => Some(person.id),
            [] => None,
            _ => {
                return Err(CommandError::Invalid(format!(
                    "ambiguous creator name: {}",
                    name.display()
                )));
            }
        };
        resolved.push((role, name, id));
    }
    let mut creators: Vec<CreatorSpec> = Vec::new();
    let mut new_people: Vec<Person> = Vec::new();
    for (role, name, id) in resolved {
        let person_id = if let Some(id) = id {
            id
        } else if let Some(person) = new_people.iter().find(|person| person.name == name) {
            person.id
        } else {
            let person = Person::try_new(PersonId::generate(), name, vec![])
                .map_err(|e| CommandError::Invalid(e.to_string()))?;
            let id = person.id;
            new_people.push(person);
            id
        };
        if !creators
            .iter()
            .any(|c| c.person_id == person_id && c.role == role)
        {
            creators.push(CreatorSpec {
                person_id,
                role,
                credited_as: None,
            });
        }
    }
    Ok((new_people, creators))
}
