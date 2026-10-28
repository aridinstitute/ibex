//! CSL-JSON (citeproc item) parsing into format-neutral drafts.
//!
//! No [`oeuvre_core::Store`] or [`crate::AppState`] dependency — person matching
//! and persistence stay in shared orchestration.

use std::collections::BTreeMap;

use oeuvre_core::{
    BibliographicDate, CreatorRole, DateCertainty, DatePrecision, DiagnosticSeverity,
    EntryProvenance, IMPORT_MAX_SOURCE_BYTES, IMPORT_PARSER_VERSION, Identifier, ImportDiagnostic,
    ImportDraft, ImportEntryDraft, ImportFormat, ImportTempId, Name, NewResource, NonEmptyStr,
    Pages, ResourceKind, ResourceMetadata, ResourceProvenance, SourceProvenance, UnresolvedCreator,
    UtcMillis, import_codes,
};
use serde::Deserialize;
use serde_json::Value;

use super::biblatex::ParseError;

/// Parse CSL-JSON source (array or single item) into a format-neutral draft.
pub fn parse_csl_json(source: &str) -> Result<ImportDraft, ParseError> {
    if source.len() > IMPORT_MAX_SOURCE_BYTES {
        return Err(ParseError::new(
            import_codes::SOURCE_TOO_LARGE,
            format!(
                "Bibliography exceeds the {} MiB import limit",
                IMPORT_MAX_SOURCE_BYTES / (1024 * 1024)
            ),
        ));
    }
    let source = source.trim_start_matches('\u{feff}').trim();
    let value: Value = serde_json::from_str(source)
        .map_err(|e| ParseError::new(import_codes::SYNTAX, format!("CSL-JSON syntax: {e}")))?;
    let items = match value {
        Value::Array(items) => items,
        Value::Object(_) => vec![value],
        other => {
            return Err(ParseError::new(
                import_codes::SYNTAX,
                format!("CSL-JSON root must be an object or array, got {other}"),
            ));
        }
    };

    let mut draft = ImportDraft {
        provenance: SourceProvenance {
            format: ImportFormat::CslJson,
            parser_version: IMPORT_PARSER_VERSION,
        },
        source_fingerprint: Some(oeuvre_core::source_fingerprint(
            ImportFormat::CslJson,
            source,
        )),
        entries: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut next_temp = 1u64;
    let mut seen_keys = BTreeMap::<String, usize>::new();

    for (index, item_value) in items.into_iter().enumerate() {
        let item: CslItem = match serde_json::from_value(item_value.clone()) {
            Ok(item) => item,
            Err(e) => {
                draft.diagnostics.push(ImportDiagnostic {
                    code: import_codes::SYNTAX.to_owned(),
                    severity: DiagnosticSeverity::Error,
                    entry_key: Some(format!("item[{index}]")),
                    field: None,
                    location: Some(format!("index {index}")),
                    message: format!("invalid CSL-JSON item: {e}"),
                });
                continue;
            }
        };
        let source_key = item
            .id
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("item-{index}"));
        if let Some(prev) = seen_keys.insert(source_key.clone(), index) {
            draft.diagnostics.push(
                ImportDiagnostic::warning(
                    import_codes::DUPLICATE_KEY,
                    format!("duplicate CSL id `{source_key}` (also at index {prev})"),
                )
                .with_entry(source_key.clone()),
            );
        }
        let original_fields = flatten_original_fields(&item_value);
        let note = item
            .note
            .clone()
            .or_else(|| original_fields.get("note").cloned());
        match map_item(&item, &source_key, &original_fields) {
            Ok(mapped) => {
                for (field, message) in mapped.field_warnings {
                    draft.diagnostics.push(
                        ImportDiagnostic::warning(import_codes::INVALID_FIELD, message)
                            .with_entry(source_key.clone())
                            .with_field(field),
                    );
                }
                let mut resource = mapped.resource;
                resource.metadata.provenance = ResourceProvenance {
                    format: Some("csl-json".into()),
                    parser_version: Some(IMPORT_PARSER_VERSION),
                    entry_type: item.type_.clone(),
                    original_fields: original_fields.clone(),
                    inherited_origins: BTreeMap::new(),
                    note: note.clone(),
                    annotation: None,
                    source_fingerprint: None,
                };
                draft.entries.push(ImportEntryDraft {
                    temp_id: ImportTempId::new(next_temp),
                    resource,
                    unresolved_creators: mapped.creators,
                    series_title: item
                        .collection_title
                        .clone()
                        .filter(|s| !s.trim().is_empty()),
                    series_number: item
                        .collection_number
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .and_then(|s| NonEmptyStr::try_new(s).ok()),
                    provenance: EntryProvenance {
                        source_key: source_key.clone(),
                        entry_type: item.type_.clone(),
                        original_fields,
                        expanded_fields: BTreeMap::new(),
                        inherited_origins: BTreeMap::new(),
                        note,
                        annotation: None,
                    },
                    had_reference_fields: false,
                    publication_ref_keys: vec![],
                    had_xdata: false,
                });
                next_temp += 1;
            }
            Err(MapError {
                code,
                field,
                message,
            }) => {
                draft.diagnostics.push(ImportDiagnostic {
                    code: code.to_owned(),
                    severity: DiagnosticSeverity::Error,
                    entry_key: Some(source_key),
                    field,
                    location: Some(format!("index {index}")),
                    message,
                });
            }
        }
    }
    Ok(draft)
}

