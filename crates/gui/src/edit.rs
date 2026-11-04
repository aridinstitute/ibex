//! Bibliographic edit drafts: the panel's window-free write-side twin.
//!
//! Value logic here stays window-free (draft in, payload out), so the merge
//! rules run under plain unit tests; `LibraryApp` owns the input entities
//! and wires them to the worker.

use gpui_kit::SharedString;
use gpui_kit::component::searchable_list::SearchableListItem;

use ibex_core::{
    BibliographicDate, CollectionId, CreatorRole, CreatorSpec, DateCertainty, DatePrecision,
    NewResource, NonEmptyStr, PersonId, Resource, ResourceId, ResourceKind, SeriesId, TagId,
    UtcMillis, utc_year_from_millis,
};

use crate::kind::kind_label;

use std::collections::BTreeMap;

/// Registry for generic bibliographic text inputs. Lists use one entry per line,
/// never commas. Dates accept a year, YYYY-MM, YYYY-MM-DD, or RFC3339 instant,
/// optionally a slash-separated range and a trailing `?` or `~`. Display fields
/// independently override a date's label; they require a corresponding date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BibliographicField {
    Subtitle,
    TitleAddon,
    ShortTitle,
    Place,
    Volume,
    Issue,
    Pages,
    TotalPages,
    ArticleNumber,
    Edition,
    Doi,
    Url,
    Eprint,
    Isbns,
    Issns,
    Isrns,
    OriginalDate,
    EventDate,
    Language,
    DocumentType,
    PublicationMethod,
    Institution,
    Organization,
    School,
    AbstractText,
    Keywords,
    Citekey,
    Venue,
    Published,
    Accessed,
    PublishedDisplay,
    AccessedDisplay,
    OriginalDateDisplay,
    EventDateDisplay,
}

impl BibliographicField {
    pub const ALL: &[Self] = &[
        Self::Subtitle,
        Self::TitleAddon,
        Self::ShortTitle,
        Self::Place,
        Self::Volume,
        Self::Issue,
        Self::Pages,
        Self::TotalPages,
        Self::ArticleNumber,
        Self::Edition,
        Self::Doi,
        Self::Url,
        Self::Eprint,
        Self::Isbns,
        Self::Issns,
        Self::Isrns,
        Self::OriginalDate,
        Self::EventDate,
        Self::Language,
        Self::DocumentType,
        Self::PublicationMethod,
        Self::Institution,
        Self::Organization,
        Self::School,
        Self::AbstractText,
        Self::Keywords,
        Self::Citekey,
        Self::Venue,
        Self::Published,
        Self::Accessed,
        Self::PublishedDisplay,
        Self::AccessedDisplay,
        Self::OriginalDateDisplay,
        Self::EventDateDisplay,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Subtitle => "Subtitle",
            Self::TitleAddon => "Title addon",
            Self::ShortTitle => "Short title",
            Self::Place => "Place",
            Self::Volume => "Volume",
            Self::Issue => "Issue",
            Self::Pages => "Pages",
            Self::TotalPages => "Total pages",
            Self::ArticleNumber => "Article number",
            Self::Edition => "Edition",
            Self::Doi => "DOI",
            Self::Url => "URL",
            Self::Eprint => "Eprint",
            Self::Isbns => "ISBNs",
            Self::Issns => "ISSNs",
            Self::Isrns => "ISRNs",
            Self::OriginalDate => "Original date",
            Self::EventDate => "Event date",
            Self::Language => "Language",
            Self::DocumentType => "Document type",
            Self::PublicationMethod => "Publication method",
            Self::Institution => "Institution",
            Self::Organization => "Organization",
            Self::School => "School",
            Self::AbstractText => "Abstract",
            Self::Keywords => "Keywords",
            Self::Citekey => "Citekey",
            Self::Venue => "Venue",
            Self::Published => "Published date",
            Self::Accessed => "Accessed date",
            Self::PublishedDisplay => "Published display",
            Self::AccessedDisplay => "Accessed display",
            Self::OriginalDateDisplay => "Original date display",
            Self::EventDateDisplay => "Event date display",
        }
    }

    pub fn multiline(self) -> bool {
        matches!(
            self,
            Self::AbstractText | Self::Keywords | Self::Isbns | Self::Issns | Self::Isrns
        )
    }
}

