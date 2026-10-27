//! Edit form for one resource's scalar fields: the panel's write-side twin.
//!
//! Value logic here stays window-free (draft in, payload out), so the merge
//! rules run under plain unit tests; `LibraryApp` owns the input entities
//! and wires them to the worker.

use gpui_kit::SharedString;
use gpui_kit::component::searchable_list::SearchableListItem;

use oeuvre_core::{
    BibliographicDate, CollectionId, CreatorRole, CreatorSpec, NewResource, NonEmptyStr, PersonId,
    Resource, ResourceId, ResourceKind, SeriesId, TagId, UtcMillis, utc_year_from_millis,
};

use crate::kind::kind_label;

/// One value per scalar the form edits. The panel reads its input entities
/// into this plain shape at save time, so the payload build below stays
/// window-free and unit-testable. Parent and series are ids, not free
/// text: the parent arrives from a dropdown keyed by id, the series rides
/// through until series storage lands a listing API. Creators are person
/// specs; the inline multi-select picks people and defaults new picks to
/// Author.
pub struct EditDraft {
    pub title: String,
    pub kind: ResourceKind,
    pub parent: Option<ResourceId>,
    pub publisher: String,
    pub published_year: String,
    pub accessed_year: String,
    pub series: String,
    pub series_number: String,
    pub creators: Vec<CreatorSpec>,
    /// Tags attached on save, in the order shown on the form.
    pub tag_ids: Vec<TagId>,
    /// Collections attached on save, in the order shown on the form.
    pub collection_ids: Vec<CollectionId>,
}

/// The value each input opens with: the stored record's own values,
/// optional fields empty (or none) when the record carries nothing.
pub fn prefill_draft(record: &Resource) -> EditDraft {
    EditDraft {
        title: record.title.as_str().to_owned(),
        kind: record.kind,
        parent: record.parent_id,
        publisher: record.publisher.clone().unwrap_or_default(),
        published_year: date_year(record.published.as_ref()),
        accessed_year: date_year(record.accessed.as_ref()),
        series: record
            .series_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        series_number: record
            .series_number
            .as_ref()
            .map(|number| number.as_str().to_owned())
            .unwrap_or_default(),
        creators: record.to_new().creators,
        tag_ids: record.tag_ids.clone(),
        collection_ids: record.collection_ids.clone(),
    }
}

/// Empty draft for creating a new Book row inline: no title yet, no
/// creators, no optionals. Save validates the title before create.
pub fn empty_create_draft() -> EditDraft {
    EditDraft {
        title: String::new(),
        kind: ResourceKind::Book,
        parent: None,
        publisher: String::new(),
        published_year: String::new(),
        accessed_year: String::new(),
        series: String::new(),
        series_number: String::new(),
        creators: Vec::new(),
        tag_ids: Vec::new(),
        collection_ids: Vec::new(),
    }
}

/// Draft to full-replace payload, over the record's write-side twin: an
/// emptied optional field clears exactly that field, the rest ride through
/// as stored. The `String` errors are the form's to show. Parent arrives
/// already keyed by id from the dropdown, so it passes through unvalidated:
/// the dropdown's options carry only stored ids.
pub fn build_update(record: &Resource, draft: &EditDraft) -> Result<NewResource, String> {
    let mut edit = record.to_new();
    apply_draft_fields(&mut edit, draft)?;
    Ok(edit)
}

/// Draft to a create payload: same scalar rules as update, no stored
/// record to ride through. Parent/series/accessed stay whatever the draft
/// carries (inline create leaves them empty).
pub fn build_create(draft: &EditDraft) -> Result<NewResource, String> {
    let mut new = NewResource {
        kind: draft.kind,
        title: NonEmptyStr::try_new("placeholder").expect("literal"),
        creators: Vec::new(),
        publisher: None,
        published: None,
        accessed: None,
        parent_id: None,
        series_id: None,
        series_number: None,
        tag_ids: vec![],
        collection_ids: vec![],
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
    };
    apply_draft_fields(&mut new, draft)?;
    Ok(new)
}

