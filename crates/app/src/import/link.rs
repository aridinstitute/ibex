//! Plan publication parents (journal / issue / book) for import entries.
//!
//! Matching rules (Task 04):
//! - **Journal:** unique ISSN → reuse; unique case-insensitive title → reuse;
//!   multiple title hits → ambiguous; ISSN hit with conflicting title → conflict.
//! - **Issue:** under a resolved journal, match `(volume, issue)` labels;
//!   create when missing. No issue without volume or number evidence.
//! - **Book:** unique ISBN → reuse; unique title → reuse; ambiguous → conflict.
//! - **Series:** title key (handled in save orchestration).
//! - **xdata** is never a publication parent.
//!
//! Inferred containers are real resources (appear in listings) but are not
//! counted as source-entry imports in [`ibex_core::ImportReport::imported`].

use ibex_core::{
    Identifier, ImportEntryDraft, ImportIssue, NewResource, NonEmptyStr, Resource, ResourceId,
    ResourceKind, ResourceSummary, import_codes,
};

use crate::CommandError;

#[derive(Debug, Clone)]
pub(crate) enum ContainerDecision {
    /// Use this already-known resource id (store or earlier in this import).
    Reuse(ResourceId),
    /// Create with the given planned resource (id pre-allocated).
    Create(Resource),
    /// Ambiguous or conflicting match; do not attach a parent.
    Conflict(ImportIssue),
}

#[derive(Debug, Default)]
pub(crate) struct PlannedParent {
    /// New containers to persist with the child (journal then issue, or book).
    pub new_dependencies: Vec<Resource>,
    pub parent_id: Option<ResourceId>,
    pub conflicts: Vec<ImportIssue>,
    /// True when a publication parent link was established.
    pub linked: bool,
}

/// Index of containers known within the current import (source keys + planned).
#[derive(Debug, Default)]
pub(crate) struct ImportEntityIndex {
    /// Source citekey → committed or planned resource id (explicit entries).
    pub by_citekey: std::collections::HashMap<String, ResourceId>,
    /// Planned/known journals by lowercase title.
    pub journals_by_title: std::collections::HashMap<String, Vec<ResourceId>>,
    /// Planned/known journals by ISSN lookup key.
    pub journals_by_issn: std::collections::HashMap<String, ResourceId>,
    /// Journal id → titles we associate (for conflict checks).
    pub journal_titles: std::collections::HashMap<ResourceId, String>,
    /// (journal_id, volume, issue) → issue resource id.
    pub issues: std::collections::HashMap<(ResourceId, String, String), ResourceId>,
    /// Books by lowercase title / ISBN.
    pub books_by_title: std::collections::HashMap<String, Vec<ResourceId>>,
    pub books_by_isbn: std::collections::HashMap<String, ResourceId>,
    pub book_titles: std::collections::HashMap<ResourceId, String>,
}

impl ImportEntityIndex {
    pub fn seed_from_store(
        &mut self,
        rows: &[ResourceSummary],
        details: &[(ResourceId, Resource)],
    ) {
        for row in rows {
            match row.kind {
                ResourceKind::Journal => {
                    self.remember_journal(row.id, row.title.as_str(), &[]);
                }
                ResourceKind::Issue => {
                    // volume/issue filled from details below
                }
                ResourceKind::Book => {
                    self.remember_book(row.id, row.title.as_str(), &[]);
                }
                _ => {}
            }
        }
        for (id, resource) in details {
            match resource.kind {
                ResourceKind::Journal => {
                    self.remember_journal(*id, resource.title.as_str(), &resource.issns);
                }
                ResourceKind::Issue => {
                    if let Some(journal_id) = resource.parent_id {
                        let vol = resource.volume.clone().unwrap_or_default();
                        let num = resource.issue.clone().unwrap_or_default();
                        self.issues.insert((journal_id, vol, num), *id);
                    }
                }
                ResourceKind::Book => {
                    self.remember_book(*id, resource.title.as_str(), &resource.isbns);
                }
                _ => {}
            }
        }
    }

    fn remember_journal(&mut self, id: ResourceId, title: &str, issns: &[Identifier]) {
        let fresh = self.journal_titles.insert(id, title.to_owned()).is_none();
        if fresh {
            self.journals_by_title
                .entry(normalize_title(title))
                .or_default()
                .push(id);
        }
        for issn in issns {
            self.journals_by_issn
                .entry(issn.lookup_key().to_owned())
                .or_insert(id);
        }
    }

