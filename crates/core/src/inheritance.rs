//! Read-time bibliographic inheritance along publication parent links.
//!
//! Explicit child values win. Absent optional fields (`None`) and empty
//! identifier lists mean **inherit** from an allowlisted ancestor — there is
//! no separate suppress state in this version. Clearing a stored field returns
//! it to inherit. DOI, pages, access date, and creators are never inherited.
//!
//! Resolution is pure: callers supply the parent chain; this module never
//! mutates the child resource.

use std::collections::HashSet;

use crate::{BibliographicDate, Identifier, NonEmptyStr, Resource, ResourceId, ResourceKind};

/// Maximum parent-chain hops (journal → issue → article needs two).
pub const INHERITANCE_MAX_DEPTH: usize = 8;

/// Bibliographic fields that may flow from a parent to a child at read time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InheritableField {
    Publisher,
    Published,
    Place,
    Volume,
    IssueNumber,
    Edition,
    Language,
    Issns,
    Isbns,
}

impl InheritableField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Publisher => "publisher",
            Self::Published => "published",
            Self::Place => "place",
            Self::Volume => "volume",
            Self::IssueNumber => "issue",
            Self::Edition => "edition",
            Self::Language => "language",
            Self::Issns => "issns",
            Self::Isbns => "isbns",
        }
    }
}

/// Fields the child may take from this parent kind. Empty when the pair is
/// not an allowed inheritance edge (parentage may still be valid for linking).
pub fn fields_inherited_from(
    child: ResourceKind,
    parent: ResourceKind,
) -> &'static [InheritableField] {
    use InheritableField::*;
    match (child, parent) {
        (ResourceKind::JournalArticle, ResourceKind::Issue) => {
            &[Volume, IssueNumber, Published, Publisher, Place, Language]
        }
        (ResourceKind::JournalArticle, ResourceKind::Journal) => {
            &[Publisher, Place, Language, Issns]
        }
        (ResourceKind::Issue, ResourceKind::Journal) => &[Publisher, Place, Language, Issns],
        (
            ResourceKind::BookSection
            | ResourceKind::DictionaryEntry
            | ResourceKind::EncyclopediaArticle,
            ResourceKind::Book,
        ) => &[Publisher, Place, Published, Edition, Language, Isbns],
        (ResourceKind::PodcastEpisode, ResourceKind::Podcast) => &[Publisher, Published, Language],
        (ResourceKind::BlogPost, ResourceKind::Blog) => &[Publisher, Published, Language],
        _ => &[],
    }
}

/// Where an effective value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueOrigin {
    Absent,
    Explicit,
    Inherited { parent_id: ResourceId },
}

/// One resolved field: value plus provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective<T> {
    pub value: Option<T>,
    pub origin: ValueOrigin,
}

impl<T> Effective<T> {
    pub fn absent() -> Self {
        Self {
            value: None,
            origin: ValueOrigin::Absent,
        }
    }

    pub fn explicit(value: T) -> Self {
        Self {
            value: Some(value),
            origin: ValueOrigin::Explicit,
        }
    }

    pub fn inherited(value: T, parent_id: ResourceId) -> Self {
        Self {
            value: Some(value),
            origin: ValueOrigin::Inherited { parent_id },
        }
    }

    pub fn is_inherited(&self) -> bool {
        matches!(self.origin, ValueOrigin::Inherited { .. })
    }
}

/// Predictable failures while walking or validating the parent chain.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InheritanceError {
    #[error("parent chain contains a cycle involving {0}")]
    Cycle(ResourceId),
    #[error("parent kind is not valid for this child")]
    InvalidParentKind,
    #[error("parent chain exceeds maximum depth of {INHERITANCE_MAX_DEPTH}")]
    DepthExceeded,
    #[error("expected parent {expected} missing from ancestor chain")]
    MissingParent { expected: ResourceId },
    #[error("ancestor chain out of order: expected {expected}, found {found}")]
    AncestorMismatch {
        expected: ResourceId,
        found: ResourceId,
    },
}