/// Shared field merge for update and create payloads.
fn apply_draft_fields(edit: &mut NewResource, draft: &EditDraft) -> Result<(), String> {
    let title = draft.title.trim();
    edit.title = NonEmptyStr::try_new(title).map_err(|_| "Title must not be empty".to_owned())?;

    edit.kind = draft.kind;
    edit.parent_id = draft.parent;
    edit.creators = draft.creators.clone();
    edit.tag_ids = draft.tag_ids.clone();
    edit.collection_ids = draft.collection_ids.clone();

    let publisher = draft.publisher.trim();
    edit.publisher = (!publisher.is_empty()).then(|| publisher.to_owned());

    edit.published = year_field("Published year", &draft.published_year)?;
    edit.accessed = year_field("Accessed year", &draft.accessed_year)?;

    let series = draft.series.trim();
    edit.series_id = (!series.is_empty())
        .then(|| SeriesId::parse(series).map_err(|_| "Series must be a series id".to_owned()))
        .transpose()?;

    let number = draft.series_number.trim();
    edit.series_number = (!number.is_empty())
        .then(|| {
            NonEmptyStr::try_new(number).map_err(|_| "Series number must not be empty".to_owned())
        })
        .transpose()?;

    Ok(())
}

/// Person ids from a multi-select, merged into creator specs: keep role and
/// credited-as when the person was already on the record, otherwise Author.
pub fn creators_from_person_ids(
    person_ids: &[PersonId],
    previous: &[CreatorSpec],
) -> Vec<CreatorSpec> {
    person_ids
        .iter()
        .map(|person_id| {
            previous
                .iter()
                .find(|c| c.person_id == *person_id)
                .cloned()
                .unwrap_or(CreatorSpec {
                    person_id: *person_id,
                    role: CreatorRole::Author,
                    credited_as: None,
                })
        })
        .collect()
}

/// One dropdown row for the kind box: the label reads, the kind keys.
#[derive(Clone)]
pub struct KindItem {
    label: SharedString,
    kind: ResourceKind,
}

impl SearchableListItem for KindItem {
    type Value = ResourceKind;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.kind
    }
}

/// Every kind the form offers, labelled the way the grid and panel label
/// them, alphabetized so the dropdown reads in order.
pub fn kind_items() -> Vec<KindItem> {
    let mut items: Vec<KindItem> = ResourceKind::ALL
        .into_iter()
        .map(|kind| KindItem {
            label: kind_label(kind).into(),
            kind,
        })
        .collect();
    items.sort_by_key(|item| item.label.to_lowercase());
    items
}

/// The stored date's UTC year as text; empty when no date is set. The form
/// edits whole years, so the fuzzy display string never enters the input.
fn date_year(date: Option<&BibliographicDate>) -> String {
    date.map(|date| utc_year_from_millis(date.instant().as_unix_ms()).to_string())
        .unwrap_or_default()
}

/// Year text to the year's January 1 UTC date; empty clears the field,
/// anything unparseable is the form's error to show.
fn year_field(label: &str, text: &str) -> Result<Option<BibliographicDate>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let year: i64 = text
        .parse()
        .map_err(|_| format!("{label} must be a whole year, got '{text}'"))?;
    let instant =
        UtcMillis::jan_1_utc(year).ok_or_else(|| format!("{label} is outside the calendar"))?;
    BibliographicDate::try_new(instant, None)
        .map(Some)
        .map_err(|_| format!("{label} is outside the calendar"))
}

#[cfg(test)]
mod tests {
    use super::{
        EditDraft, KindItem, SearchableListItem, build_create, build_update,
        creators_from_person_ids, empty_create_draft, kind_items, prefill_draft,
    };
    use oeuvre_core::{
        BibliographicDate, CreatorRole, CreatorSpec, NewResource, NonEmptyStr, PersonId, Resource,
        ResourceId, ResourceKind, SeriesId, UtcMillis,
    };