/// One value per scalar the form edits. The panel reads its input entities
/// into this plain shape at save time, so the payload build below stays
/// window-free and unit-testable. Parent and series arrive as selected ids.
/// Legacy year strings support the compact inline editor; the detail panel uses
/// full dates in `fields`. Creator specs preserve roles, credits, and order.
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
    /// Absent keys preserve stored values; present empty strings clear them.
    pub fields: BTreeMap<BibliographicField, String>,
}

/// The value each input opens with: the stored record's own values,
/// optional fields empty (or none) when the record carries nothing.
pub fn prefill_draft(record: &Resource) -> EditDraft {
    let values = record.to_new();
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
        creators: values.creators.clone(),
        tag_ids: record.tag_ids.clone(),
        collection_ids: record.collection_ids.clone(),
        fields: BibliographicField::ALL
            .iter()
            .map(|field| (*field, field_text(&values, *field)))
            .collect(),
    }
}

fn field_text(record: &NewResource, field: BibliographicField) -> String {
    match field {
        BibliographicField::Subtitle => record
            .subtitle
            .as_ref()
            .map(|value| value.as_str().to_owned())
            .unwrap_or_default(),
        BibliographicField::TitleAddon => record
            .title_addon
            .as_ref()
            .map(|value| value.as_str().to_owned())
            .unwrap_or_default(),
        BibliographicField::ShortTitle => record
            .short_title
            .as_ref()
            .map(|value| value.as_str().to_owned())
            .unwrap_or_default(),
        BibliographicField::Place => record.place.clone().unwrap_or_default(),
        BibliographicField::Volume => record.volume.clone().unwrap_or_default(),
        BibliographicField::Issue => record.issue.clone().unwrap_or_default(),
        BibliographicField::Pages => record
            .pages
            .as_ref()
            .map(|value| value.as_str().to_owned())
            .unwrap_or_default(),
        BibliographicField::TotalPages => record.total_pages.clone().unwrap_or_default(),
        BibliographicField::ArticleNumber => record.article_number.clone().unwrap_or_default(),
        BibliographicField::Edition => record.edition.clone().unwrap_or_default(),
        BibliographicField::Doi => record.doi.clone().unwrap_or_default(),
        BibliographicField::Url => record.url.clone().unwrap_or_default(),
        BibliographicField::Eprint => record.eprint.clone().unwrap_or_default(),
        BibliographicField::Language => record.language.clone().unwrap_or_default(),
        BibliographicField::DocumentType => record.document_type.clone().unwrap_or_default(),
        BibliographicField::PublicationMethod => {
            record.publication_method.clone().unwrap_or_default()
        }
        BibliographicField::Institution => record.institution.clone().unwrap_or_default(),
        BibliographicField::Organization => record.organization.clone().unwrap_or_default(),
        BibliographicField::School => record.school.clone().unwrap_or_default(),
        BibliographicField::AbstractText => record.abstract_text.clone().unwrap_or_default(),
        BibliographicField::Citekey => record.metadata.citekey.clone().unwrap_or_default(),
        BibliographicField::Venue => record.metadata.venue.clone().unwrap_or_default(),
        BibliographicField::Isbns => record
            .isbns
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        BibliographicField::Issns => record
            .issns
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        BibliographicField::Isrns => record
            .isrns
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        BibliographicField::Keywords => record.keywords.join("\n"),
        BibliographicField::Published => date_text(record.published.as_ref()),
        BibliographicField::Accessed => date_text(record.accessed.as_ref()),
        BibliographicField::OriginalDate => date_text(record.original_date.as_ref()),
        BibliographicField::EventDate => date_text(record.event_date.as_ref()),
        BibliographicField::PublishedDisplay => display_text(record.published.as_ref()),
        BibliographicField::AccessedDisplay => display_text(record.accessed.as_ref()),
        BibliographicField::OriginalDateDisplay => display_text(record.original_date.as_ref()),
        BibliographicField::EventDateDisplay => display_text(record.event_date.as_ref()),
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
        fields: BTreeMap::new(),
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

    use BibliographicField as F;
    merge_date(
        &mut edit.published,
        draft,
        F::Published,
        F::PublishedDisplay,
        Some(("Published year", &draft.published_year)),
    )?;
    merge_date(
        &mut edit.accessed,
        draft,
        F::Accessed,
        F::AccessedDisplay,
        Some(("Accessed year", &draft.accessed_year)),
    )?;
    merge_date(
        &mut edit.original_date,
        draft,
        F::OriginalDate,
        F::OriginalDateDisplay,
        None,
    )?;
    merge_date(
        &mut edit.event_date,
        draft,
        F::EventDate,
        F::EventDateDisplay,
        None,
    )?;

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

    for (field, text) in &draft.fields {
        if *text == field_text(edit, *field) {
            continue;
        }
        let text = text.trim();
        let value = (!text.is_empty()).then_some(text);
        match field {
            BibliographicField::Subtitle => {
                edit.subtitle = value
                    .map(NonEmptyStr::try_new)
                    .transpose()
                    .map_err(|e| format!("{}: {e}", field.label()))?
            }
            BibliographicField::TitleAddon => {
                edit.title_addon = value
                    .map(NonEmptyStr::try_new)
                    .transpose()
                    .map_err(|e| format!("{}: {e}", field.label()))?
            }
            BibliographicField::ShortTitle => {
                edit.short_title = value
                    .map(NonEmptyStr::try_new)
                    .transpose()
                    .map_err(|e| format!("{}: {e}", field.label()))?
            }
            BibliographicField::Place => edit.place = value.map(str::to_owned),
            BibliographicField::Volume => edit.volume = value.map(str::to_owned),
            BibliographicField::Issue => edit.issue = value.map(str::to_owned),
            BibliographicField::Pages => {
                edit.pages = value
                    .map(ibex_core::Pages::parse)
                    .transpose()
                    .map_err(|e| format!("{}: {e}", field.label()))?
            }
            BibliographicField::TotalPages => edit.total_pages = value.map(str::to_owned),
            BibliographicField::ArticleNumber => edit.article_number = value.map(str::to_owned),
            BibliographicField::Edition => edit.edition = value.map(str::to_owned),
            BibliographicField::Doi => edit.doi = value.map(str::to_owned),
            BibliographicField::Url => edit.url = value.map(str::to_owned),
            BibliographicField::Eprint => edit.eprint = value.map(str::to_owned),
            BibliographicField::Language => edit.language = value.map(str::to_owned),
            BibliographicField::DocumentType => edit.document_type = value.map(str::to_owned),
            BibliographicField::PublicationMethod => {
                edit.publication_method = value.map(str::to_owned)
            }
            BibliographicField::Institution => edit.institution = value.map(str::to_owned),
            BibliographicField::Organization => edit.organization = value.map(str::to_owned),
            BibliographicField::School => edit.school = value.map(str::to_owned),
            BibliographicField::AbstractText => edit.abstract_text = value.map(str::to_owned),
            BibliographicField::Citekey => edit.metadata.citekey = value.map(str::to_owned),
            BibliographicField::Venue => edit.metadata.venue = value.map(str::to_owned),
            BibliographicField::Isbns => edit.isbns = identifier_field(*field, text)?,
            BibliographicField::Issns => edit.issns = identifier_field(*field, text)?,
            BibliographicField::Isrns => edit.isrns = identifier_field(*field, text)?,
            BibliographicField::Keywords => edit.keywords = list_field(*field, text)?,
            BibliographicField::Published
            | BibliographicField::Accessed
            | BibliographicField::OriginalDate
            | BibliographicField::EventDate
            | BibliographicField::PublishedDisplay
            | BibliographicField::AccessedDisplay
            | BibliographicField::OriginalDateDisplay
            | BibliographicField::EventDateDisplay => {}
        }
    }
    Ok(())
}

fn list_field(field: BibliographicField, text: &str) -> Result<Vec<String>, String> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    text.lines()
        .map(|line| {
            let line = line.trim();
            if line.is_empty() || line.chars().any(char::is_control) {
                Err(format!(
                    "{} must contain one non-empty entry per line",
                    field.label()
                ))
            } else {
                Ok(line.to_owned())
            }
        })
        .collect()
}

