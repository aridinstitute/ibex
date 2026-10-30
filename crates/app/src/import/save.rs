//! Shared import orchestration: resolve people/series/containers and persist drafts.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use oeuvre_core::{
    CreatorRole, CreatorSpec, EntryChoice, ImportChoices, ImportDraft, ImportEntryDraft,
    ImportIssue, ImportProgress, ImportReport, ImportedEntry, ImportedResourceWrite, Name, NameKey,
    NewSeries, NonEmptyStr, Person, PersonChoice, PersonId, Resource, ResourceId, ResourceKind,
    Series, SeriesId, import_codes,
};

use crate::command::resource::validate_new_resource_refs;
use crate::event::{self, Event};
use crate::{AppState, CommandError};

use super::assign::AssignmentPlan;
use super::link::{
    ContainerDecision, ImportEntityIndex, plan_parent_for_entry, resolve_explicit_container,
};

/// Optional cancel flag and progress callback for bounded import units.
#[derive(Default, Clone)]
pub struct ImportHooks {
    pub cancel: Option<Arc<AtomicBool>>,
    pub on_progress: Option<Arc<dyn Fn(ImportProgress) + Send + Sync>>,
}

/// Library identities used for retry idempotency and publication duplicates.
#[derive(Debug, Default)]
pub(crate) struct IdentityIndex {
    /// (source_fingerprint, citekey) → committed resource.
    pub by_source_entry: HashMap<(String, String), ResourceId>,
    /// Normalized DOI → resource ids.
    pub by_doi: HashMap<String, Vec<ResourceId>>,
    /// Normalized ISBN → resource ids.
    pub by_isbn: HashMap<String, Vec<ResourceId>>,
}

impl IdentityIndex {
    pub fn remember_resource(&mut self, resource: &Resource) {
        if let (Some(fp), Some(key)) = (
            resource.metadata.provenance.source_fingerprint.as_ref(),
            resource.metadata.citekey.as_ref(),
        ) {
            self.by_source_entry
                .insert((fp.clone(), key.clone()), resource.id);
        }
        if let Some(doi) = resource.doi.as_ref() {
            let key = normalize_doi(doi);
            if !key.is_empty() {
                self.by_doi.entry(key).or_default().push(resource.id);
            }
        }
        for isbn in &resource.isbns {
            self.by_isbn
                .entry(isbn.lookup_key().to_owned())
                .or_default()
                .push(resource.id);
        }
    }

    pub fn find_publication_duplicate(
        &self,
        entry: &ImportEntryDraft,
    ) -> Option<(ResourceId, String)> {
        if let Some(doi) = entry.resource.doi.as_ref() {
            let key = normalize_doi(doi);
            if let Some(ids) = self.by_doi.get(&key) {
                if let Some(id) = ids.first() {
                    return Some((*id, format!("publication DOI already in library ({doi})")));
                }
            }
        }
        // ISBN identity applies to books only. Chapters often inherit a parent
        // ISBN via crossref and must not be treated as book duplicates.
        if entry.kind() == ResourceKind::Book {
            for isbn in &entry.resource.isbns {
                let key = isbn.lookup_key();
                if let Some(ids) = self.by_isbn.get(key) {
                    if let Some(id) = ids.first() {
                        return Some((
                            *id,
                            format!("publication ISBN already in library ({isbn})"),
                        ));
                    }
                }
            }
        }
        None
    }
}

fn normalize_doi(doi: &str) -> String {
    doi.trim()
        .trim_start_matches("https://doi.org/")
        .trim_start_matches("http://doi.org/")
        .trim_start_matches("doi:")
        .trim()
        .to_ascii_lowercase()
}

/// Resolve entities in `draft` and persist through the shared store path.
pub async fn save_draft(
    state: &AppState,
    draft: ImportDraft,
) -> Result<ImportReport, CommandError> {
    save_draft_with_choices(state, draft, ImportChoices::default()).await
}

/// Persist `draft` applying explicit review choices (revalidated at save time).
pub async fn save_draft_with_choices(
    state: &AppState,
    draft: ImportDraft,
    choices: ImportChoices,
) -> Result<ImportReport, CommandError> {
    save_draft_inner(state, draft, choices, None, ImportHooks::default()).await
}