    fn remember_book(&mut self, id: ResourceId, title: &str, isbns: &[Identifier]) {
        let fresh = self.book_titles.insert(id, title.to_owned()).is_none();
        if fresh {
            self.books_by_title
                .entry(normalize_title(title))
                .or_default()
                .push(id);
        }
        for isbn in isbns {
            self.books_by_isbn
                .entry(isbn.lookup_key().to_owned())
                .or_insert(id);
        }
    }

    pub fn register_committed(&mut self, citekey: &str, resource: &Resource) {
        self.by_citekey.insert(citekey.to_owned(), resource.id);
        match resource.kind {
            ResourceKind::Journal => {
                self.remember_journal(resource.id, resource.title.as_str(), &resource.issns);
            }
            ResourceKind::Issue => {
                if let Some(journal_id) = resource.parent_id {
                    let vol = resource.volume.clone().unwrap_or_default();
                    let num = resource.issue.clone().unwrap_or_default();
                    self.issues.insert((journal_id, vol, num), resource.id);
                }
            }
            ResourceKind::Book => {
                self.remember_book(resource.id, resource.title.as_str(), &resource.isbns);
            }
            _ => {}
        }
    }

    pub fn register_planned_journal(&mut self, resource: &Resource) {
        self.remember_journal(resource.id, resource.title.as_str(), &resource.issns);
    }

    pub fn register_planned_issue(&mut self, resource: &Resource) {
        if let Some(journal_id) = resource.parent_id {
            let vol = resource.volume.clone().unwrap_or_default();
            let num = resource.issue.clone().unwrap_or_default();
            self.issues.insert((journal_id, vol, num), resource.id);
        }
    }

    pub fn register_planned_book(&mut self, resource: &Resource) {
        self.remember_book(resource.id, resource.title.as_str(), &resource.isbns);
    }
}

pub(crate) fn normalize_title(title: &str) -> String {
    title.trim().to_lowercase()
}

pub(crate) fn issue_title(volume: Option<&str>, number: Option<&str>) -> String {
    match (
        volume.map(str::trim).filter(|s| !s.is_empty()),
        number.map(str::trim).filter(|s| !s.is_empty()),
    ) {
        (Some(v), Some(n)) => format!("Vol. {v}, no. {n}"),
        (Some(v), None) => format!("Vol. {v}"),
        (None, Some(n)) => format!("No. {n}"),
        (None, None) => "Issue".into(),
    }
}

pub(crate) fn blank_container(
    kind: ResourceKind,
    title: NonEmptyStr,
    parent_id: Option<ResourceId>,
) -> NewResource {
    NewResource {
        kind,
        title,
        creators: vec![],
        publisher: None,
        published: None,
        accessed: None,
        parent_id,
        series_id: None,
        series_number: None,
        tag_ids: vec![],
        collection_ids: vec![],
        metadata: Default::default(),
        subtitle: None,
        title_addon: None,
        short_title: None,
        place: None,
        volume: None,
        issue: None,
        pages: None,
        total_pages: None,
        article_number: None,
        edition: None,
        doi: None,
        url: None,
        eprint: None,
        isbns: vec![],
        issns: vec![],
        isrns: vec![],
        original_date: None,
        event_date: None,
        language: None,
        document_type: None,
        publication_method: None,
        institution: None,
        organization: None,
        school: None,
        abstract_text: None,
        keywords: vec![],
    }
}

fn plan_resource(new: NewResource) -> Result<Resource, CommandError> {
    Resource::try_new(ResourceId::generate(), new).map_err(|e| CommandError::Invalid(e.to_string()))
}