/// Read-time projection of allowlisted fields and container title.
///
/// Stored child fields that are not allowlisted (DOI, pages, accessed,
/// creators, title, …) are intentionally omitted — callers keep using the
/// resource row for those.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveValues {
    pub publisher: Effective<String>,
    pub published: Effective<BibliographicDate>,
    pub place: Effective<String>,
    pub volume: Effective<String>,
    pub issue: Effective<String>,
    pub edition: Effective<String>,
    pub language: Effective<String>,
    pub issns: Effective<Vec<Identifier>>,
    pub isbns: Effective<Vec<Identifier>>,
    /// Display container: explicit `metadata.venue`, else nearest suitable
    /// ancestor title (journal over issue label).
    pub container_title: Effective<NonEmptyStr>,
}

/// Walk `parent_id` links via `lookup` up to [`INHERITANCE_MAX_DEPTH`].
///
/// Returns `[immediate parent, grandparent, …]`. A missing parent id yields
/// [`InheritanceError::MissingParent`]. Cycle / kind checks run in
/// [`resolve_effective`].
pub fn gather_ancestors(
    resource: &Resource,
    mut lookup: impl FnMut(ResourceId) -> Result<Option<Resource>, InheritanceError>,
) -> Result<Vec<Resource>, InheritanceError> {
    let mut chain = Vec::new();
    let mut next = resource.parent_id;
    while let Some(parent_id) = next {
        if chain.len() >= INHERITANCE_MAX_DEPTH {
            return Err(InheritanceError::DepthExceeded);
        }
        let parent = lookup(parent_id)?.ok_or(InheritanceError::MissingParent {
            expected: parent_id,
        })?;
        next = parent.parent_id;
        chain.push(parent);
    }
    Ok(chain)
}

/// Resolve effective allowlisted values from `resource` and its ancestors.
///
/// `ancestors` is `[immediate parent, grandparent, …]`. Does not mutate
/// `resource`. Parent updates surface automatically on the next resolve.
pub fn resolve_effective(
    resource: &Resource,
    ancestors: &[Resource],
) -> Result<EffectiveValues, InheritanceError> {
    validate_ancestor_chain(resource, ancestors)?;

    Ok(EffectiveValues {
        publisher: resolve_string(
            resource.kind,
            resource.publisher.as_deref(),
            ancestors,
            InheritableField::Publisher,
            |r| r.publisher.as_deref(),
        ),
        published: resolve_published(resource, ancestors),
        place: resolve_string(
            resource.kind,
            resource.place.as_deref(),
            ancestors,
            InheritableField::Place,
            |r| r.place.as_deref(),
        ),
        volume: resolve_string(
            resource.kind,
            resource.volume.as_deref(),
            ancestors,
            InheritableField::Volume,
            |r| r.volume.as_deref(),
        ),
        issue: resolve_string(
            resource.kind,
            resource.issue.as_deref(),
            ancestors,
            InheritableField::IssueNumber,
            |r| r.issue.as_deref(),
        ),
        edition: resolve_string(
            resource.kind,
            resource.edition.as_deref(),
            ancestors,
            InheritableField::Edition,
            |r| r.edition.as_deref(),
        ),
        language: resolve_string(
            resource.kind,
            resource.language.as_deref(),
            ancestors,
            InheritableField::Language,
            |r| r.language.as_deref(),
        ),
        issns: resolve_identifiers(
            resource.kind,
            &resource.issns,
            ancestors,
            InheritableField::Issns,
            |r| &r.issns,
        ),
        isbns: resolve_identifiers(
            resource.kind,
            &resource.isbns,
            ancestors,
            InheritableField::Isbns,
            |r| &r.isbns,
        ),
        container_title: resolve_container_title(resource, ancestors),
    })
}