/// Persist `draft` with progress/cancellation hooks between committed units.
pub async fn save_draft_with_hooks(
    state: &AppState,
    draft: ImportDraft,
    choices: ImportChoices,
    hooks: ImportHooks,
) -> Result<ImportReport, CommandError> {
    save_draft_inner(state, draft, choices, None, hooks).await
}

pub(crate) async fn save_draft_inner(
    state: &AppState,
    draft: ImportDraft,
    choices: ImportChoices,
    mut ev: Option<&mut Event>,
    hooks: ImportHooks,
) -> Result<ImportReport, CommandError> {
    if let Some(ev) = ev.as_mut() {
        ev.set(event::IMPORT_IMPORTED_COUNT, 0u64);
        ev.set(event::IMPORT_SKIPPED_COUNT, 0u64);
        ev.set(event::IMPORT_WARNING_COUNT, 0u64);
    }

    let fingerprint = draft.source_fingerprint.clone();
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

    // Validate batch targets before any write; named targets are created
    // lazily, just before the first entry that receives them.
    let mut plan = AssignmentPlan::resolve(state, &choices.assignments).await?;
    if let Some(ev) = ev.as_mut() {
        ev.set(event::IMPORT_ASSIGNED_TAG_COUNT, plan.tag_count());
        ev.set(
            event::IMPORT_ASSIGNED_COLLECTION_COUNT,
            plan.collection_count(),
        );
    }

    let library_resources = state.store().list_resources_for_import().await?;
    let mut identity = IdentityIndex::default();
    for resource in &library_resources {
        identity.remember_resource(resource);
    }
    let mut index = ImportEntityIndex::default();
    seed_index_from_resources(&mut index, &library_resources);

    let mut person_cache = PersonLookupCache::load(state).await?;

    let mut series = HashMap::<String, SeriesId>::new();
    let entry_keys: Vec<String> = draft
        .entries
        .iter()
        .map(|e| e.provenance.source_key.clone())
        .collect();
    let total_entries = draft.entries.len();
    for (entry_pos, entry) in draft.entries.into_iter().enumerate() {
        if hooks
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::SeqCst))
        {
            for key in entry_keys.iter().skip(entry_pos) {
                report.not_attempted.push(
                    ImportIssue::new(
                        Some(key.clone()),
                        "not attempted because import was cancelled",
                    )
                    .with_code(import_codes::CANCELLED),
                );
            }
            report.assignments = plan.report();
            sync_event_counts(ev.as_deref_mut(), &report);
            return Ok(report);
        }

        let entry_key = entry.provenance.source_key.clone();
        let had_reference_fields = entry.had_reference_fields;
        let had_xdata = entry.had_xdata;
        if let Some(ev) = ev.as_mut() {
            ev.set(event::IMPORT_ENTRY_KEY, entry_key.clone());
        }

        let entry_choice = choices
            .entries
            .get(&entry_key)
            .cloned()
            .unwrap_or(EntryChoice::Accept);

        if matches!(entry_choice, EntryChoice::Skip) {
            report.skipped.push(
                ImportIssue::new(Some(entry_key), "skipped by review choice")
                    .with_code(import_codes::ENTRY_SKIPPED),
            );
            continue;
        }

        // Source-entry idempotency: same fingerprint + citekey → reuse.
        if let Some(fp) = fingerprint.as_ref() {
            if let Some(id) = identity
                .by_source_entry
                .get(&(fp.clone(), entry_key.clone()))
                .copied()
            {
                if !matches!(entry_choice, EntryChoice::IntentionalDuplicate) {
                    index.by_citekey.insert(entry_key.clone(), id);
                    if let Err(error) = plan.attach(state, id).await {
                        if record_entry_error(
                            &mut report,
                            entry_key,
                            error,
                            &entry_keys[entry_pos + 1..],
                        )? {
                            report.assignments = plan.report();
                            sync_event_counts(ev.as_deref_mut(), &report);
                            return Ok(report);
                        }
                        continue;
                    }
                    report.reused.push(ImportedEntry {
                        entry_key: entry_key.clone(),
                        resource_id: id,
                    });
                    continue;
                }
            }
        }

        // Publication duplicate from a different source.
        let pub_dup = identity.find_publication_duplicate(&entry);
        match (&entry_choice, pub_dup) {
            (EntryChoice::Reuse { resource_id }, _) => {
                match revalidate_reuse(state, *resource_id).await {
                    Ok(()) => {
                        index.by_citekey.insert(entry_key.clone(), *resource_id);
                        if let Err(error) = plan.attach(state, *resource_id).await {
                            if record_entry_error(
                                &mut report,
                                entry_key,
                                error,
                                &entry_keys[entry_pos + 1..],
                            )? {
                                report.assignments = plan.report();
                                sync_event_counts(ev.as_deref_mut(), &report);
                                return Ok(report);
                            }
                            continue;
                        }
                        report.reused.push(ImportedEntry {
                            entry_key,
                            resource_id: *resource_id,
                        });
                        continue;
                    }
                    Err(mut issue) => {
                        issue.entry_key = Some(entry_key);
                        report.conflicted.push(issue);
                        continue;
                    }
                }
            }
            (EntryChoice::Accept, Some((id, reason))) => {
                report.conflicted.push(
                    ImportIssue::new(Some(entry_key), reason)
                        .with_code(import_codes::PUBLICATION_DUPLICATE),
                );
                let _ = id;
                continue;
            }
            (EntryChoice::IntentionalDuplicate, _) | (EntryChoice::Accept, None) => {}
            (EntryChoice::Skip, _) => unreachable!("handled above"),
        }

        // Explicit journal/book entries may already exist as inferred containers.
        if let Some(decision) = resolve_explicit_container(&entry, &mut index)? {
            match decision {
                ContainerDecision::Reuse(id) => {
                    index.by_citekey.insert(entry_key.clone(), id);
                    if let Err(error) = plan.attach(state, id).await {
                        if record_entry_error(
                            &mut report,
                            entry_key,
                            error,
                            &entry_keys[entry_pos + 1..],
                        )? {
                            report.assignments = plan.report();
                            sync_event_counts(ev.as_deref_mut(), &report);
                            return Ok(report);
                        }
                        continue;
                    }
                    report.reused.push(ImportedEntry {
                        entry_key: entry_key.clone(),
                        resource_id: id,
                    });
                    continue;
                }
                ContainerDecision::Conflict(issue) => {
                    report.conflicted.push(issue);
                    continue;
                }
                ContainerDecision::Create(_) => {}
            }
        }

        let names: Vec<_> = entry
            .unresolved_creators
            .iter()
            .map(|c| (c.role, c.name.clone()))
            .collect();
        let publication_refs = entry.publication_ref_keys.clone();
        let result = async {
            let (new_people, creators) =
                plan_creators(state, &person_cache, &entry_key, &names, &choices).await?;
            let mut new = entry.resource.clone();
            new.creators = creators;
            if let Some(fp) = fingerprint.as_ref() {
                new.metadata.provenance.source_fingerprint = Some(fp.clone());
            }
            if new.metadata.citekey.is_none() {
                new.metadata.citekey = Some(entry_key.clone());
            }
            if !plan.is_empty() {
                plan.materialize(state).await?;
                plan.apply_to_new(&mut new);
            }

            let mut new_series = None;
            let mut planned_series_title = None;
            if let Some(title) = entry.series_title.clone() {
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
                new.series_number = entry.series_number.clone();
            }

            let parent_plan = plan_parent_for_entry(&entry, &mut index)?;
            if let Some(conflict) = parent_plan.conflicts.into_iter().next() {
                return Ok(EntrySave::Conflicted(conflict));
            }
            new.parent_id = parent_plan.parent_id;
            let new_dependencies = parent_plan.new_dependencies;
            let linked = parent_plan.linked;

            let pending_people: Vec<PersonId> = new_people.iter().map(|person| person.id).collect();
            let pending_series = new_series.as_ref().map(|s| s.id);
            let pending_resources: Vec<ResourceId> =
                new_dependencies.iter().map(|r| r.id).collect();
            for dep in &new_dependencies {
                validate_new_resource_refs(state, &dep.to_new(), &[], None, &pending_resources)
                    .await?;
            }
            validate_new_resource_refs(
                state,
                &new,
                &pending_people,
                pending_series,
                &pending_resources,
            )
            .await?;

            let resource = state
                .store()
                .create_imported_resource(ImportedResourceWrite {
                    new_people: new_people.clone(),
                    new_series,
                    new_dependencies,
                    resource: new,
                })
                .await
                .map_err(CommandError::from)?;
            person_cache.remember_new(&new_people);
            if let (Some(title), Some(series_id)) = (planned_series_title, resource.series_id) {
                series.insert(title, series_id);
            }
            Ok(EntrySave::Imported {
                resource,
                linked,
                had_reference_fields,
                had_xdata,
                publication_refs,
            })
        }
        .await;

        match result {
            Ok(EntrySave::Imported {
                resource,
                linked,
                had_reference_fields,
                had_xdata,
                publication_refs,
            }) => {
                identity.remember_resource(&resource);
                index.register_committed(&entry_key, &resource);
                report.imported.push(ImportedEntry {
                    entry_key: entry_key.clone(),
                    resource_id: resource.id,
                });
                if had_reference_fields && !linked {
                    let message = if had_xdata && publication_refs.is_empty() {
                        "xdata metadata was resolved/preserved; xdata is not a parent-resource link"
                    } else {
                        "Reference metadata was resolved/preserved; no parent-resource link was created"
                    };
                    report.warnings.push(
                        ImportIssue::new(Some(entry_key.clone()), message)
                            .with_code(import_codes::REFERENCE_NO_PARENT_LINK),
                    );
                }
                if let Some(cb) = hooks.on_progress.as_ref() {
                    cb(ImportProgress {
                        completed_entries: entry_pos + 1,
                        total_entries,
                        last_entry_key: Some(entry_key),
                    });
                }
            }
            Ok(EntrySave::Conflicted(issue)) => {
                report.conflicted.push(issue);
            }
            Err(error) => {
                if record_entry_error(&mut report, entry_key, error, &entry_keys[entry_pos + 1..])?
                {
                    report.assignments = plan.report();
                    sync_event_counts(ev.as_deref_mut(), &report);
                    return Ok(report);
                }
            }
        }
        sync_event_counts(ev.as_deref_mut(), &report);
    }
    report.assignments = plan.report();
    Ok(report)
}