fn identifier_field(
    field: BibliographicField,
    text: &str,
) -> Result<Vec<ibex_core::Identifier>, String> {
    Ok(list_field(field, text)?
        .into_iter()
        .map(ibex_core::Identifier::new)
        .collect())
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

fn display_text(date: Option<&BibliographicDate>) -> String {
    date.and_then(BibliographicDate::display)
        .unwrap_or_default()
        .to_owned()
}

fn date_text(date: Option<&BibliographicDate>) -> String {
    let Some(date) = date else {
        return String::new();
    };
    let mut text = instant_text(date.instant(), date.precision());
    if let Some(end) = date.end() {
        text.push('/');
        let end_text = instant_text(end, date.precision());
        // Only the start has stored precision; an end can carry finer detail.
        if parse_date_instant(&end_text).is_some_and(|(instant, _)| instant == end) {
            text.push_str(&end_text);
        } else {
            text.push_str(&instant_text(end, DatePrecision::DateTime));
        }
    }
    match date.certainty() {
        DateCertainty::Uncertain => text.push('?'),
        DateCertainty::Approximate => text.push('~'),
        DateCertainty::Exact => {}
    }
    text
}

fn instant_text(instant: UtcMillis, precision: DatePrecision) -> String {
    let Ok(at) = time::OffsetDateTime::from_unix_timestamp_nanos(
        i128::from(instant.as_unix_ms()) * 1_000_000,
    ) else {
        return instant.as_unix_ms().to_string();
    };
    match precision {
        DatePrecision::Year => at.year().to_string(),
        DatePrecision::Month => format!("{:04}-{:02}", at.year(), u8::from(at.month())),
        DatePrecision::Day => format!(
            "{:04}-{:02}-{:02}",
            at.year(),
            u8::from(at.month()),
            at.day()
        ),
        DatePrecision::DateTime | DatePrecision::Unspecified => at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| at.to_string()),
    }
}

