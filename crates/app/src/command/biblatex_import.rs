use biblatex::{Bibliography, ChunksExt, Entry, EntryType, RetrievalError};
use oeuvre_core::{
    BibliographicDate, CreatorRole, CreatorSpec, Name, NewResource, NonEmptyStr, ResourceId,
    ResourceKind, ResourceMetadata, UtcMillis,
};

use crate::event::{self, Event};
use crate::{AppState, CommandError, Handler, Outcome};

use super::resource::CreateResource;

/// Import BibLaTeX text into the library. The first slice accepts one book;
/// batch handling and other entry types will extend this command.
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
        let bib = Bibliography::parse(&self.source_text)
            .map_err(|e| CommandError::Invalid(format!("BibLaTeX syntax: {e}")))?;
        if bib.len() != 1 {
            return Err(CommandError::Invalid(
                "this import slice requires exactly one entry".into(),
            ));
        }
        let entry = bib.iter().next().expect("checked entry count");
        ev.set(event::IMPORT_ENTRY_KEY, entry.key.clone());
        let (mut new, authors) = map_book(entry)?;
        for name in authors {
            let people = state.store().find_persons_by_name(&name).await?;
            let person_id = match people.as_slice() {
                [person] => person.id,
                [] => state.store().create_person(name, vec![]).await?.id,
                _ => {
                    return Err(CommandError::Invalid(format!(
                        "ambiguous author name in {}",
                        entry.key
                    )));
                }
            };
            new.creators.push(CreatorSpec {
                person_id,
                role: CreatorRole::Author,
                credited_as: None,
            });
        }
        // Invoke the existing validator/persistence handler inside this command,
        // rather than emitting a second top-level completion event.
        let resource = CreateResource { new }.run(state, ev).await?.0;
        ev.set(event::IMPORT_IMPORTED_COUNT, 1u64);
        Ok(Imported(ImportReport {
            imported: vec![ImportedEntry {
                entry_key: entry.key.clone(),
                resource_id: resource.id,
            }],
            ..Default::default()
        }))
    }
}

fn value(entry: &Entry, field: &str) -> Option<String> {
    entry
        .get(field)
        .map(|chunks| chunks.format_verbatim().trim().to_owned())
}

fn map_book(entry: &Entry) -> Result<(NewResource, Vec<Name>), CommandError> {
    if entry.entry_type != EntryType::Book {
        return Err(CommandError::Invalid(format!(
            "unsupported BibLaTeX entry type for {}: {:?}",
            entry.key, entry.entry_type
        )));
    }
    let title = value(entry, "title")
        .and_then(|s| NonEmptyStr::try_new(s).ok())
        .ok_or_else(|| CommandError::Invalid(format!("{} needs a title", entry.key)))?;
    let authors = match entry.author() {
        Ok(authors) => authors,
        Err(RetrievalError::Missing(_)) => Vec::new(),
        Err(e) => return Err(CommandError::Invalid(format!("{} author: {e}", entry.key))),
    };
    let authors = authors
        .into_iter()
        .map(|p| {
            let last = if p.prefix.is_empty() {
                p.name
            } else {
                format!("{} {}", p.prefix, p.name)
            };
            Name::try_new(Some(p.given_name), Some(last), Some(p.suffix))
                .map_err(|e| CommandError::Invalid(format!("{} author: {e}", entry.key)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let published = value(entry, "year")
        .map(|year| {
            let year = year.parse::<i64>().map_err(|_| {
                CommandError::Invalid(format!("{} has invalid year {year:?}", entry.key))
            })?;
            let instant = UtcMillis::jan_1_utc(year).ok_or_else(|| {
                CommandError::Invalid(format!("{} has out-of-range year {year}", entry.key))
            })?;
            BibliographicDate::try_new(instant, None)
                .map_err(|e| CommandError::Invalid(e.to_string()))
        })
        .transpose()?;
    let mut metadata = ResourceMetadata {
        citekey: Some(entry.key.clone()),
        venue: value(entry, "booktitle").or_else(|| value(entry, "journaltitle")),
        ..Default::default()
    };
    for key in entry.fields.keys() {
        if let Some(field) = value(entry, key) {
            metadata.source_fields.insert(key.clone(), field);
        }
    }
    Ok((
        NewResource {
            kind: ResourceKind::Book,
            title,
            creators: vec![],
            publisher: value(entry, "publisher"),
            published,
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
            volume: value(entry, "volume"),
            issue: value(entry, "number").or_else(|| value(entry, "issue")),
            pages: value(entry, "pages"),
            total_pages: None,
            article_number: None,
            edition: None,
            doi: value(entry, "doi"),
            url: value(entry, "url"),
            eprint: None,
            isbns: value(entry, "isbn").into_iter().collect(),
            issns: vec![],
            isrns: vec![],
            original_date: None,
            event_date: None,
            language: value(entry, "langid").or_else(|| value(entry, "language")),
            document_type: None,
            publication_method: None,
            institution: None,
            organization: None,
            school: None,
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
            metadata,
        },
        authors,
    ))
}