/// Record a per-entry failure. Returns `true` when the save must stop (a
/// storage failure); remaining entries are then marked not-attempted.
fn record_entry_error(
    report: &mut ImportReport,
    entry_key: String,
    error: CommandError,
    remaining: &[String],
) -> Result<bool, CommandError> {
    match error {
        CommandError::Invalid(message) => {
            let code = if message.starts_with("ambiguous creator") {
                Some(import_codes::AMBIGUOUS_CREATOR.to_owned())
            } else if message.starts_with("stale preview") {
                Some(import_codes::STALE_PREVIEW.to_owned())
            } else {
                Some(import_codes::ENTRY_SKIPPED.to_owned())
            };
            let is_stale = code.as_deref() == Some(import_codes::STALE_PREVIEW);
            let is_ambiguous = code.as_deref() == Some(import_codes::AMBIGUOUS_CREATOR);
            let issue = ImportIssue {
                entry_key: Some(entry_key),
                code,
                message,
            };
            if is_stale || is_ambiguous {
                report.conflicted.push(issue);
            } else {
                report.skipped.push(issue);
            }
            Ok(false)
        }
        CommandError::Store(err) => {
            // Truthful partial report: keep prior commits; mark this failed
            // and remaining entries not-attempted.
            report.failed.push(
                ImportIssue::new(Some(entry_key), err.to_string())
                    .with_code(import_codes::ENTRY_FAILED),
            );
            for key in remaining {
                report.not_attempted.push(
                    ImportIssue::new(
                        Some(key.clone()),
                        "not attempted because an earlier storage operation failed",
                    )
                    .with_code(import_codes::ENTRY_NOT_ATTEMPTED),
                );
            }
            Ok(true)
        }
        error => Err(error),
    }
}