// Compare against the original date before applying either representation. An
// unchanged full-date input must not undo an inline year edit, or infer precision.
fn merge_date(
    date: &mut Option<BibliographicDate>,
    draft: &EditDraft,
    field: BibliographicField,
    display_field: BibliographicField,
    legacy: Option<(&str, &str)>,
) -> Result<(), String> {
    let old_display = display_text(date.as_ref());
    let edited_full = draft
        .fields
        .get(&field)
        .filter(|text| text.trim() != date_text(date.as_ref()));
    let mut changed = if let Some(text) = edited_full {
        parse_date(field, text)?
    } else if let Some((label, year)) =
        legacy.filter(|(_, year)| year.trim() != date_year(date.as_ref()))
    {
        year_field(label, year)?
    } else {
        date.clone()
    };
    let label = draft
        .fields
        .get(&display_field)
        .filter(|text| text.trim() != old_display)
        .map(|text| text.trim())
        .unwrap_or(&old_display);
    if let Some(value) = changed.as_ref() {
        changed = Some(
            BibliographicDate::try_new_precise(
                value.instant(),
                value.end(),
                value.precision(),
                value.certainty(),
                (!label.is_empty())
                    .then(|| NonEmptyStr::try_new(label))
                    .transpose()
                    .map_err(|e| format!("{}: {e}", display_field.label()))?,
            )
            .map_err(|e| format!("{}: {e}", field.label()))?,
        );
    } else if !label.is_empty() && label != old_display {
        return Err(format!("{} requires a date", display_field.label()));
    }
    *date = changed;
    Ok(())
}

fn parse_date(field: BibliographicField, text: &str) -> Result<Option<BibliographicDate>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let (text, certainty) = if let Some(text) = text.strip_suffix('?') {
        (text, DateCertainty::Uncertain)
    } else if let Some(text) = text.strip_suffix('~') {
        (text, DateCertainty::Approximate)
    } else {
        (text, DateCertainty::Exact)
    };
    let error = || {
        format!(
            "{} must be a year, YYYY-MM, YYYY-MM-DD or RFC3339 date, optionally a range with / and suffix ? or ~",
            field.label()
        )
    };
    let (start, end) = text
        .split_once('/')
        .map_or((text, None), |(start, end)| (start, Some(end)));
    let (instant, precision) = parse_date_instant(start).ok_or_else(error)?;
    let end = end
        .map(|end| {
            parse_date_instant(end)
                .map(|(instant, _)| instant)
                .ok_or_else(error)
        })
        .transpose()?;
    if end.is_some_and(|end| end.as_unix_ms() < instant.as_unix_ms()) {
        return Err(format!(
            "{} range end must not precede its start",
            field.label()
        ));
    }
    BibliographicDate::try_new_precise(instant, end, precision, certainty, None)
        .map(Some)
        .map_err(|e| format!("{}: {e}", field.label()))
}