fn match_journal(
    index: &ImportEntityIndex,
    title: &str,
    issns: &[Identifier],
    entry_key: &str,
) -> Result<ContainerDecision, CommandError> {
    // Identifier-first.
    let mut saw_issn = false;
    for issn in issns {
        saw_issn = true;
        let key = issn.lookup_key();
        if let Some(id) = index.journals_by_issn.get(key) {
            if let Some(existing_title) = index.journal_titles.get(id) {
                if normalize_title(existing_title) != normalize_title(title) {
                    return Ok(ContainerDecision::Conflict(
                        ImportIssue::new(
                            Some(entry_key.to_owned()),
                            format!(
                                "ISSN {key} matches journal '{existing_title}' but title is '{title}'"
                            ),
                        )
                        .with_code(import_codes::CONFLICTING_CONTAINER),
                    ));
                }
            }
            return Ok(ContainerDecision::Reuse(*id));
        }
    }
    // Distinct ISSN that did not match an existing row identifies a new journal
    // even when the title collides (title alone is not proof of identity).
    if saw_issn {
        let mut new = blank_container(
            ResourceKind::Journal,
            NonEmptyStr::try_new(title).map_err(|e| CommandError::Invalid(e.to_string()))?,
            None,
        );
        new.issns = issns.to_vec();
        return Ok(ContainerDecision::Create(plan_resource(new)?));
    }
    let hits = index
        .journals_by_title
        .get(&normalize_title(title))
        .cloned()
        .unwrap_or_default();
    match hits.as_slice() {
        [] => {
            let mut new = blank_container(
                ResourceKind::Journal,
                NonEmptyStr::try_new(title).map_err(|e| CommandError::Invalid(e.to_string()))?,
                None,
            );
            new.issns = issns.to_vec();
            Ok(ContainerDecision::Create(plan_resource(new)?))
        }
        [id] => Ok(ContainerDecision::Reuse(*id)),
        _ => Ok(ContainerDecision::Conflict(
            ImportIssue::new(
                Some(entry_key.to_owned()),
                format!("ambiguous journal title '{title}'"),
            )
            .with_code(import_codes::AMBIGUOUS_CONTAINER),
        )),
    }
}

fn match_book(
    index: &ImportEntityIndex,
    title: &str,
    isbns: &[Identifier],
    entry_key: &str,
) -> Result<ContainerDecision, CommandError> {
    let mut saw_isbn = false;
    for isbn in isbns {
        saw_isbn = true;
        let key = isbn.lookup_key();
        if let Some(id) = index.books_by_isbn.get(key) {
            if let Some(existing_title) = index.book_titles.get(id) {
                if normalize_title(existing_title) != normalize_title(title) {
                    return Ok(ContainerDecision::Conflict(
                        ImportIssue::new(
                            Some(entry_key.to_owned()),
                            format!(
                                "ISBN {key} matches book '{existing_title}' but title is '{title}'"
                            ),
                        )
                        .with_code(import_codes::CONFLICTING_CONTAINER),
                    ));
                }
            }
            return Ok(ContainerDecision::Reuse(*id));
        }
    }
    if saw_isbn {
        let mut new = blank_container(
            ResourceKind::Book,
            NonEmptyStr::try_new(title).map_err(|e| CommandError::Invalid(e.to_string()))?,
            None,
        );
        new.isbns = isbns.to_vec();
        return Ok(ContainerDecision::Create(plan_resource(new)?));
    }
    let hits = index
        .books_by_title
        .get(&normalize_title(title))
        .cloned()
        .unwrap_or_default();
    match hits.as_slice() {
        [] => {
            let mut new = blank_container(
                ResourceKind::Book,
                NonEmptyStr::try_new(title).map_err(|e| CommandError::Invalid(e.to_string()))?,
                None,
            );
            new.isbns = isbns.to_vec();
            Ok(ContainerDecision::Create(plan_resource(new)?))
        }
        [id] => Ok(ContainerDecision::Reuse(*id)),
        _ => Ok(ContainerDecision::Conflict(
            ImportIssue::new(
                Some(entry_key.to_owned()),
                format!("ambiguous book title '{title}'"),
            )
            .with_code(import_codes::AMBIGUOUS_CONTAINER),
        )),
    }
}

fn match_or_create_issue(
    index: &ImportEntityIndex,
    journal_id: ResourceId,
    volume: Option<&str>,
    number: Option<&str>,
) -> Result<ContainerDecision, CommandError> {
    let vol = volume.map(str::trim).filter(|s| !s.is_empty());
    let num = number.map(str::trim).filter(|s| !s.is_empty());
    if vol.is_none() && num.is_none() {
        return Ok(ContainerDecision::Reuse(journal_id)); // caller treats as direct journal
    }
    let vol_key = vol.unwrap_or("").to_owned();
    let num_key = num.unwrap_or("").to_owned();
    if let Some(id) = index
        .issues
        .get(&(journal_id, vol_key.clone(), num_key.clone()))
    {
        return Ok(ContainerDecision::Reuse(*id));
    }
    let title = issue_title(vol, num);
    let mut new = blank_container(
        ResourceKind::Issue,
        NonEmptyStr::try_new(&title).map_err(|e| CommandError::Invalid(e.to_string()))?,
        Some(journal_id),
    );
    new.volume = vol.map(str::to_owned);
    new.issue = num.map(str::to_owned);
    Ok(ContainerDecision::Create(plan_resource(new)?))
}