fn sync_event_counts(ev: Option<&mut Event>, report: &ImportReport) {
    if let Some(ev) = ev {
        ev.set(event::IMPORT_IMPORTED_COUNT, report.imported.len());
        ev.set(event::IMPORT_SKIPPED_COUNT, report.skipped.len());
        ev.set(event::IMPORT_WARNING_COUNT, report.warnings.len());
    }
}

enum EntrySave {
    Imported {
        resource: Resource,
        linked: bool,
        had_reference_fields: bool,
        had_xdata: bool,
        publication_refs: Vec<String>,
    },
    Conflicted(ImportIssue),
}

async fn revalidate_reuse(state: &AppState, resource_id: ResourceId) -> Result<(), ImportIssue> {
    match state.store().get_resource(resource_id).await {
        Ok(_) => Ok(()),
        Err(_) => Err(ImportIssue::new(
            None,
            format!("stale preview: resource {resource_id} no longer exists"),
        )
        .with_code(import_codes::STALE_PREVIEW)),
    }
}

pub(crate) fn seed_index_from_resources(index: &mut ImportEntityIndex, resources: &[Resource]) {
    let rows: Vec<_> = resources
        .iter()
        .map(|r| oeuvre_core::ResourceSummary {
            id: r.id,
            kind: r.kind,
            title: r.title.clone(),
            creators_label: String::new(),
            published_label: None,
            publisher: r.publisher.clone(),
            container_title: None,
            date_added: r.date_added,
            date_edited: r.date_edited,
        })
        .collect();
    let details: Vec<_> = resources
        .iter()
        .filter(|r| {
            matches!(
                r.kind,
                ResourceKind::Journal | ResourceKind::Issue | ResourceKind::Book
            )
        })
        .map(|r| (r.id, r.clone()))
        .collect();
    index.seed_from_store(&rows, &details);
}