#[derive(Debug, Deserialize)]
struct CslItem {
    id: Option<String>,
    #[serde(rename = "type")]
    type_: Option<String>,
    title: Option<String>,
    #[serde(default)]
    author: Vec<CslName>,
    #[serde(default)]
    editor: Vec<CslName>,
    #[serde(default)]
    translator: Vec<CslName>,
    publisher: Option<String>,
    #[serde(rename = "publisher-place")]
    publisher_place: Option<String>,
    issued: Option<CslDate>,
    accessed: Option<CslDate>,
    #[serde(rename = "DOI")]
    doi: Option<String>,
    #[serde(rename = "ISBN")]
    isbn: Option<CslStringOrList>,
    #[serde(rename = "ISSN")]
    issn: Option<CslStringOrList>,
    #[serde(rename = "URL")]
    url: Option<String>,
    #[serde(rename = "container-title")]
    container_title: Option<String>,
    volume: Option<String>,
    issue: Option<CslStringOrNumber>,
    page: Option<String>,
    #[serde(rename = "number-of-pages")]
    number_of_pages: Option<CslStringOrNumber>,
    edition: Option<CslStringOrNumber>,
    language: Option<String>,
    abstract_: Option<String>,
    #[serde(rename = "abstract")]
    abstract_field: Option<String>,
    #[serde(default)]
    keyword: Option<CslStringOrList>,
    note: Option<String>,
    #[serde(rename = "collection-title")]
    collection_title: Option<String>,
    #[serde(rename = "collection-number")]
    collection_number: Option<String>,
    #[serde(rename = "event-date")]
    event_date: Option<CslDate>,
    #[serde(rename = "original-date")]
    original_date: Option<CslDate>,
    genre: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CslName {
    family: Option<String>,
    given: Option<String>,
    #[serde(rename = "non-dropping-particle")]
    non_dropping_particle: Option<String>,
    #[serde(rename = "dropping-particle")]
    dropping_particle: Option<String>,
    suffix: Option<String>,
    literal: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CslDate {
    #[serde(rename = "date-parts")]
    date_parts: Option<Vec<Vec<i32>>>,
    literal: Option<String>,
    raw: Option<String>,
    circa: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CslStringOrList {
    One(String),
    Many(Vec<String>),
}

impl CslStringOrList {
    fn values(&self) -> Vec<String> {
        match self {
            Self::One(s) => split_identifier_list(s),
            Self::Many(v) => v.iter().flat_map(|s| split_identifier_list(s)).collect(),
        }
    }

    fn keywords(&self) -> Vec<String> {
        match self {
            Self::One(s) => s
                .split([',', ';'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
            Self::Many(v) => v
                .iter()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CslStringOrNumber {
    String(String),
    Number(i64),
    Float(f64),
}

impl CslStringOrNumber {
    fn as_string(&self) -> String {
        match self {
            Self::String(s) => s.clone(),
            Self::Number(n) => n.to_string(),
            Self::Float(n) => {
                if n.fract() == 0.0 {
                    (*n as i64).to_string()
                } else {
                    n.to_string()
                }
            }
        }
    }
}

struct MappedEntry {
    resource: NewResource,
    creators: Vec<UnresolvedCreator>,
    field_warnings: Vec<(String, String)>,
}

struct MapError {
    code: &'static str,
    field: Option<String>,
    message: String,
}

fn map_err(code: &'static str, message: impl Into<String>) -> MapError {
    MapError {
        code,
        field: None,
        message: message.into(),
    }
}

fn map_field_err(code: &'static str, field: &str, message: impl Into<String>) -> MapError {
    MapError {
        code,
        field: Some(field.to_owned()),
        message: message.into(),
    }
}

fn map_item(
    item: &CslItem,
    source_key: &str,
    original_fields: &BTreeMap<String, String>,
) -> Result<MappedEntry, MapError> {
    let kind = map_type(item.type_.as_deref())?;
    let title = item
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| map_err(import_codes::MISSING_TITLE, "entry needs a title"))?;
    let mut field_warnings = Vec::new();
    let mut creators = Vec::new();
    for (names, role) in [
        (&item.author, CreatorRole::Author),
        (&item.editor, CreatorRole::Editor),
        (&item.translator, CreatorRole::Translator),
    ] {
        for name in names {
            match map_name(name) {
                Ok(Some(n)) => creators.push(UnresolvedCreator { role, name: n }),
                Ok(None) => {}
                Err(message) => {
                    field_warnings.push((role_field(role).into(), message));
                }
            }
        }
    }
    let pages = match item
        .page
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(raw) => match Pages::parse(raw) {
            Ok(pages) => Some(pages),
            Err(e) => {
                field_warnings.push(("page".into(), format!("page: {e}")));
                None
            }
        },
        None => None,
    };
    let issued = optional_date(item.issued.as_ref(), "issued", &mut field_warnings);
    let accessed = optional_date(item.accessed.as_ref(), "accessed", &mut field_warnings);
    let original_date = optional_date(
        item.original_date.as_ref(),
        "original-date",
        &mut field_warnings,
    );
    let event_date = optional_date(item.event_date.as_ref(), "event-date", &mut field_warnings);
    let abstract_text = item
        .abstract_field
        .clone()
        .or_else(|| item.abstract_.clone());
    let keywords = item
        .keyword
        .as_ref()
        .map(CslStringOrList::keywords)
        .unwrap_or_default();
    let issue = item.issue.as_ref().map(CslStringOrNumber::as_string);
    let edition = item.edition.as_ref().map(CslStringOrNumber::as_string);
    let total_pages = item
        .number_of_pages
        .as_ref()
        .map(CslStringOrNumber::as_string);
    let resource = NewResource {
        kind,
        title: NonEmptyStr::try_new(title)
            .map_err(|e| map_err(import_codes::INVALID_FIELD, e.to_string()))?,
        creators: vec![],
        publisher: item.publisher.clone().filter(|s| !s.trim().is_empty()),
        published: issued,
        accessed,
        parent_id: None,
        series_id: None,
        series_number: None,
        tag_ids: vec![],
        collection_ids: vec![],
        metadata: ResourceMetadata {
            citekey: Some(source_key.to_owned()),
            venue: item
                .container_title
                .clone()
                .filter(|s| !s.trim().is_empty()),
            source_fields: original_fields.clone(),
            provenance: ResourceProvenance::default(),
        },
        subtitle: None,
        title_addon: None,
        short_title: None,
        place: item
            .publisher_place
            .clone()
            .filter(|s| !s.trim().is_empty()),
        volume: item.volume.clone().filter(|s| !s.trim().is_empty()),
        issue: issue.filter(|s| !s.trim().is_empty()),
        pages,
        total_pages: total_pages.filter(|s| !s.trim().is_empty()),
        article_number: None,
        edition: edition.filter(|s| !s.trim().is_empty()),
        doi: item.doi.clone().filter(|s| !s.trim().is_empty()),
        url: item.url.clone().filter(|s| !s.trim().is_empty()),
        eprint: None,
        isbns: item
            .isbn
            .as_ref()
            .map(CslStringOrList::values)
            .unwrap_or_default()
            .into_iter()
            .map(Identifier::new)
            .collect(),
        issns: item
            .issn
            .as_ref()
            .map(CslStringOrList::values)
            .unwrap_or_default()
            .into_iter()
            .map(Identifier::new)
            .collect(),
        isrns: vec![],
        original_date,
        event_date,
        language: item.language.clone().filter(|s| !s.trim().is_empty()),
        document_type: item.genre.clone().filter(|s| !s.trim().is_empty()),
        publication_method: None,
        institution: None,
        organization: None,
        school: None,
        abstract_text: abstract_text.filter(|s| !s.trim().is_empty()),
        keywords,
    };
    Ok(MappedEntry {
        resource,
        creators,
        field_warnings,
    })
}

fn role_field(role: CreatorRole) -> &'static str {
    match role {
        CreatorRole::Author => "author",
        CreatorRole::Editor => "editor",
        CreatorRole::Translator => "translator",
        _ => "creator",
    }
}

fn map_type(type_: Option<&str>) -> Result<ResourceKind, MapError> {
    let Some(type_) = type_.map(str::trim).filter(|s| !s.is_empty()) else {
        return Err(map_err(
            import_codes::UNSUPPORTED_TYPE,
            "CSL item missing type",
        ));
    };
    let kind = match type_ {
        "book" | "monograph" => ResourceKind::Book,
        "article-journal" | "article" => ResourceKind::JournalArticle,
        "article-magazine" => ResourceKind::MagazineArticle,
        "article-newspaper" => ResourceKind::NewspaperArticle,
        "chapter" => ResourceKind::BookSection,
        "entry-encyclopedia" => ResourceKind::EncyclopediaArticle,
        "entry-dictionary" => ResourceKind::DictionaryEntry,
        "paper-conference" => ResourceKind::ConferencePaper,
        "thesis" => ResourceKind::Thesis,
        "report" => ResourceKind::Report,
        "webpage" => ResourceKind::Webpage,
        "post-weblog" => ResourceKind::BlogPost,
        "dataset" => ResourceKind::Dataset,
        "software" | "computer-program" => ResourceKind::ComputerProgram,
        "manuscript" | "personal_communication" => ResourceKind::Manuscript,
        "song" | "broadcast" => ResourceKind::PodcastEpisode,
        "periodical" => ResourceKind::Journal,
        other => {
            return Err(map_err(
                import_codes::UNSUPPORTED_TYPE,
                format!("unsupported CSL type: {other}"),
            ));
        }
    };
    Ok(kind)
}

fn map_name(name: &CslName) -> Result<Option<Name>, String> {
    if let Some(literal) = name
        .literal
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Name::corporate(literal)
            .map(Some)
            .map_err(|e| e.to_string());
    }
    let particle = name
        .non_dropping_particle
        .as_deref()
        .or(name.dropping_particle.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let family = name
        .family
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let given = name
        .given
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let suffix = name
        .suffix
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if family.is_none() && given.is_none() && particle.is_none() && suffix.is_none() {
        return Ok(None);
    }
    Name::try_new_parts(given, particle, family, suffix)
        .map(Some)
        .map_err(|e| e.to_string())
}

fn optional_date(
    date: Option<&CslDate>,
    field: &str,
    warnings: &mut Vec<(String, String)>,
) -> Option<BibliographicDate> {
    match date.and_then(|d| csl_date(d, field).transpose()) {
        Some(Ok(v)) => Some(v),
        Some(Err(err)) => {
            warnings.push((field.to_owned(), err.message));
            None
        }
        None => None,
    }
}

fn csl_date(date: &CslDate, field: &str) -> Result<Option<BibliographicDate>, MapError> {
    let certainty = if date.circa == Some(true) {
        DateCertainty::Approximate
    } else {
        DateCertainty::Exact
    };
    if let Some(parts) = date.date_parts.as_ref().and_then(|p| p.first()) {
        if parts.is_empty() {
            return Ok(None);
        }
        let year = parts[0];
        let month = parts.get(1).copied();
        let day = parts.get(2).copied();
        let (instant, precision) = parts_to_millis(year, month, day, field)?;
        let end = date
            .date_parts
            .as_ref()
            .and_then(|p| p.get(1))
            .map(|end_parts| {
                let y = *end_parts.first().unwrap_or(&year);
                let m = end_parts.get(1).copied();
                let d = end_parts.get(2).copied();
                parts_to_millis(y, m, d, field).map(|(ms, _)| ms)
            })
            .transpose()?;
        let display = date
            .literal
            .clone()
            .or_else(|| date.raw.clone())
            .or_else(|| Some(format_parts(year, month, day)));
        return BibliographicDate::try_new_precise(
            instant,
            end,
            precision,
            certainty,
            display
                .as_deref()
                .map(NonEmptyStr::try_new)
                .transpose()
                .map_err(|e| map_field_err(import_codes::INVALID_FIELD, field, e.to_string()))?,
        )
        .map(Some)
        .map_err(|e| map_field_err(import_codes::INVALID_FIELD, field, e.to_string()));
    }
    if let Some(literal) = date
        .literal
        .as_deref()
        .or(date.raw.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        // Year-only literal fallback.
        if let Ok(year) = literal.parse::<i32>() {
            let (instant, precision) = parts_to_millis(year, None, None, field)?;
            return BibliographicDate::try_new_precise(
                instant,
                None,
                precision,
                certainty,
                Some(NonEmptyStr::try_new(literal).map_err(|e| {
                    map_field_err(import_codes::INVALID_FIELD, field, e.to_string())
                })?),
            )
            .map(Some)
            .map_err(|e| map_field_err(import_codes::INVALID_FIELD, field, e.to_string()));
        }
        return Err(map_field_err(
            import_codes::INVALID_FIELD,
            field,
            format!("unrecognized {field} date literal"),
        ));
    }
    Ok(None)
}

fn parts_to_millis(
    year: i32,
    month: Option<i32>,
    day: Option<i32>,
    field: &str,
) -> Result<(UtcMillis, DatePrecision), MapError> {
    let month_num = month.unwrap_or(1);
    let day_num = day.unwrap_or(1);
    let cal_month = time::Month::try_from(u8::try_from(month_num).unwrap_or(0))
        .map_err(|e| map_field_err(import_codes::INVALID_FIELD, field, e.to_string()))?;
    let date = time::Date::from_calendar_date(year, cal_month, u8::try_from(day_num).unwrap_or(0))
        .map_err(|e| map_field_err(import_codes::INVALID_FIELD, field, e.to_string()))?;
    let instant = UtcMillis::from_unix_ms(date.midnight().assume_utc().unix_timestamp() * 1000);
    let precision = if day.is_some() {
        DatePrecision::Day
    } else if month.is_some() {
        DatePrecision::Month
    } else {
        DatePrecision::Year
    };
    Ok((instant, precision))
}

fn format_parts(year: i32, month: Option<i32>, day: Option<i32>) -> String {
    match (month, day) {
        (Some(m), Some(d)) => format!("{year:04}-{m:02}-{d:02}"),
        (Some(m), None) => format!("{year:04}-{m:02}"),
        _ => year.to_string(),
    }
}

fn split_identifier_list(raw: &str) -> Vec<String> {
    raw.split([',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn flatten_original_fields(value: &Value) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(obj) = value.as_object() else {
        return out;
    };
    for (key, val) in obj {
        let rendered = match val {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => continue,
            other => other.to_string(),
        };
        if !rendered.is_empty() {
            out.insert(key.clone(), rendered);
        }
    }
    out
}