fn parse_date_instant(text: &str) -> Option<(UtcMillis, DatePrecision)> {
    if text.contains(['T', 't']) {
        let at = time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339)
            .ok()?;
        return Some((
            UtcMillis::from_unix_ms(i64::try_from(at.unix_timestamp_nanos() / 1_000_000).ok()?),
            DatePrecision::DateTime,
        ));
    }
    let (sign, unsigned) = text.strip_prefix('-').map_or((1, text), |text| (-1, text));
    let parts = unsigned.split('-').collect::<Vec<_>>();
    if parts.is_empty()
        || parts.len() > 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let year = parts[0].parse::<i32>().ok()?.checked_mul(sign)?;
    let month = parts
        .get(1)
        .map(|part| {
            if part.len() == 2 {
                part.parse::<u8>().ok()
            } else {
                None
            }
        })
        .unwrap_or(Some(1))?;
    let day = parts
        .get(2)
        .map(|part| {
            if part.len() == 2 {
                part.parse::<u8>().ok()
            } else {
                None
            }
        })
        .unwrap_or(Some(1))?;
    let at = time::Date::from_calendar_date(year, time::Month::try_from(month).ok()?, day)
        .ok()?
        .midnight()
        .assume_utc();
    let precision = match parts.len() {
        1 => DatePrecision::Year,
        2 => DatePrecision::Month,
        _ => DatePrecision::Day,
    };
    Some((
        UtcMillis::from_unix_ms(at.unix_timestamp() * 1000),
        precision,
    ))
}
/// The stored date's UTC year as text; empty when no date is set. The form
/// Legacy inputs edit whole years; display labels never enter these inputs.
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
    BibliographicDate::try_new_precise(
        instant,
        None,
        DatePrecision::Year,
        DateCertainty::Exact,
        None,
    )
    .map(Some)
    .map_err(|_| format!("{label} is outside the calendar"))
}