/// Per-import person resolution cache (avoids N find_persons_by_name calls).
#[derive(Debug, Default)]
pub(crate) struct PersonLookupCache {
    by_key: HashMap<NameKey, Vec<Person>>,
    by_id: HashMap<PersonId, Person>,
}

impl PersonLookupCache {
    pub async fn load(state: &AppState) -> Result<Self, CommandError> {
        let people = state.store().list_persons().await?;
        let mut by_key: HashMap<NameKey, Vec<Person>> = HashMap::new();
        let mut by_id = HashMap::new();
        for person in people {
            by_id.insert(person.id, person.clone());
            for key in person.name_keys() {
                by_key.entry(key).or_default().push(person.clone());
            }
        }
        // Deduplicate people per key (primary + alias may share).
        for list in by_key.values_mut() {
            list.sort_by(|a, b| a.id.to_string().cmp(&b.id.to_string()));
            list.dedup_by(|a, b| a.id == b.id);
        }
        Ok(Self { by_key, by_id })
    }

    pub(crate) fn find_by_name(&self, name: &Name) -> Vec<Person> {
        self.by_key.get(&name.key()).cloned().unwrap_or_default()
    }

    fn get(&self, id: PersonId) -> Option<&Person> {
        self.by_id.get(&id)
    }

    /// Remember people created earlier in this import so later entries reuse them.
    fn remember_new(&mut self, people: &[Person]) {
        for person in people {
            self.by_id.insert(person.id, person.clone());
            for key in person.name_keys() {
                let list = self.by_key.entry(key).or_default();
                if !list.iter().any(|p| p.id == person.id) {
                    list.push(person.clone());
                }
            }
        }
    }
}

/// Resolve creator names to existing people or newly planned [`Person`] rows.
/// Does not write: new people are persisted with the resource atomically.
async fn plan_creators(
    state: &AppState,
    cache: &PersonLookupCache,
    entry_key: &str,
    names: &[(CreatorRole, Name)],
    choices: &ImportChoices,
) -> Result<(Vec<Person>, Vec<CreatorSpec>), CommandError> {
    let mut resolved = Vec::new();
    for (i, (role, name)) in names.iter().enumerate() {
        let people = cache.find_by_name(name);
        let choice = choices.people.get(&ImportChoices::person_key(entry_key, i));
        let id = match (people.as_slice(), choice) {
            (
                _,
                Some(PersonChoice::UseExisting {
                    person_id,
                    expected_name_key,
                }),
            ) => {
                let person = match cache.get(*person_id) {
                    Some(p) => p.clone(),
                    None => state.store().get_person(*person_id).await.map_err(|_| {
                        CommandError::Invalid(format!(
                            "stale preview: person {person_id} no longer exists"
                        ))
                    })?,
                };
                if person.name.key().as_str() != expected_name_key {
                    return Err(CommandError::Invalid(format!(
                        "stale preview: person {person_id} name no longer matches preview"
                    )));
                }
                Some(*person_id)
            }
            (_, Some(PersonChoice::CreateNew)) => None,
            ([person], None) => Some(person.id),
            ([], None) => None,
            ([_, ..], None) => {
                return Err(CommandError::Invalid(format!(
                    "ambiguous creator name: {}",
                    name.display()
                )));
            }
        };
        resolved.push((*role, name.clone(), id));
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