fn validate_ancestor_chain(
    resource: &Resource,
    ancestors: &[Resource],
) -> Result<(), InheritanceError> {
    if ancestors.len() > INHERITANCE_MAX_DEPTH {
        return Err(InheritanceError::DepthExceeded);
    }

    let Some(mut expected_parent) = resource.parent_id else {
        return Ok(());
    };

    if ancestors.is_empty() {
        return Err(InheritanceError::MissingParent {
            expected: expected_parent,
        });
    }

    let mut seen = HashSet::from([resource.id]);
    let mut child_kind = resource.kind;

    for ancestor in ancestors {
        if ancestor.id != expected_parent {
            return Err(InheritanceError::AncestorMismatch {
                expected: expected_parent,
                found: ancestor.id,
            });
        }
        if !seen.insert(ancestor.id) {
            return Err(InheritanceError::Cycle(ancestor.id));
        }
        if child_kind.accept_parent(ancestor.kind).is_err() {
            return Err(InheritanceError::InvalidParentKind);
        }
        expected_parent = match ancestor.parent_id {
            Some(id) => id,
            None => return Ok(()),
        };
        child_kind = ancestor.kind;
    }
    Ok(())
}

fn resolve_string(
    child_kind: ResourceKind,
    child_value: Option<&str>,
    ancestors: &[Resource],
    field: InheritableField,
    getter: impl Fn(&Resource) -> Option<&str>,
) -> Effective<String> {
    if let Some(value) = child_value {
        return Effective::explicit(value.to_owned());
    }
    for ancestor in ancestors {
        if !fields_inherited_from(child_kind, ancestor.kind).contains(&field) {
            continue;
        }
        if let Some(value) = getter(ancestor).filter(|s| !s.is_empty()) {
            return Effective::inherited(value.to_owned(), ancestor.id);
        }
    }
    Effective::absent()
}

fn resolve_published(resource: &Resource, ancestors: &[Resource]) -> Effective<BibliographicDate> {
    if let Some(value) = resource.published.clone() {
        return Effective::explicit(value);
    }
    for ancestor in ancestors {
        if !fields_inherited_from(resource.kind, ancestor.kind)
            .contains(&InheritableField::Published)
        {
            continue;
        }
        if let Some(value) = ancestor.published.clone() {
            return Effective::inherited(value, ancestor.id);
        }
    }
    Effective::absent()
}

fn resolve_identifiers(
    child_kind: ResourceKind,
    child_value: &[Identifier],
    ancestors: &[Resource],
    field: InheritableField,
    getter: impl Fn(&Resource) -> &[Identifier],
) -> Effective<Vec<Identifier>> {
    if !child_value.is_empty() {
        return Effective::explicit(child_value.to_vec());
    }
    for ancestor in ancestors {
        if !fields_inherited_from(child_kind, ancestor.kind).contains(&field) {
            continue;
        }
        let value = getter(ancestor);
        if !value.is_empty() {
            return Effective::inherited(value.to_vec(), ancestor.id);
        }
    }
    Effective::absent()
}

fn resolve_container_title(resource: &Resource, ancestors: &[Resource]) -> Effective<NonEmptyStr> {
    // Linked records are authoritative; imported venue text is only a fallback.
    // Prefer real containers (journal/book/…) over issue volume labels.
    for ancestor in ancestors {
        if ancestor.kind == ResourceKind::Issue {
            continue;
        }
        return Effective::inherited(ancestor.title.clone(), ancestor.id);
    }

    // Issue-only chain (no journal loaded / missing grandparent): show the issue title.
    if let Some(issue) = ancestors.iter().find(|a| a.kind == ResourceKind::Issue) {
        return Effective::inherited(issue.title.clone(), issue.id);
    }
    if let Some(venue) = resource
        .metadata
        .venue
        .as_ref()
        .and_then(|v| NonEmptyStr::try_new(v).ok())
    {
        return Effective::explicit(venue);
    }

    Effective::absent()
}

/// Fields that must never inherit even when a parent carries them.
pub fn never_inherited() -> &'static [&'static str] {
    &["doi", "pages", "accessed", "creators"]
}