#[cfg(test)]
mod tests {
    use super::{
        EditDraft, KindItem, SearchableListItem, build_create, build_update,
        creators_from_person_ids, empty_create_draft, kind_items, prefill_draft,
    };
    use ibex_core::{
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
            fields: Default::default(),
        }
    }
    #[test]
    fn range_end_keeps_finer_detail_when_start_is_edited() {
        use super::BibliographicField as F;
        let mut draft = empty_create_draft();
        draft.title = "Range".into();
        draft
            .fields
            .insert(F::OriginalDate, "2024/2025-03-04T12:00:00Z".into());
        let record =
            Resource::try_new(ResourceId::generate(), build_create(&draft).unwrap()).unwrap();
        let mut draft = prefill_draft(&record);
        assert_eq!(draft.fields[&F::OriginalDate], "2024/2025-03-04T12:00:00Z");
        draft.fields.insert(
            F::OriginalDate,
            draft.fields[&F::OriginalDate].replacen("2024", "2023", 1),
        );
        let updated = build_update(&record, &draft).unwrap();
        let date = updated.original_date.unwrap();
        assert_eq!(date.instant(), UtcMillis::jan_1_utc(2023).unwrap());
        assert_eq!(date.end(), record.original_date.unwrap().end());
    }
    #[test]
    fn registry_populates_every_input_and_marks_multiline_values() {
        use super::BibliographicField as F;
        let mut record = loaded_record();
        record.published = None;
        record.accessed = None;
        let draft = prefill_draft(&record);
        assert_eq!(F::ALL.len(), 34);
        assert_eq!(draft.fields.len(), F::ALL.len());
        for field in F::ALL {
            assert_eq!(draft.fields[field], "");
            assert!(!field.label().is_empty());
            assert_eq!(
                field.multiline(),
                [F::AbstractText, F::Keywords, F::Isbns, F::Issns, F::Isrns].contains(field)
            );
        }
        assert!(empty_create_draft().fields.is_empty());
    }
    #[test]
    fn absent_fields_and_unchanged_structured_values_preserve_metadata() {
        use super::BibliographicField as F;
        let mut record = loaded_record();
        record.subtitle = Some(NonEmptyStr::try_new("Unexposed subtitle").unwrap());
        record.isbns = vec![ibex_core::Identifier {
            original: "Source, spelling".into(),
            canonical: Some("custom-key".into()),
        }];
        record.pages = Some(ibex_core::Pages {
            literal: NonEmptyStr::try_new("1, 3").unwrap(),
            spans: vec![],
        });
        record.keywords = vec!["first, keyword".into(), "second".into()];
        record.metadata.citekey = Some("old-key".into());
        record.metadata.venue = Some("old-venue".into());
        record
            .metadata
            .source_fields
            .insert("title".into(), "Source title".into());
        record.metadata.provenance = ibex_core::ResourceProvenance {
            format: Some("biblatex".into()),
            parser_version: Some(7),
            entry_type: Some("article".into()),
            original_fields: [("title".into(), "Original title".into())].into(),
            inherited_origins: [("publisher".into(), "parent-key".into())].into(),
            note: Some("Source note".into()),
            annotation: Some("Source annotation".into()),
            source_fingerprint: Some("fingerprint".into()),
        };
        let mut draft = prefill_draft(&record);
        assert_eq!(build_update(&record, &draft).unwrap(), record.to_new());
        draft.fields.clear();
        assert_eq!(build_update(&record, &draft).unwrap(), record.to_new());
        draft.fields.insert(F::Citekey, "new-key".into());
        draft.fields.insert(F::Venue, "new-venue".into());
        let mut expected = record.to_new();
        expected.metadata.citekey = Some("new-key".into());
        expected.metadata.venue = Some("new-venue".into());
        assert_eq!(build_update(&record, &draft).unwrap(), expected);
    }
    #[test]
    fn invalid_dates_and_orphan_displays_name_the_field() {
        use super::BibliographicField as F;
        let record = loaded_record();
        for field in [F::Published, F::Accessed, F::OriginalDate, F::EventDate] {
            for text in [
                "soon",
                "2023-02-29",
                "2024-13",
                "2024-01-32",
                "2024-1",
                "2024/2023",
                "2024/",
                "/2024",
                "2024/2025/2026",
                "2024?~",
                "2024-01-01T25:00:00Z",
                "2024-01-01T00:00:00",
                "999999999999999999999",
            ] {
                let mut draft = prefill_draft(&record);
                draft.fields.insert(field, text.into());
                let err = build_update(&record, &draft).unwrap_err();
                assert!(err.contains(field.label()), "{field:?}: {text}: {err}");
                assert!(build_create(&draft).is_err());
            }
        }
        for field in [
            F::PublishedDisplay,
            F::AccessedDisplay,
            F::OriginalDateDisplay,
            F::EventDateDisplay,
        ] {
            let mut draft = empty_create_draft();
            draft.title = "No date".into();
            draft.fields.insert(field, "Orphan display".into());
            assert!(build_create(&draft).unwrap_err().contains(field.label()));
        }
    }
    #[test]
    fn edited_full_dates_win_but_unchanged_full_dates_allow_year_edits() {
        use super::BibliographicField as F;
        use ibex_core::DatePrecision as P;
        let mut record = loaded_record();
        record.published = Some(
            BibliographicDate::try_new_precise(
                UtcMillis::from_unix_ms(1709201096123),
                None,
                P::DateTime,
                ibex_core::DateCertainty::Exact,
                None,
            )
            .unwrap(),
        );
        record.accessed = record.published.clone();
        for (field, year_is_published) in [(F::Published, true), (F::Accessed, false)] {
            let mut draft = prefill_draft(&record);
            if year_is_published {
                draft.published_year = "2025".into();
            } else {
                draft.accessed_year = "2025".into();
            }
            let updated = build_update(&record, &draft).unwrap();
            let date = if year_is_published {
                updated.published
            } else {
                updated.accessed
            }
            .unwrap();
            assert_eq!(date.instant(), UtcMillis::jan_1_utc(2025).unwrap());
            assert_eq!(date.precision(), P::Year);
            draft.fields.insert(field, "2026-03-04".into());
            // Even an invalid legacy input is irrelevant when the full date is edited.
            if year_is_published {
                draft.published_year = "invalid".into();
            } else {
                draft.accessed_year = "invalid".into();
            }
            let updated = build_update(&record, &draft).unwrap();
            let date = if year_is_published {
                updated.published
            } else {
                updated.accessed
            }
            .unwrap();
            assert_eq!(date.instant().as_unix_ms(), 1772582400000);
            assert_eq!(date.precision(), P::Day);
            draft.fields.insert(field, String::new());
            let updated = build_update(&record, &draft).unwrap();
            assert!(
                if year_is_published {
                    updated.published
                } else {
                    updated.accessed
                }
                .is_none()
            );
        }
    }
    #[test]
    fn full_dates_edit_prefill_display_and_clear() {
        use super::BibliographicField as F;
        use ibex_core::{DateCertainty as C, DatePrecision as P};
        let pairs = [
            (F::Published, F::PublishedDisplay, "published"),
            (F::Accessed, F::AccessedDisplay, "accessed"),
            (F::OriginalDate, F::OriginalDateDisplay, "original_date"),
            (F::EventDate, F::EventDateDisplay, "event_date"),
        ];
        for (field, display, key) in pairs {
            for (text, precision, certainty, instant, end) in [
                ("2024", P::Year, C::Exact, 1704067200000, None),
                ("2024-02?", P::Month, C::Uncertain, 1706745600000, None),
                ("2024-02-29~", P::Day, C::Approximate, 1709164800000, None),
                (
                    "2024-02/2024-03~",
                    P::Month,
                    C::Approximate,
                    1706745600000,
                    Some(1709251200000),
                ),
                (
                    "2024-02-29T12:34:56.123+02:30",
                    P::DateTime,
                    C::Exact,
                    1709201096123,
                    None,
                ),
            ] {
                let mut draft = empty_create_draft();
                draft.title = "Dates".into();
                draft.fields.insert(field, text.into());
                draft.fields.insert(display, "Custom label".into());
                let new = build_create(&draft).unwrap();
                let date = match field {
                    F::Published => &new.published,
                    F::Accessed => &new.accessed,
                    F::OriginalDate => &new.original_date,
                    F::EventDate => &new.event_date,
                    _ => unreachable!(),
                }
                .as_ref()
                .unwrap();
                assert_eq!(date.instant().as_unix_ms(), instant, "{text}");
                assert_eq!(date.end().map(|v| v.as_unix_ms()), end);
                assert_eq!(date.precision(), precision);
                assert_eq!(date.certainty(), certainty);
                assert_eq!(date.display(), Some("Custom label"));
                let original = loaded_record();
                let mut update_draft = prefill_draft(&original);
                update_draft.fields.insert(field, text.into());
                update_draft.fields.insert(display, "Custom label".into());
                let updated =
                    serde_json::to_value(build_update(&original, &update_draft).unwrap()).unwrap();
                assert_eq!(updated[key], serde_json::to_value(&new).unwrap()[key]);
                let record = Resource::try_new(ResourceId::generate(), new.clone()).unwrap();
                let mut draft = prefill_draft(&record);
                assert!(!draft.fields[&field].is_empty());
                assert_eq!(draft.fields[&display], "Custom label");
                assert_eq!(build_update(&record, &draft).unwrap(), new);
                draft.fields.insert(display, String::new());
                let updated = serde_json::to_value(build_update(&record, &draft).unwrap()).unwrap();
                assert!(updated[key]["display"].is_null());
                draft.fields.insert(field, String::new());
                assert!(
                    serde_json::to_value(build_update(&record, &draft).unwrap()).unwrap()[key]
                        .is_null()
                );
            }
        }
    }
    #[test]
    fn invalid_lists_are_rejected_with_field_labels() {
        use super::BibliographicField as F;
        let record = loaded_record();
        for field in [F::Isbns, F::Issns, F::Isrns, F::Keywords] {
            for text in ["first\n\nsecond", "first\n  \nsecond", "bad\0entry"] {
                let mut draft = prefill_draft(&record);
                draft.fields.insert(field, text.into());
                let err = build_update(&record, &draft).unwrap_err();
                assert!(err.contains(field.label()), "{err}");
            }
        }
    }
    #[test]
    fn list_fields_use_lines_preserve_commas_and_clear() {
        use super::BibliographicField as F;
        for (field, key) in [
            (F::Isbns, "isbns"),
            (F::Issns, "issns"),
            (F::Isrns, "isrns"),
            (F::Keywords, "keywords"),
        ] {
            let mut draft = empty_create_draft();
            draft.title = "Lists".into();
            assert!(draft.fields.is_empty());
            draft
                .fields
                .insert(field, "  first, literal\n second \n".into());
            let new = build_create(&draft).unwrap();
            let json = serde_json::to_value(&new).unwrap();
            if field == F::Keywords {
                assert_eq!(json[key], serde_json::json!(["first, literal", "second"]));
            } else {
                assert_eq!(json[key][0]["original"], "first, literal");
                assert_eq!(json[key][1]["original"], "second");
                assert_eq!(json[key].as_array().unwrap().len(), 2);
            }
            let record = Resource::try_new(ResourceId::generate(), new).unwrap();
            let mut draft = prefill_draft(&record);
            assert_eq!(draft.fields[&field], "first, literal\nsecond");
            draft.fields.insert(field, String::new());
            let cleared = serde_json::to_value(build_update(&record, &draft).unwrap()).unwrap();
            assert_eq!(cleared[key], serde_json::json!([]));
        }
    }

    #[test]
    fn scalar_fields_edit_prefill_and_clear() {
        use super::BibliographicField as F;
        let cases = [
            (F::Subtitle, "subtitle"),
            (F::TitleAddon, "title_addon"),
            (F::ShortTitle, "short_title"),
            (F::Place, "place"),
            (F::Volume, "volume"),
            (F::Issue, "issue"),
            (F::Pages, "pages"),
            (F::TotalPages, "total_pages"),
            (F::ArticleNumber, "article_number"),
            (F::Edition, "edition"),
            (F::Doi, "doi"),
            (F::Url, "url"),
            (F::Eprint, "eprint"),
            (F::Language, "language"),
            (F::DocumentType, "document_type"),
            (F::PublicationMethod, "publication_method"),
            (F::Institution, "institution"),
            (F::Organization, "organization"),
            (F::School, "school"),
            (F::AbstractText, "abstract_text"),
            (F::Citekey, "citekey"),
            (F::Venue, "venue"),
        ];
        let record = loaded_record();
        for (field, key) in cases {
            let mut draft = prefill_draft(&record);
            draft.fields.insert(field, "  edited, value  ".into());
            let new = build_update(&record, &draft).unwrap();
            let json = serde_json::to_value(&new).unwrap();
            let value = if matches!(field, F::Citekey | F::Venue) {
                &json["metadata"][key]
            } else {
                &json[key]
            };
            let value = if field == F::Pages {
                &value["literal"]
            } else {
                value
            };
            assert_eq!(value, "edited, value", "{field:?}");
            let created = serde_json::to_value(build_create(&draft).unwrap()).unwrap();
            let created_value = if matches!(field, F::Citekey | F::Venue) {
                &created["metadata"][key]
            } else {
                &created[key]
            };
            let updated_value = if matches!(field, F::Citekey | F::Venue) {
                &json["metadata"][key]
            } else {
                &json[key]
            };
            assert_eq!(
                created_value, updated_value,
                "{field:?} create matches update"
            );
            let stored = Resource::try_new(record.id, new).unwrap();
            let mut draft = prefill_draft(&stored);
            assert_eq!(draft.fields[&field], "edited, value");
            draft.fields.insert(field, "  ".into());
            let cleared = serde_json::to_value(build_update(&stored, &draft).unwrap()).unwrap();
            let value = if matches!(field, F::Citekey | F::Venue) {
                &cleared["metadata"][key]
            } else {
                &cleared[key]
            };
            assert!(value.is_null(), "{field:?}");
        }
    }
    #[test]
    fn unchanged_legacy_year_preserves_exact_date_data() {
        use ibex_core::DatePrecision as P;
        for precision in [P::Unspecified, P::Year, P::Month, P::Day, P::DateTime] {
            let mut record = loaded_record();
            record.published = Some(
                BibliographicDate::try_new_precise(
                    UtcMillis::from_unix_ms(1_600_000_000_123),
                    Some(UtcMillis::from_unix_ms(1_600_086_400_456)),
                    precision,
                    ibex_core::DateCertainty::Uncertain,
                    Some(NonEmptyStr::try_new("Autumn 2020").unwrap()),
                )
                .unwrap(),
            );
            record.accessed = record.published.clone();
            record.original_date = record.published.clone();
            record.event_date = record.published.clone();
            let mut draft = prefill_draft(&record);
            assert_eq!(build_update(&record, &draft).unwrap(), record.to_new());
            draft.fields.clear();
            assert_eq!(build_update(&record, &draft).unwrap(), record.to_new());
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