    /// A record carrying every scalar the form edits, in a shape the form
    /// can round-trip: book with one author, publisher, both dates, a
    /// parent link, series, and series number.
    fn loaded_record() -> Resource {
        let author = PersonId::generate();
        Resource::try_new(
            ResourceId::generate(),
            NewResource {
                kind: ResourceKind::BookSection,
                title: NonEmptyStr::try_new("Citation Networks").expect("title non-empty"),
                creators: vec![CreatorSpec {
                    person_id: author,
                    role: CreatorRole::Author,
                    credited_as: Some(NonEmptyStr::try_new("A. Author").expect("credited")),
                }],
                publisher: Some("MIT Press".into()),
                published: Some(
                    BibliographicDate::try_new(
                        UtcMillis::jan_1_utc(1991).expect("year in calendar"),
                        None,
                    )
                    .expect("date valid"),
                ),
                accessed: Some(
                    BibliographicDate::try_new(
                        UtcMillis::jan_1_utc(2020).expect("year in calendar"),
                        None,
                    )
                    .expect("date valid"),
                ),
                parent_id: Some(ResourceId::generate()),
                series_id: Some(SeriesId::generate()),
                series_number: Some(NonEmptyStr::try_new("LCL 26").expect("number")),
                tag_ids: vec![],
                collection_ids: vec![],
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
        .expect("record valid")
    }

    fn draft_with(title: &str, creators: Vec<CreatorSpec>) -> EditDraft {
        EditDraft {
            title: title.into(),
            kind: ResourceKind::BookSection,
            parent: None,
            publisher: "MIT Press".into(),
            published_year: "1991".into(),
            accessed_year: "2020".into(),
            series: "".into(),
            series_number: "".into(),
            creators,
            tag_ids: Vec::new(),
            collection_ids: vec![],
        }
    }

    #[test]
    fn prefill_copies_every_stored_field() {
        let record = loaded_record();
        let draft = prefill_draft(&record);
        assert_eq!(draft.title, "Citation Networks");
        assert_eq!(draft.kind, ResourceKind::BookSection);
        assert_eq!(draft.parent, record.parent_id);
        assert_eq!(draft.publisher, "MIT Press");
        assert_eq!(draft.published_year, "1991");
        assert_eq!(draft.accessed_year, "2020");
        assert_eq!(
            draft.series,
            record.series_id.expect("series set").to_string()
        );
        assert_eq!(draft.series_number, "LCL 26");
        assert_eq!(draft.creators.len(), 1);
        assert_eq!(draft.creators[0].person_id, record.creators[0].person_id);
    }

    #[test]
    fn prefill_empties_absent_optionals() {
        let mut record = loaded_record();
        let mut edit = record.to_new();
        edit.publisher = None;
        edit.published = None;
        edit.accessed = None;
        edit.parent_id = None;
        edit.series_id = None;
        edit.series_number = None;
        edit.creators = Vec::new();
        record = Resource::try_new(record.id, edit).expect("stripped record valid");

        let draft = prefill_draft(&record);
        assert_eq!(draft.parent, None);
        assert_eq!(draft.publisher, "");
        assert_eq!(draft.published_year, "");
        assert_eq!(draft.accessed_year, "");
        assert_eq!(draft.series, "");
        assert_eq!(draft.series_number, "");
        assert!(draft.creators.is_empty());
        // Title is never optional, so it still prefills.
        assert_eq!(draft.title, "Citation Networks");
    }

    #[test]
    fn build_update_applies_edits_and_clears_emptied() {
        let record = loaded_record();
        let mut draft = draft_with("Retitled", record.to_new().creators);
        draft.kind = ResourceKind::Book;
        draft.parent = Some(ResourceId::generate());
        draft.publisher = "Penguin".into();
        draft.published_year = "2024".into();
        draft.accessed_year = "".into();
        // Series rides through untouched text.
        draft.series = record.series_id.expect("series set").to_string();
        draft.series_number = "XL 1".into();

        let new = build_update(&record, &draft).expect("draft merges");
        assert_eq!(new.title.as_str(), "Retitled");
        assert_eq!(new.kind, ResourceKind::Book);
        assert_eq!(new.parent_id, draft.parent);
        assert_eq!(new.publisher.as_deref(), Some("Penguin"));
        assert_eq!(
            new.published
                .as_ref()
                .expect("published set")
                .instant()
                .as_unix_ms(),
            UtcMillis::jan_1_utc(2024)
                .expect("year in calendar")
                .as_unix_ms(),
        );
        assert_eq!(new.accessed, None);
        assert_eq!(new.series_id, record.series_id);
        assert_eq!(new.series_number.as_ref().map(|n| n.as_str()), Some("XL 1"));
    }

    #[test]
    fn build_update_applies_creators_from_draft() {
        let record = loaded_record();
        let previous = record.to_new().creators;
        let other = PersonId::generate();
        let mut draft = draft_with("Retitled", previous.clone());
        draft.creators = creators_from_person_ids(&[other], &previous);

        let new = build_update(&record, &draft).expect("draft merges");
        assert_eq!(new.creators.len(), 1);
        assert_eq!(new.creators[0].person_id, other);
        assert_eq!(new.creators[0].role, CreatorRole::Author);
        assert_eq!(new.creators[0].credited_as, None);
        // The helper draft carries no parent, so the merge clears the
        // record's: an unselected dropdown means no parent.
        assert_eq!(new.parent_id, None);
    }

    #[test]
    fn build_update_allows_empty_creators() {
        let record = loaded_record();
        let draft = draft_with("Retitled", Vec::new());
        let new = build_update(&record, &draft).expect("empty creators ok");
        assert!(new.creators.is_empty());
    }

    #[test]
    fn build_create_from_empty_draft_rejects_blank_title() {
        let err = build_create(&empty_create_draft()).expect_err("blank title");
        assert_eq!(err, "Title must not be empty");
    }

    #[test]
    fn build_create_makes_book_with_optional_fields() {
        let author = PersonId::generate();
        let mut draft = empty_create_draft();
        draft.title = "Unsigned dictionary".into();
        draft.publisher = "OUP".into();
        draft.published_year = "2020".into();
        draft.creators = vec![CreatorSpec {
            person_id: author,
            role: CreatorRole::Author,
            credited_as: None,
        }];

        let new = build_create(&draft).expect("create payload");
        assert_eq!(new.kind, ResourceKind::Book);
        assert_eq!(new.title.as_str(), "Unsigned dictionary");
        assert_eq!(new.publisher.as_deref(), Some("OUP"));
        assert_eq!(new.creators.len(), 1);
        assert_eq!(new.parent_id, None);
        assert_eq!(new.accessed, None);
        assert_eq!(new.series_id, None);
    }

    #[test]
    fn creators_from_person_ids_keeps_prior_credited_as() {
        let author = PersonId::generate();
        let previous = vec![CreatorSpec {
            person_id: author,
            role: CreatorRole::Editor,
            credited_as: Some(NonEmptyStr::try_new("A. Author").unwrap()),
        }];
        let merged = creators_from_person_ids(&[author], &previous);
        assert_eq!(merged[0].role, CreatorRole::Editor);
        assert_eq!(
            merged[0].credited_as.as_ref().map(|c| c.as_str()),
            Some("A. Author"),
        );
    }

    #[test]
    fn sidebar_selection_adds_and_removes_authors_without_losing_existing_credit() {
        let record = loaded_record();
        let previous = record.to_new().creators;
        let existing = previous[0].person_id;
        let added = PersonId::generate();
        let mut draft = prefill_draft(&record);
        draft.creators = creators_from_person_ids(&[existing, added], &previous);
        let updated = build_update(&record, &draft).unwrap();
        assert_eq!(updated.creators[0], previous[0]);
        assert_eq!(updated.creators[1].person_id, added);
        assert_eq!(updated.creators[1].role, CreatorRole::Author);

        draft.creators = creators_from_person_ids(&[added], &previous);
        let updated = build_update(&record, &draft).unwrap();
        assert_eq!(updated.creators.len(), 1);
        assert_eq!(updated.creators[0].person_id, added);
    }

    #[test]
    fn build_update_rejects_bad_values() {
        let record = loaded_record();
        let creators = record.to_new().creators;

        let err = build_update(&record, &draft_with("   ", creators.clone()))
            .expect_err("blank title rejects");
        assert_eq!(err, "Title must not be empty");

        let mut draft = draft_with("Retitled", creators.clone());
        draft.published_year = "soon".into();
        let err = build_update(&record, &draft).expect_err("bad year rejects");
        assert!(
            err.contains("Published year"),
            "message names the field: {err}"
        );

        let mut draft = draft_with("Retitled", creators);
        draft.series = "not-an-id".into();
        let err = build_update(&record, &draft).expect_err("bad series rejects");
        assert!(err.contains("Series"), "message names the field: {err}");
    }

    #[test]
    fn kind_items_cover_every_kind_alphabetized() {
        let items = kind_items();
        assert_eq!(items.len(), ResourceKind::ALL.len());
        let labels: Vec<String> = items
            .iter()
            .map(KindItem::title)
            .map(String::from)
            .collect();
        let mut sorted = labels.clone();
        sorted.sort_by_key(|label| label.to_lowercase());
        assert_eq!(labels, sorted);
        // Every kind appears exactly once.
        for kind in ResourceKind::ALL {
            assert_eq!(items.iter().filter(|item| item.value() == &kind).count(), 1);
        }
    }
}
