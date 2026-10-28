//! BibLaTeX syntax parsing, reference inheritance, and field mapping.
//!
//! No [`oeuvre_core::Store`] or [`crate::AppState`] dependency.

use std::collections::HashMap;

use biblatex::{Bibliography, ChunksExt, Date, DateValue, Entry, EntryType, RetrievalError};
use oeuvre_core::{
    BibliographicDate, CreatorRole, DiagnosticSeverity, EntryProvenance, IMPORT_MAX_SOURCE_BYTES,
    IMPORT_PARSER_VERSION, ImportDiagnostic, ImportDraft, ImportEntryDraft, ImportFormat,
    ImportTempId, Name, NewResource, NonEmptyStr, ResourceKind, ResourceMetadata,
    SourceProvenance, UnresolvedCreator, UtcMillis, import_codes,
};

/// Fatal parse failure for a whole bibliography source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ParseError {
    pub code: String,
    pub message: String,
}

impl ParseError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

/// Parse BibLaTeX source into a format-neutral draft (no database access).
pub fn parse_biblatex(source: &str) -> Result<ImportDraft, ParseError> {
    if source.len() > IMPORT_MAX_SOURCE_BYTES {
        return Err(ParseError::new(
            import_codes::SOURCE_TOO_LARGE,
            format!(
                "Bibliography exceeds the {} MiB import limit",
                IMPORT_MAX_SOURCE_BYTES / (1024 * 1024)
            ),
        ));
    }
    let source = source.trim_start_matches('\u{feff}');
    let bib = parse_bibliography(source)?;
    let mut draft = ImportDraft {
        provenance: SourceProvenance {
            format: ImportFormat::Biblatex,
            parser_version: IMPORT_PARSER_VERSION,
        },
        entries: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut next_temp = 1u64;
    for entry in bib.iter() {
        match map_entry(entry) {
            Ok(mapped) => {
                let had_reference_fields =
                    entry.get("crossref").is_some() || entry.get("xref").is_some();
                let series_title = value(entry, "series");
                let series_number = if series_title.is_some() {
                    value(entry, "number")
                        .map(|v| non_empty(&v))
                        .transpose()
                        .map_err(|message| {
                            (
                                import_codes::INVALID_FIELD,
                                Some("number".to_owned()),
                                message,
                            )
                        })
                } else {
                    Ok(None)
                };
                let series_number = match series_number {
                    Ok(n) => n,
                    Err((code, field, message)) => {
                        draft.diagnostics.push(ImportDiagnostic {
                            code: code.to_owned(),
                            severity: DiagnosticSeverity::Error,
                            entry_key: Some(entry.key.clone()),
                            field,
                            location: None,
                            message,
                        });
                        continue;
                    }
                };
                let original_fields = entry
                    .fields
                    .keys()
                    .filter(|k| !matches!(k.as_str(), "note" | "annotation"))
                    .filter_map(|k| value(entry, k).map(|v| (k.clone(), v)))
                    .collect();
                draft.entries.push(ImportEntryDraft {
                    temp_id: ImportTempId::new(next_temp),
                    resource: mapped.resource,
                    unresolved_creators: mapped.creators,
                    series_title,
                    series_number,
                    provenance: EntryProvenance {
                        source_key: entry.key.clone(),
                        entry_type: Some(format!("{:?}", entry.entry_type)),
                        original_fields,
                    },
                    had_reference_fields,
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
                    entry_key: Some(entry.key.clone()),
                    field,
                    location: None,
                    message,
                });
            }
        }
    }
    Ok(draft)
}

struct MappedEntry {
    resource: NewResource,
    creators: Vec<UnresolvedCreator>,
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

fn value(entry: &Entry, field: &str) -> Option<String> {
    entry
        .get(field)
        .map(|chunks| chunks.format_verbatim().trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn non_empty(value: &str) -> Result<NonEmptyStr, String> {
    NonEmptyStr::try_new(value).map_err(|e| e.to_string())
}

fn names(
    entry: &Entry,
    field: &str,
    role: CreatorRole,
) -> Result<Vec<UnresolvedCreator>, MapError> {
    let people = match entry.get_as::<Vec<biblatex::Person>>(field) {
        Ok(people) => people,
        Err(RetrievalError::Missing(_)) => return Ok(vec![]),
        Err(e) => {
            return Err(map_field_err(
                import_codes::INVALID_FIELD,
                field,
                format!("{field}: {e}"),
            ));
        }
    };
    people
        .into_iter()
        .map(|p| {
            let last = if p.prefix.is_empty() {
                p.name
            } else {
                format!("{} {}", p.prefix, p.name)
            };
            Name::try_new(Some(p.given_name), Some(last), Some(p.suffix))
                .map(|name| UnresolvedCreator { role, name })
                .map_err(|e| {
                    map_field_err(import_codes::INVALID_FIELD, field, format!("{field}: {e}"))
                })
        })
        .collect()
}

fn date(entry: &Entry, field: &str) -> Result<Option<BibliographicDate>, MapError> {
    let parsed = if field == "date" {
        match entry.date() {
            Ok(biblatex::PermissiveType::Typed(date)) => Ok(date),
            Ok(biblatex::PermissiveType::Chunks(_)) => {
                return Err(map_field_err(
                    import_codes::INVALID_FIELD,
                    field,
                    "unrecognized publication date",
                ));
            }
            Err(error) => Err(error),
        }
    } else {
        entry.get_as::<Date>(field)
    };
    let date = match parsed {
        Ok(date) => date,
        Err(RetrievalError::Missing(_)) => return Ok(None),
        Err(e) => {
            return Err(map_field_err(
                import_codes::INVALID_FIELD,
                field,
                format!("{field}: {e}"),
            ));
        }
    };
    // The model stores one instant plus its original display, not a date interval.
    let at = match date.value {
        DateValue::At(at)
        | DateValue::After(at)
        | DateValue::Before(at)
        | DateValue::Between(at, _) => at,
    };
    let month = time::Month::try_from(at.month.unwrap_or(0) + 1)
        .map_err(|e| map_field_err(import_codes::INVALID_FIELD, field, e.to_string()))?;
    let day = time::Date::from_calendar_date(at.year, month, at.day.unwrap_or(0) + 1)
        .map_err(|e| map_field_err(import_codes::INVALID_FIELD, field, e.to_string()))?;
    let seconds = at.time.map(|t| t.to_utc_seconds() as i64).unwrap_or(0);
    let instant =
        UtcMillis::from_unix_ms((day.midnight().assume_utc().unix_timestamp() + seconds) * 1000);
    let display = value(entry, field).or_else(|| {
        if field == "date" {
            value(entry, "year")
        } else {
            None
        }
    });
    BibliographicDate::try_new(
        instant,
        display
            .map(|s| non_empty(&s))
            .transpose()
            .map_err(|e| map_field_err(import_codes::INVALID_FIELD, field, e))?,
    )
    .map(Some)
    .map_err(|e| map_field_err(import_codes::INVALID_FIELD, field, e.to_string()))
}

fn map_entry(entry: &Entry) -> Result<MappedEntry, MapError> {
    use EntryType::*;
    let kind = match &entry.entry_type {
        Book | MvBook | Booklet | Collection | MvCollection | Reference | MvReference
        | Proceedings | MvProceedings | Manual => ResourceKind::Book,
        Article | SuppPeriodical => ResourceKind::JournalArticle,
        InBook | InCollection | BookInBook | SuppBook | SuppCollection => ResourceKind::BookSection,
        InReference => ResourceKind::EncyclopediaArticle,
        InProceedings => ResourceKind::ConferencePaper,
        Thesis | MastersThesis | PhdThesis => ResourceKind::Thesis,
        Report | TechReport => ResourceKind::Report,
        Online => ResourceKind::Webpage,
        Dataset => ResourceKind::Dataset,
        Software => ResourceKind::ComputerProgram,
        Unpublished => ResourceKind::Manuscript,
        Periodical => ResourceKind::Journal,
        Unknown(name) if name == "preprint" => ResourceKind::Preprint,
        Misc if entry.get("eprint").is_some() => ResourceKind::Preprint,
        Misc => ResourceKind::Manuscript,
        other => {
            return Err(map_err(
                import_codes::UNSUPPORTED_TYPE,
                format!("unsupported entry type: {other:?}"),
            ));
        }
    };
    let title = value(entry, "title").ok_or_else(|| {
        map_err(import_codes::MISSING_TITLE, "entry needs a title")
    })?;
    let mut creators = Vec::new();
    for (field, role) in [
        ("author", CreatorRole::Author),
        ("editor", CreatorRole::Editor),
        ("translator", CreatorRole::Translator),
    ] {
        creators.extend(names(entry, field, role)?);
    }
    let source_fields = entry
        .fields
        .keys()
        .filter(|k| !matches!(k.as_str(), "note" | "annotation"))
        .filter_map(|k| value(entry, k).map(|v| (k.clone(), v)))
        .collect();
    let resource = NewResource {
        kind,
        title: non_empty(&title).map_err(|e| map_err(import_codes::INVALID_FIELD, e))?,
        creators: vec![],
        publisher: value(entry, "publisher"),
        published: date(entry, "date")?,
        accessed: date(entry, "urldate")?,
        parent_id: None,
        series_id: None,
        series_number: None,
        tag_ids: vec![],
        collection_ids: vec![],
        metadata: ResourceMetadata {
            citekey: Some(entry.key.clone()),
            venue: value(entry, "booktitle")
                .or_else(|| value(entry, "journaltitle"))
                .or_else(|| value(entry, "journal")),
            source_fields,
        },
        subtitle: value(entry, "subtitle")
            .map(|v| non_empty(&v))
            .transpose()
            .map_err(|e| map_field_err(import_codes::INVALID_FIELD, "subtitle", e))?,
        title_addon: value(entry, "titleaddon")
            .map(|v| non_empty(&v))
            .transpose()
            .map_err(|e| map_field_err(import_codes::INVALID_FIELD, "titleaddon", e))?,
        short_title: value(entry, "shorttitle")
            .map(|v| non_empty(&v))
            .transpose()
            .map_err(|e| map_field_err(import_codes::INVALID_FIELD, "shorttitle", e))?,
        place: value(entry, "location").or_else(|| value(entry, "address")),
        volume: value(entry, "volume"),
        issue: value(entry, "number").or_else(|| value(entry, "issue")),
        pages: value(entry, "pages"),
        total_pages: value(entry, "pagetotal"),
        article_number: value(entry, "eid"),
        edition: value(entry, "edition"),
        doi: value(entry, "doi"),
        url: value(entry, "url"),
        eprint: value(entry, "eprint"),
        isbns: value(entry, "isbn").into_iter().collect(),
        issns: value(entry, "issn").into_iter().collect(),
        isrns: value(entry, "isrn").into_iter().collect(),
        original_date: date(entry, "origdate")?,
        event_date: date(entry, "eventdate")?,
        language: value(entry, "langid").or_else(|| value(entry, "language")),
        document_type: value(entry, "type").or_else(|| match entry.entry_type {
            MastersThesis => Some("Master's thesis".into()),
            PhdThesis => Some("PhD thesis".into()),
            TechReport => Some("Technical report".into()),
            _ => None,
        }),
        publication_method: value(entry, "howpublished"),
        institution: value(entry, "institution"),
        organization: value(entry, "organization"),
        school: value(entry, "school"),
        abstract_text: value(entry, "abstract"),
        keywords: value(entry, "keywords")
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    };
    Ok(MappedEntry {
        resource,
        creators,
    })
}

fn parse_bibliography(source: &str) -> Result<Bibliography, ParseError> {
    let syntax = |e: biblatex::ParseError| {
        let message = format!("BibLaTeX syntax: {e}");
        let code = if message.contains("duplicate") {
            import_codes::DUPLICATE_KEY
        } else {
            import_codes::SYNTAX
        };
        ParseError::new(code, message)
    };
    let raw = biblatex::RawBibliography::parse(source).map_err(syntax)?;
    // Temporary field names must not collide with arbitrary user source fields.
    let mut prefix = "oeuvre_import_".to_owned();
    while raw.entries.iter().any(|entry| {
        entry
            .v
            .fields
            .iter()
            .any(|field| field.key.v.to_ascii_lowercase().starts_with(&prefix))
    }) {
        prefix.push('_');
    }
    let crossref_alias = format!("{prefix}crossref");
    let xdata_alias = format!("{prefix}xdata");
    let mut explicit = raw.clone();
    // Parse fields without following references, so cycles cannot overflow the parser's stack.
    for entry in &mut explicit.entries {
        for field in &mut entry.v.fields {
            if field.key.v.eq_ignore_ascii_case("crossref") {
                field.key.v = crossref_alias.as_str();
            }
            if field.key.v.eq_ignore_ascii_case("xdata") {
                field.key.v = xdata_alias.as_str();
            }
        }
    }
    let mut explicit = Bibliography::from_raw(explicit).map_err(syntax)?;
    for entry in explicit.iter_mut() {
        for (temporary, original) in [
            (crossref_alias.as_str(), "crossref"),
            (xdata_alias.as_str(), "xdata"),
        ] {
            if let Some(chunks) = entry.fields.remove(temporary) {
                entry.fields.insert(original.into(), chunks);
            }
        }
    }

    fn visit<'a>(
        key: &'a str,
        bib: &'a Bibliography,
        path: &mut Vec<&'a str>,
        done: &mut HashMap<&'a str, (usize, usize)>,
    ) -> Result<(usize, usize), ParseError> {
        let invalid = |code: &'static str| {
            ParseError::new(
                code,
                format!(
                    "cyclic, excessively deep, or excessively expanding BibLaTeX reference at {key}"
                ),
            )
        };
        if path.contains(&key) {
            return Err(invalid(import_codes::REFERENCE_CYCLE));
        }
        if path.len() >= 64 {
            return Err(invalid(import_codes::REFERENCE_TOO_DEEP));
        }
        if let Some(&(depth, work)) = done.get(key) {
            if path.len() + depth > 64 {
                return Err(invalid(import_codes::REFERENCE_TOO_DEEP));
            }
            return Ok((depth, work));
        }
        let Some(entry) = bib.get(key) else {
            return Ok((0, 0));
        };
        path.push(key);
        let (mut depth, mut work) = (1, 1);
        for field in ["crossref", "xdata"] {
            if let Some(chunks) = entry.get(field) {
                let refs = chunks.format_verbatim();
                for reference in refs.split(',').map(str::trim) {
                    if let Some(parent) = bib.get(reference) {
                        let (parent_depth, parent_work) = visit(&parent.key, bib, path, done)?;
                        depth = depth.max(1 + parent_depth);
                        work += parent_work;
                        if path.len() - 1 + depth > 64 {
                            return Err(invalid(import_codes::REFERENCE_TOO_DEEP));
                        }
                        if work > 100_000 {
                            return Err(invalid(import_codes::REFERENCE_EXPAND));
                        }
                    }
                }
            }
        }
        path.pop();
        done.insert(key, (depth, work));
        Ok((depth, work))
    }
    let mut done = HashMap::new();
    let mut total_work = 0;
    for entry in explicit.iter() {
        total_work += visit(&entry.key, &explicit, &mut Vec::new(), &mut done)?.1;
        if total_work > 1_000_000 {
            return Err(ParseError::new(
                import_codes::REFERENCE_EXPAND,
                "BibLaTeX references expand too many entries",
            ));
        }
    }

    fn resolve(key: &str, bib: &Bibliography, cache: &mut HashMap<String, Entry>) -> Entry {
        if let Some(entry) = cache.get(key) {
            return entry.clone();
        }
        let mut entry = bib.get(key).expect("reference exists").clone();
        let references: Vec<String> = ["crossref", "xdata"]
            .into_iter()
            .filter_map(|field| value(&entry, field))
            .flat_map(|s| {
                s.split(',')
                    .map(|s| s.trim().to_owned())
                    .collect::<Vec<_>>()
            })
            .collect();
        for reference in references {
            if bib.get(&reference).is_none() {
                continue;
            }
            let parent = resolve(&reference, bib, cache);
            let owns_date = entry.get("date").is_some() || entry.get("year").is_some();
            // Resolve ancestors first, preserving explicit child dates at every level.
            // Do not parse semantic field values here: a bad parent date should skip
            // only the affected entries, not prevent importing unrelated resources.
            for (field, chunks) in &parent.fields {
                if matches!(
                    field.as_str(),
                    "crossref" | "xref" | "xdata" | "note" | "annotation"
                ) {
                    continue;
                }
                if parent.entry_type != EntryType::XData
                    && matches!(
                        field.as_str(),
                        "title" | "subtitle" | "titleaddon" | "shorttitle"
                    )
                {
                    continue;
                }
                if owns_date && matches!(field.as_str(), "date" | "year" | "month" | "day") {
                    continue;
                }
                entry
                    .fields
                    .entry(field.clone())
                    .or_insert_with(|| chunks.clone());
            }
            let container_field = match entry.entry_type {
                EntryType::Article | EntryType::SuppPeriodical
                    if parent.entry_type == EntryType::Periodical =>
                {
                    Some("journaltitle")
                }
                EntryType::InBook
                | EntryType::InCollection
                | EntryType::InProceedings
                | EntryType::InReference
                | EntryType::BookInBook
                | EntryType::SuppBook
                | EntryType::SuppCollection
                    if parent.entry_type.is_collection() =>
                {
                    Some("booktitle")
                }
                _ if parent.entry_type.is_multi_volume() => Some("maintitle"),
                _ => None,
            };
            if let (Some(field), Some(title)) = (container_field, parent.fields.get("title")) {
                entry
                    .fields
                    .entry(field.into())
                    .or_insert_with(|| title.clone());
            }
        }
        cache.insert(key.to_owned(), entry.clone());
        entry
    }
    let mut cache = HashMap::new();
    let mut resolved = Bibliography::new();
    for entry in explicit.iter() {
        resolved.insert(resolve(&entry.key, &explicit, &mut cache));
    }
    Ok(resolved)
}