/// Plan parent links for a child entry. Mutates `index` when creating planned entities.
pub(crate) fn plan_parent_for_entry(
    entry: &ImportEntryDraft,
    index: &mut ImportEntityIndex,
) -> Result<PlannedParent, CommandError> {
    let entry_key = entry.source_key();
    let mut planned = PlannedParent::default();

    // Explicit publication crossref/xref to an already-indexed citekey.
    for ref_key in &entry.publication_ref_keys {
        if let Some(id) = index.by_citekey.get(ref_key).copied() {
            // Direct link when the referenced entry is an allowed parent kind.
            // For articles referencing a journal, still build an issue when evidence exists.
            match entry.kind() {
                ResourceKind::JournalArticle => {
                    plan_article_under_journal(
                        entry,
                        index,
                        ContainerDecision::Reuse(id),
                        &mut planned,
                    )?;
                    return Ok(planned);
                }
                ResourceKind::BookSection
                | ResourceKind::DictionaryEntry
                | ResourceKind::EncyclopediaArticle => {
                    planned.parent_id = Some(id);
                    planned.linked = true;
                    return Ok(planned);
                }
                _ => {}
            }
        }
    }

    match entry.kind() {
        ResourceKind::JournalArticle => {
            let title = entry
                .resource
                .metadata
                .venue
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let Some(title) = title else {
                return Ok(planned); // no reliable journal identity
            };
            let decision = match_journal(index, title, &entry.resource.issns, entry_key)?;
            plan_article_under_journal(entry, index, decision, &mut planned)?;
        }
        ResourceKind::BookSection
        | ResourceKind::DictionaryEntry
        | ResourceKind::EncyclopediaArticle => {
            let title = entry
                .resource
                .metadata
                .venue
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let Some(title) = title else {
                return Ok(planned);
            };
            match match_book(index, title, &entry.resource.isbns, entry_key)? {
                ContainerDecision::Reuse(id) => {
                    planned.parent_id = Some(id);
                    planned.linked = true;
                }
                ContainerDecision::Create(book) => {
                    index.register_planned_book(&book);
                    let id = book.id;
                    planned.new_dependencies.push(book);
                    planned.parent_id = Some(id);
                    planned.linked = true;
                }
                ContainerDecision::Conflict(issue) => planned.conflicts.push(issue),
            }
        }
        _ => {}
    }
    Ok(planned)
}

fn plan_article_under_journal(
    entry: &ImportEntryDraft,
    index: &mut ImportEntityIndex,
    journal_decision: ContainerDecision,
    planned: &mut PlannedParent,
) -> Result<(), CommandError> {
    let journal_id = match journal_decision {
        ContainerDecision::Reuse(id) => id,
        ContainerDecision::Create(journal) => {
            index.register_planned_journal(&journal);
            let id = journal.id;
            planned.new_dependencies.push(journal);
            id
        }
        ContainerDecision::Conflict(issue) => {
            planned.conflicts.push(issue);
            return Ok(());
        }
    };

    let has_issue_evidence = entry
        .resource
        .volume
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
        || entry
            .resource
            .issue
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty());

    if !has_issue_evidence {
        planned.parent_id = Some(journal_id);
        planned.linked = true;
        return Ok(());
    }

    match match_or_create_issue(
        index,
        journal_id,
        entry.resource.volume.as_deref(),
        entry.resource.issue.as_deref(),
    )? {
        ContainerDecision::Reuse(id) if id == journal_id => {
            planned.parent_id = Some(journal_id);
            planned.linked = true;
        }
        ContainerDecision::Reuse(id) => {
            planned.parent_id = Some(id);
            planned.linked = true;
        }
        ContainerDecision::Create(issue) => {
            index.register_planned_issue(&issue);
            let id = issue.id;
            planned.new_dependencies.push(issue);
            planned.parent_id = Some(id);
            planned.linked = true;
        }
        ContainerDecision::Conflict(issue) => planned.conflicts.push(issue),
    }
    Ok(())
}

/// When an explicit journal/book source entry is saved, match or create without
/// inventing a second copy of an inferred container.
pub(crate) fn resolve_explicit_container(
    entry: &ImportEntryDraft,
    index: &mut ImportEntityIndex,
) -> Result<Option<ContainerDecision>, CommandError> {
    let entry_key = entry.source_key();
    match entry.kind() {
        ResourceKind::Journal => {
            let decision = match_journal(
                index,
                entry.title().as_str(),
                &entry.resource.issns,
                entry_key,
            )?;
            Ok(Some(decision))
        }
        ResourceKind::Book => {
            let decision = match_book(
                index,
                entry.title().as_str(),
                &entry.resource.isbns,
                entry_key,
            )?;
            Ok(Some(decision))
        }
        _ => Ok(None),
    }
}
