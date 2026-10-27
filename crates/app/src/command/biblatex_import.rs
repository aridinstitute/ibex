//! BibLaTeX import adds new resources; it never overwrites existing records.
//! Syntax and cross-reference resolution happen before writes. Invalid or
//! unsupported entries are skipped individually; storage failures stop the batch.

use std::collections::HashMap;

use biblatex::{Bibliography, ChunksExt, Date, DateValue, Entry, EntryType, RetrievalError};
use oeuvre_core::{
    BibliographicDate, CreatorRole, CreatorSpec, Name, NewResource, NewSeries, NonEmptyStr,
    ResourceId, ResourceKind, ResourceMetadata, SeriesId, UtcMillis,
};

use crate::event::{self, Event};
use crate::{AppState, CommandError, Handler, Outcome};

use super::resource::CreateResource;

#[derive(Debug, Clone)]
pub struct ImportBiblatex {
    pub source_text: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportedEntry {
    pub entry_key: String,
    pub resource_id: ResourceId,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportIssue {
    pub entry_key: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ImportReport {
    pub imported: Vec<ImportedEntry>,
    pub skipped: Vec<ImportIssue>,
    pub warnings: Vec<ImportIssue>,
}

pub(crate) struct Imported(pub ImportReport);

impl From<Imported> for Outcome {
    fn from(Imported(report): Imported) -> Self {
        Outcome::ImportBiblatex(report)
    }
}

#[async_trait::async_trait]
impl Handler for ImportBiblatex {
    const NAME: &'static str = "import_biblatex";
    type Output = Imported;

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Imported, CommandError> {
        ev.set(event::IMPORT_IMPORTED_COUNT, 0u64);
        ev.set(event::IMPORT_SKIPPED_COUNT, 0u64);
        ev.set(event::IMPORT_WARNING_COUNT, 0u64);
        let bib = parse_bibliography(self.source_text.trim_start_matches('\u{feff}'))?;
        let mut report = ImportReport::default();
        let mut series = HashMap::<String, SeriesId>::new();
        for entry in bib.iter() {
            ev.set(event::IMPORT_ENTRY_KEY, entry.key.clone());
            let mapped = map_entry(entry);
            let result = match mapped {
                Ok((mut new, names)) => {
                    match resolve_creators(state, names).await {
                        Ok(creators) => {
                            new.creators = creators;
                            if let Some(title) = value(entry, "series") {
                                let id = if let Some(id) = series.get(&title) {
                                    *id
                                } else {
                                    let created = state
                                        .store()
                                        .create_series(NewSeries {
                                            title: non_empty(&title)?,
                                        })
                                        .await?;
                                    series.insert(title, created.id);
                                    created.id
                                };
                                new.series_id = Some(id);
                                new.series_number =
                                    value(entry, "number").map(|v| non_empty(&v)).transpose()?;
                            }
                            // Share validation without emitting a completion event per row.
                            CreateResource { new }.run(state, ev).await.map(|r| r.0)
                        }
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            };
            match result {
                Ok(resource) => {
                    report.imported.push(ImportedEntry {
                        entry_key: entry.key.clone(),
                        resource_id: resource.id,
                    });
                    if entry.get("crossref").is_some() || entry.get("xref").is_some() {
                        report.warnings.push(ImportIssue { entry_key: Some(entry.key.clone()), message: "Reference metadata was resolved/preserved; no parent-resource link was created".into() });
                    }
                }
                Err(CommandError::Invalid(message)) => report.skipped.push(ImportIssue {
                    entry_key: Some(entry.key.clone()),
                    message,
                }),
                Err(error) => return Err(error),
            }
            ev.set(event::IMPORT_IMPORTED_COUNT, report.imported.len());
            ev.set(event::IMPORT_SKIPPED_COUNT, report.skipped.len());
            ev.set(event::IMPORT_WARNING_COUNT, report.warnings.len());
        }
        Ok(Imported(report))
    }
}

async fn resolve_creators(
    state: &AppState,
    names: Vec<(CreatorRole, Name)>,
) -> Result<Vec<CreatorSpec>, CommandError> {
    // Check all ambiguities before creating any people for this entry.
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
    let mut created = Vec::<(Name, oeuvre_core::PersonId)>::new();
    for (role, name, id) in resolved {
        let person_id = if let Some(id) = id {
            id
        } else if let Some((_, id)) = created.iter().find(|(n, _)| n == &name) {
            *id
        } else {
            let id = state.store().create_person(name.clone(), vec![]).await?.id;
            created.push((name, id));
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
    Ok(creators)
}

fn value(entry: &Entry, field: &str) -> Option<String> {
    entry
        .get(field)
        .map(|chunks| chunks.format_verbatim().trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn non_empty(value: &str) -> Result<NonEmptyStr, CommandError> {
    NonEmptyStr::try_new(value).map_err(|e| CommandError::Invalid(e.to_string()))
}

fn names(
    entry: &Entry,
    field: &str,
    role: CreatorRole,
) -> Result<Vec<(CreatorRole, Name)>, CommandError> {
    let people = match entry.get_as::<Vec<biblatex::Person>>(field) {
        Ok(people) => people,
        Err(RetrievalError::Missing(_)) => return Ok(vec![]),
        Err(e) => return Err(CommandError::Invalid(format!("{field}: {e}"))),
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
                .map(|name| (role, name))
                .map_err(|e| CommandError::Invalid(format!("{field}: {e}")))
        })
        .collect()
}

fn date(entry: &Entry, field: &str) -> Result<Option<BibliographicDate>, CommandError> {
    let parsed = if field == "date" {
        match entry.date() {
            Ok(biblatex::PermissiveType::Typed(date)) => Ok(date),
            Ok(biblatex::PermissiveType::Chunks(_)) => {
                return Err(CommandError::Invalid(
                    "unrecognized publication date".into(),
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
        Err(e) => return Err(CommandError::Invalid(format!("{field}: {e}"))),
    };
    // The model stores one instant plus its original display, not a date interval.
    let at = match date.value {
        DateValue::At(at)
        | DateValue::After(at)
        | DateValue::Before(at)
        | DateValue::Between(at, _) => at,
    };
    let month = time::Month::try_from(at.month.unwrap_or(0) + 1)
        .map_err(|e| CommandError::Invalid(e.to_string()))?;
    let day = time::Date::from_calendar_date(at.year, month, at.day.unwrap_or(0) + 1)
        .map_err(|e| CommandError::Invalid(e.to_string()))?;
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
    BibliographicDate::try_new(instant, display.map(|s| non_empty(&s)).transpose()?)
        .map(Some)
        .map_err(|e| CommandError::Invalid(e.to_string()))
}

fn map_entry(entry: &Entry) -> Result<(NewResource, Vec<(CreatorRole, Name)>), CommandError> {
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
            return Err(CommandError::Invalid(format!(
                "unsupported entry type: {other:?}"
            )));
        }
    };
    let title =
        value(entry, "title").ok_or_else(|| CommandError::Invalid("entry needs a title".into()))?;
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
    let new = NewResource {
        kind,
        title: non_empty(&title)?,
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
            .transpose()?,
        title_addon: value(entry, "titleaddon")
            .map(|v| non_empty(&v))
            .transpose()?,
        short_title: value(entry, "shorttitle")
            .map(|v| non_empty(&v))
            .transpose()?,
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
    Ok((new, creators))
}

fn parse_bibliography(source: &str) -> Result<Bibliography, CommandError> {
    let invalid = |e: biblatex::ParseError| CommandError::Invalid(format!("BibLaTeX syntax: {e}"));
    let raw = biblatex::RawBibliography::parse(source).map_err(invalid)?;
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
    let mut explicit = Bibliography::from_raw(explicit).map_err(invalid)?;
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
    ) -> Result<(usize, usize), CommandError> {
        let invalid = || {
            CommandError::Invalid(format!(
                "cyclic, excessively deep, or excessively expanding BibLaTeX reference at {key}"
            ))
        };
        if path.len() >= 64 || path.contains(&key) {
            return Err(invalid());
        }
        if let Some(&(depth, work)) = done.get(key) {
            if path.len() + depth > 64 {
                return Err(invalid());
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
                        if path.len() - 1 + depth > 64 || work > 100_000 {
                            return Err(invalid());
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
            return Err(CommandError::Invalid(
                "BibLaTeX references expand too many entries".into(),
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