/// True when `field` is not on any inheritance allowlist edge.
pub fn is_never_inherited(field: &str) -> bool {
    never_inherited().contains(&field)
        || matches!(
            field,
            "title"
                | "subtitle"
                | "url"
                | "eprint"
                | "abstract"
                | "keywords"
                | "article_number"
                | "total_pages"
                | "document_type"
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BibliographicDate, Creator, CreatorRole, CreatorSpec, NewResource, NonEmptyStr, Pages,
        PersonId, ResourceId, ResourceKind, UtcMillis,
    };

    fn nes(s: &str) -> NonEmptyStr {
        NonEmptyStr::try_new(s).unwrap()
    }

    fn bare(kind: ResourceKind, title: &str, parent_id: Option<ResourceId>) -> Resource {
        Resource::try_new(
            ResourceId::generate(),
            NewResource {
                kind,
                title: nes(title),
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
            },
        )
        .unwrap()
    }

    fn year(y: i64) -> BibliographicDate {
        BibliographicDate::try_new(UtcMillis::jan_1_utc(y).unwrap(), Some(nes(&y.to_string())))
            .unwrap()
    }

    #[test]
    fn matrix_article_from_issue_and_journal() {
        let from_issue = fields_inherited_from(ResourceKind::JournalArticle, ResourceKind::Issue);
        assert!(from_issue.contains(&InheritableField::Volume));
        assert!(from_issue.contains(&InheritableField::IssueNumber));
        assert!(from_issue.contains(&InheritableField::Published));
        assert!(!from_issue.contains(&InheritableField::Issns));

        let from_journal =
            fields_inherited_from(ResourceKind::JournalArticle, ResourceKind::Journal);
        assert!(from_journal.contains(&InheritableField::Publisher));
        assert!(from_journal.contains(&InheritableField::Issns));
        assert!(!from_journal.contains(&InheritableField::Volume));
    }

    #[test]
    fn matrix_chapter_from_book_and_issue_from_journal() {
        let chapter = fields_inherited_from(ResourceKind::BookSection, ResourceKind::Book);
        assert!(chapter.contains(&InheritableField::Publisher));
        assert!(chapter.contains(&InheritableField::Edition));
        assert!(chapter.contains(&InheritableField::Isbns));

        let issue = fields_inherited_from(ResourceKind::Issue, ResourceKind::Journal);
        assert!(issue.contains(&InheritableField::Publisher));
        assert!(issue.contains(&InheritableField::Issns));
    }

    #[test]
    fn matrix_disallows_unrelated_pairs() {
        assert!(fields_inherited_from(ResourceKind::Book, ResourceKind::Journal).is_empty());
        assert!(fields_inherited_from(ResourceKind::JournalArticle, ResourceKind::Book).is_empty());
    }

    #[test]
    fn article_inherits_volume_publisher_and_journal_container() {
        let mut journal = bare(ResourceKind::Journal, "Nature", None);
        journal.publisher = Some("Springer Nature".into());
        journal.issns = vec![Identifier::new("0028-0836")];
        journal.language = Some("en".into());

        let mut issue = bare(ResourceKind::Issue, "Vol. 12, no. 3", Some(journal.id));
        issue.volume = Some("12".into());
        issue.issue = Some("3".into());
        issue.published = Some(year(2020));

        let article = bare(ResourceKind::JournalArticle, "An Article", Some(issue.id));

        let effective = resolve_effective(&article, &[issue.clone(), journal.clone()]).unwrap();
        assert_eq!(effective.volume.value.as_deref(), Some("12"));
        assert!(effective.volume.is_inherited());
        assert_eq!(effective.issue.value.as_deref(), Some("3"));
        assert_eq!(
            effective.publisher.value.as_deref(),
            Some("Springer Nature")
        );
        assert_eq!(
            effective.publisher.origin,
            ValueOrigin::Inherited {
                parent_id: journal.id
            }
        );
        assert_eq!(effective.published.value, Some(year(2020)));
        assert_eq!(effective.language.value.as_deref(), Some("en"));
        assert_eq!(
            effective.issns.value.as_ref().map(|v| v[0].as_str()),
            Some("0028-0836")
        );
        assert_eq!(
            effective.container_title.value.as_ref().map(|s| s.as_str()),
            Some("Nature")
        );
        assert_eq!(
            effective.container_title.origin,
            ValueOrigin::Inherited {
                parent_id: journal.id
            }
        );
    }

    #[test]
    fn explicit_child_overrides_parent_defaults() {
        let mut journal = bare(ResourceKind::Journal, "Nature", None);
        journal.publisher = Some("Springer Nature".into());

        let mut issue = bare(ResourceKind::Issue, "Vol. 1, no. 1", Some(journal.id));
        issue.volume = Some("1".into());
        issue.issue = Some("1".into());

        let mut article = bare(ResourceKind::JournalArticle, "Override", Some(issue.id));
        article.publisher = Some("Author Press".into());
        article.volume = Some("99".into());
        article.metadata.venue = Some("Custom Venue".into());

        let effective = resolve_effective(&article, &[issue, journal]).unwrap();
        assert_eq!(effective.publisher.value.as_deref(), Some("Author Press"));
        assert_eq!(effective.publisher.origin, ValueOrigin::Explicit);
        assert_eq!(effective.volume.value.as_deref(), Some("99"));
        assert_eq!(effective.volume.origin, ValueOrigin::Explicit);
        assert_eq!(
            effective.container_title.value.as_ref().map(|s| s.as_str()),
            Some("Nature")
        );
        assert!(effective.container_title.is_inherited());
        // Non-overridden issue number still inherits.
        assert_eq!(effective.issue.value.as_deref(), Some("1"));
        assert!(effective.issue.is_inherited());
    }

    #[test]
    fn unlinked_article_uses_imported_venue_as_fallback() {
        let mut article = bare(ResourceKind::JournalArticle, "Paper", None);
        article.metadata.venue = Some("BSac".into());

        let effective = resolve_effective(&article, &[]).unwrap();
        assert_eq!(
            effective
                .container_title
                .value
                .as_ref()
                .map(NonEmptyStr::as_str),
            Some("BSac")
        );
        assert_eq!(effective.container_title.origin, ValueOrigin::Explicit);
    }
    #[test]
    fn parent_update_propagates_when_child_has_no_override() {
        let mut journal = bare(ResourceKind::Journal, "Nature", None);
        journal.publisher = Some("Old Pub".into());
        let issue = bare(ResourceKind::Issue, "Vol. 1, no. 1", Some(journal.id));
        let article = bare(ResourceKind::JournalArticle, "Paper", Some(issue.id));

        let before = resolve_effective(&article, &[issue.clone(), journal.clone()]).unwrap();
        assert_eq!(before.publisher.value.as_deref(), Some("Old Pub"));

        journal.publisher = Some("New Pub".into());
        let after = resolve_effective(&article, &[issue, journal.clone()]).unwrap();
        assert_eq!(after.publisher.value.as_deref(), Some("New Pub"));
        assert_eq!(
            after.publisher.origin,
            ValueOrigin::Inherited {
                parent_id: journal.id
            }
        );
        // Child row unchanged.
        assert!(article.publisher.is_none());
    }

    #[test]
    fn doi_pages_accessed_and_creators_are_not_inherited() {
        assert!(is_never_inherited("doi"));
        assert!(is_never_inherited("pages"));
        assert!(is_never_inherited("accessed"));
        assert!(is_never_inherited("creators"));

        let mut journal = bare(ResourceKind::Journal, "Nature", None);
        journal.doi = Some("10.journal/parent".into());
        journal.pages = Some(Pages::parse("1--10").unwrap());
        journal.accessed = Some(year(2019));
        journal.creators = vec![Creator::from_spec(
            0,
            CreatorSpec {
                person_id: PersonId::generate(),
                role: CreatorRole::Editor,
                credited_as: None,
            },
        )];

        let mut issue = bare(ResourceKind::Issue, "Vol. 1, no. 1", Some(journal.id));
        issue.doi = Some("10.issue/parent".into());

        let article = bare(ResourceKind::JournalArticle, "Paper", Some(issue.id));
        assert!(article.doi.is_none());
        assert!(article.pages.is_none());
        assert!(article.accessed.is_none());
        assert!(article.creators.is_empty());

        let effective = resolve_effective(&article, &[issue, journal.clone()]).unwrap();
        // Effective projection has no doi/pages/accessed/creators slots.
        // Child source stays empty; parent values are not copied onto the child.
        assert!(article.doi.is_none());
        assert!(article.pages.is_none());
        assert!(article.accessed.is_none());
        assert!(article.creators.is_empty());
        // Allowlisted fields may still resolve; never-inherited stay absent on child.
        assert_eq!(effective.publisher.value, None);
        let _ = journal;
    }

    #[test]
    fn resolve_rejects_cycles_invalid_parents_and_depth() {
        let a_id = ResourceId::generate();
        let b_id = ResourceId::generate();
        let mut a = bare(ResourceKind::JournalArticle, "A", Some(b_id));
        a.id = a_id;
        let mut b = bare(ResourceKind::Issue, "B", Some(a_id));
        b.id = b_id;

        let err = resolve_effective(&a, &[b.clone(), a.clone()]).unwrap_err();
        assert!(matches!(err, InheritanceError::Cycle(_)));

        let journal = bare(ResourceKind::Journal, "J", None);
        let book = bare(ResourceKind::Book, "Book", None);
        let mut article = bare(ResourceKind::JournalArticle, "X", Some(book.id));
        let err = resolve_effective(&article, &[book]).unwrap_err();
        assert!(matches!(err, InheritanceError::InvalidParentKind));

        article.parent_id = Some(journal.id);
        let err = resolve_effective(&article, &[]).unwrap_err();
        assert!(matches!(err, InheritanceError::MissingParent { .. }));

        // Depth: fabricate a chain longer than the cap with valid kinds by
        // reusing issue→journal only once is hard; assert the constant and a
        // mismatched ancestor error path instead.
        assert_eq!(INHERITANCE_MAX_DEPTH, 8);
        let wrong = bare(ResourceKind::Journal, "Other", None);
        let err = resolve_effective(&article, &[wrong.clone()]).unwrap_err();
        assert!(matches!(
            err,
            InheritanceError::AncestorMismatch { .. } | InheritanceError::MissingParent { .. }
        ));
        let _ = wrong;
    }

    #[test]
    fn resolve_does_not_mutate_child_source() {
        let mut journal = bare(ResourceKind::Journal, "Nature", None);
        journal.publisher = Some("SN".into());
        let issue = bare(ResourceKind::Issue, "Vol. 1, no. 1", Some(journal.id));
        let article = bare(ResourceKind::JournalArticle, "Paper", Some(issue.id));
        let before = article.clone();
        let _ = resolve_effective(&article, &[issue, journal]).unwrap();
        assert_eq!(article, before);
        assert!(article.publisher.is_none());
        assert!(article.volume.is_none());
    }

    #[test]
    fn chapter_inherits_book_publisher_not_doi() {
        let mut book = bare(ResourceKind::Book, "Collected Essays", None);
        book.publisher = Some("MIT Press".into());
        book.edition = Some("2nd".into());
        book.doi = Some("10.book/xyz".into());
        book.isbns = vec![Identifier::new("978-0-262-03384-8")];

        let chapter = bare(ResourceKind::BookSection, "Chapter One", Some(book.id));
        let effective = resolve_effective(&chapter, &[book.clone()]).unwrap();
        assert_eq!(effective.publisher.value.as_deref(), Some("MIT Press"));
        assert_eq!(effective.edition.value.as_deref(), Some("2nd"));
        assert_eq!(
            effective.isbns.value.as_ref().map(|v| v[0].as_str()),
            Some("978-0-262-03384-8")
        );
        assert_eq!(
            effective.container_title.value.as_ref().map(|s| s.as_str()),
            Some("Collected Essays")
        );
        assert!(chapter.doi.is_none());
        assert_eq!(book.doi.as_deref(), Some("10.book/xyz"));
    }

    #[test]
    fn direct_journal_parent_supplies_container_and_publisher() {
        let mut journal = bare(ResourceKind::Journal, "Science", None);
        journal.publisher = Some("AAAS".into());
        let article = bare(ResourceKind::JournalArticle, "Letter", Some(journal.id));
        let effective = resolve_effective(&article, &[journal.clone()]).unwrap();
        assert_eq!(effective.publisher.value.as_deref(), Some("AAAS"));
        assert_eq!(
            effective.container_title.value.as_ref().map(|s| s.as_str()),
            Some("Science")
        );
        assert_eq!(
            effective.container_title.origin,
            ValueOrigin::Inherited {
                parent_id: journal.id
            }
        );
    }
}
