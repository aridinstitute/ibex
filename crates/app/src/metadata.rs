//! Free ISBN/DOI metadata lookup shared by desktop and CLI.
use crate::command::{
    CreatePerson, CreateResource, FindPersonsByName, GetResource, ListResourcesQueryPage,
    UpdateResource,
};
use crate::{AppState, Command, Outcome};
use std::time::Duration;

use ibex_core::{
    BibliographicDate, CreatorRole, CreatorSpec, DateCertainty, DatePrecision, Identifier, Name,
    NewResource, NonEmptyStr, Pages, Resource, ResourceFilter, ResourceId, ResourceKind,
    ResourceQuery, UtcMillis,
};
use serde_json::Value;

/// Fetched fields plus authors that must be linked to person records before saving.
#[derive(Debug)]
pub struct Lookup {
    pub resource: NewResource,
    pub authors: Vec<Name>,
    pub editors: Vec<Name>,
    pub journal: Option<NewResource>,
}

fn author_name(full: &str) -> Option<Name> {
    let full = full.trim();
    let (name, suffix) = if let Some(name) = full.strip_suffix(" Jr.") {
        (name, Some("Jr."))
    } else {
        (full, None)
    };
    match name.rsplit_once(' ') {
        Some((first, last)) => Name::try_new(Some(first), Some(last), suffix).ok(),
        None => Name::try_new(None::<&str>, Some(name), suffix).ok(),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum IdentifierInput {
    Isbn(String),
    Doi(String),
}

fn detect(input: &str) -> Result<IdentifierInput, String> {
    let input = input.trim();
    let doi = input
        .strip_prefix("https://doi.org/")
        .or_else(|| input.strip_prefix("http://doi.org/"))
        .or_else(|| input.strip_prefix("doi:"))
        .unwrap_or(input);
    if doi.len() > 3
        && doi.starts_with("10.")
        && doi.contains('/')
        && !doi.chars().any(char::is_whitespace)
    {
        return Ok(IdentifierInput::Doi(doi.to_owned()));
    }
    let isbn: String = input
        .chars()
        .filter(|c| *c != '-' && !c.is_ascii_whitespace())
        .collect();
    let bytes = isbn.as_bytes();
    let valid = match bytes.len() {
        10 => {
            bytes[..9].iter().all(u8::is_ascii_digit)
                && (bytes[9].is_ascii_digit() || matches!(bytes[9], b'X' | b'x'))
                && bytes
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        (10 - i as u32)
                            * if *b == b'X' || *b == b'x' {
                                10
                            } else {
                                (b - b'0') as u32
                            }
                    })
                    .sum::<u32>()
                    % 11
                    == 0
        }
        13 => {
            bytes.iter().all(u8::is_ascii_digit)
                && bytes
                    .iter()
                    .enumerate()
                    .map(|(i, b)| (b - b'0') as u32 * if i % 2 == 0 { 1 } else { 3 })
                    .sum::<u32>()
                    % 10
                    == 0
        }
        _ => false,
    };
    if valid {
        Ok(IdentifierInput::Isbn(isbn.to_ascii_uppercase()))
    } else {
        Err("Enter a valid ISBN-10, ISBN-13, or DOI".into())
    }
}

fn year_date(year: i32) -> Option<BibliographicDate> {
    let instant = UtcMillis::jan_1_utc(i64::from(year))?;
    BibliographicDate::try_new_precise(
        instant,
        None,
        DatePrecision::Year,
        DateCertainty::Exact,
        None,
    )
    .ok()
}

fn calendar_date(year: i32, month: u8, day: Option<u8>) -> Option<BibliographicDate> {
    let month = time::Month::try_from(month).ok()?;
    let date = time::Date::from_calendar_date(year, month, day.unwrap_or(1)).ok()?;
    let instant = UtcMillis::from_unix_ms(date.midnight().assume_utc().unix_timestamp() * 1_000);
    BibliographicDate::try_new_precise(
        instant,
        None,
        if day.is_some() {
            DatePrecision::Day
        } else {
            DatePrecision::Month
        },
        DateCertainty::Exact,
        None,
    )
    .ok()
}

fn crossref_date(parts: &Value) -> Option<BibliographicDate> {
    let parts = parts["date-parts"][0].as_array()?;
    let year = i32::try_from(parts.first()?.as_i64()?).ok()?;
    let month = parts
        .get(1)
        .and_then(Value::as_u64)
        .and_then(|m| u8::try_from(m).ok());
    let day = parts
        .get(2)
        .and_then(Value::as_u64)
        .and_then(|d| u8::try_from(d).ok());
    month
        .and_then(|m| calendar_date(year, m, day))
        .or_else(|| year_date(year))
}

fn openlibrary_date(raw: &str) -> Option<BibliographicDate> {
    let numbers: Vec<&str> = raw
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .collect();
    let year: i32 = numbers.iter().find(|s| s.len() == 4)?.parse().ok()?;
    if raw.starts_with(&year.to_string()) {
        // ISO year-month-day; never infer a day from ambiguous slash dates.
        let iso = raw.as_bytes().get(4) == Some(&b'-');
        if iso {
            let month = numbers.get(1).and_then(|s| s.parse().ok());
            let day = numbers.get(2).and_then(|s| s.parse().ok());
            if let Some(date) = month.and_then(|m| calendar_date(year, m, day)) {
                return Some(date);
            }
        }
    }
    let month = raw
        .split(|c: char| !c.is_ascii_alphabetic())
        .find_map(|part| match part.to_ascii_lowercase().as_str() {
            "jan" | "january" => Some(1),
            "feb" | "february" => Some(2),
            "mar" | "march" => Some(3),
            "apr" | "april" => Some(4),
            "may" => Some(5),
            "jun" | "june" => Some(6),
            "jul" | "july" => Some(7),
            "aug" | "august" => Some(8),
            "sep" | "sept" | "september" => Some(9),
            "oct" | "october" => Some(10),
            "nov" | "november" => Some(11),
            "dec" | "december" => Some(12),
            _ => None,
        });
    let day = numbers
        .iter()
        .find(|part| part.len() <= 2)
        .and_then(|s| s.parse().ok());
    month
        .and_then(|m| calendar_date(year, m, day))
        .or_else(|| year_date(year))
}

fn unique_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut result = Vec::new();
    for value in values {
        if !result
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&value))
        {
            result.push(value);
        }
    }
    result
}

fn first_non_empty(value: &Value) -> Option<NonEmptyStr> {
    value.as_str().and_then(|s| NonEmptyStr::try_new(s).ok())
}

fn first_listed_string(value: &Value) -> Option<NonEmptyStr> {
    let value = value
        .as_array()
        .and_then(|values| values.first())
        .unwrap_or(value);
    first_non_empty(value)
}

fn abstract_text(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        if let Some(end) = rest[start..].find('>') {
            let tag = &rest[start + 1..start + end];
            if tag.starts_with('/') || tag.ends_with("br/") {
                out.push(' ');
            }
            rest = &rest[start + end + 1..];
        } else {
            rest = &rest[start..];
            break;
        }
    }
    out.push_str(rest);
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'");
    let clean = out.split_whitespace().collect::<Vec<_>>().join(" ");
    (!clean.is_empty()).then_some(clean)
}

fn first_string(value: &Value, path: &str) -> Option<String> {
    value
        .get(path)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub fn empty_resource(title: &str, kind: ResourceKind) -> Result<NewResource, String> {
    let title = NonEmptyStr::try_new(title)
        .map_err(|_| "Metadata provider returned no title".to_owned())?;
    Ok(NewResource {
        kind,
        title,
        creators: vec![],
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
    })
}

fn openlibrary(book: &Value, isbn: &str) -> Result<Lookup, String> {
    let mut new = empty_resource(
        first_string(book, "title").as_deref().unwrap_or(""),
        ResourceKind::Book,
    )?;
    new.isbns.push(Identifier::new(isbn));
    for field in ["isbn_10", "isbn_13"] {
        for value in book["identifiers"][field]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let id = Identifier::new(value);
            if !new
                .isbns
                .iter()
                .any(|existing| existing.lookup_key() == id.lookup_key())
            {
                new.isbns.push(id);
            }
        }
    }
    new.subtitle = first_non_empty(&book["subtitle"]);
    new.place = book["publish_places"]
        .as_array()
        .and_then(|places| places.first())
        .and_then(|place| first_string(place, "name"));
    new.url = first_string(book, "url");
    new.keywords = unique_strings(
        book["subjects"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|subject| first_string(subject, "name")),
    );
    new.publisher = book["publishers"]
        .as_array()
        .and_then(|v| v.first())
        .and_then(|v| first_string(v, "name"));
    new.total_pages = book["number_of_pages"].as_u64().map(|v| v.to_string());
    new.published = book["publish_date"].as_str().and_then(openlibrary_date);
    Ok(Lookup {
        resource: new,
        journal: None,
        authors: book["authors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| first_string(a, "name"))
            .filter_map(|s| author_name(&s))
            .collect(),
        editors: vec![],
    })
}

fn crossref(work: &Value, doi: &str) -> Result<Lookup, String> {
    let title = work["title"]
        .as_array()
        .and_then(|v| v.first())
        .and_then(Value::as_str)
        .unwrap_or("");
    let kind = match work["type"].as_str() {
        Some("book" | "edited-book" | "monograph") => ResourceKind::Book,
        Some("book-chapter" | "book-section") => ResourceKind::BookSection,
        Some("proceedings-article") => ResourceKind::ConferencePaper,
        Some("posted-content") => ResourceKind::Preprint,
        Some("report") => ResourceKind::Report,
        _ => ResourceKind::JournalArticle,
    };
    let mut new = empty_resource(title, kind)?;
    new.doi = Some(doi.to_owned());
    new.publisher = first_string(work, "publisher");
    new.volume = first_string(work, "volume");
    new.issue = first_string(work, "issue");
    new.url = first_string(work, "URL");
    new.subtitle = first_listed_string(&work["subtitle"]);
    new.short_title = first_listed_string(&work["short-title"]);
    new.pages = first_string(work, "page").and_then(|page| Pages::parse(&page).ok());
    new.article_number = first_string(work, "article-number");
    new.language = first_string(work, "language");
    new.abstract_text = work["abstract"].as_str().and_then(abstract_text);
    new.keywords = unique_strings(
        work["subject"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
    );
    new.published = crossref_date(&work["published"]).or_else(|| crossref_date(&work["issued"]));
    new.isbns = work["ISBN"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(Identifier::new)
        .collect();
    new.issns = work["ISSN"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(Identifier::new)
        .collect();
    let journal = if kind == ResourceKind::JournalArticle {
        Some(
            work["container-title"]
                .as_array()
                .and_then(|titles| titles.first())
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(|title| {
                    let mut journal = empty_resource(title, ResourceKind::Journal)?;
                    journal.issns = std::mem::take(&mut new.issns);
                    journal.short_title = first_listed_string(&work["short-container-title"]);
                    journal.publisher = new.publisher.clone();
                    Ok::<_, String>(journal)
                })
                .transpose()?
                .ok_or_else(|| "Crossref returned no journal title for this article".to_owned())?,
        )
    } else {
        None
    };
    Ok(Lookup {
        resource: new,
        journal,
        authors: work["author"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| {
                let last = a["family"].as_str()?.trim();
                Name::try_new(a["given"].as_str(), Some(last), None::<&str>).ok()
            })
            .collect(),
        editors: work["editor"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|editor| {
                let last = editor["family"].as_str()?.trim();
                Name::try_new(editor["given"].as_str(), Some(last), None::<&str>).ok()
            })
            .collect(),
    })
}

pub async fn lookup(input: &str) -> Result<Lookup, String> {
    lookup_at(
        input,
        "https://openlibrary.org/api/books",
        "https://api.crossref.org/works",
    )
    .await
}

async fn lookup_at(input: &str, books: &str, works: &str) -> Result<Lookup, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent(concat!(
            "Ibex/",
            env!("CARGO_PKG_VERSION"),
            " (metadata lookup)"
        ))
        .build()
        .map_err(|e| e.to_string())?;
    let identifier = detect(input)?;
    let (url, key) = match &identifier {
        IdentifierInput::Isbn(isbn) => {
            let mut url = reqwest::Url::parse(books).map_err(|e| e.to_string())?;
            url.query_pairs_mut()
                .append_pair("bibkeys", &format!("ISBN:{isbn}"))
                .append_pair("jscmd", "data")
                .append_pair("format", "json");
            (url, Some(format!("ISBN:{isbn}")))
        }
        IdentifierInput::Doi(doi) => {
            let mut url = reqwest::Url::parse(works).map_err(|e| e.to_string())?;
            url.path_segments_mut()
                .map_err(|_| "Invalid Crossref URL".to_owned())?
                .push(doi);
            (url, None)
        }
    };
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Metadata request failed: {e}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("No metadata found for this identifier".into());
    }
    let response = response
        .error_for_status()
        .map_err(|e| format!("Metadata provider failed: {e}"))?;
    let data: Value = response
        .json()
        .await
        .map_err(|e| format!("Invalid metadata response: {e}"))?;
    match identifier {
        IdentifierInput::Isbn(isbn) => openlibrary(
            data.get(key.as_deref().unwrap())
                .ok_or("No metadata found for this ISBN")?,
            &isbn,
        ),
        IdentifierInput::Doi(doi) => crossref(
            data.get("message")
                .ok_or("No metadata found for this DOI")?,
            &doi,
        ),
    }
}

/// Preserve all local fields and fill only absent bibliographic metadata.
pub fn fill_gaps(record: &Resource, fetched: NewResource) -> NewResource {
    let mut current = record.to_new();
    if current.kind == ResourceKind::JournalArticle && current.parent_id.is_none() {
        current.parent_id = fetched.parent_id;
    }
    let existing_roles: Vec<_> = current
        .creators
        .iter()
        .map(|creator| creator.role)
        .collect();
    for creator in fetched.creators {
        if !existing_roles.contains(&creator.role)
            && !current
                .creators
                .iter()
                .any(|local| local.role == creator.role && local.person_id == creator.person_id)
        {
            current.creators.push(creator);
        }
    }
    macro_rules! fill { ($($field:ident),+ $(,)?) => { $(if current.$field.is_none() { current.$field = fetched.$field; })+ }; }
    fill!(
        publisher,
        published,
        subtitle,
        short_title,
        place,
        volume,
        issue,
        pages,
        total_pages,
        article_number,
        edition,
        doi,
        url,
        language,
        abstract_text
    );
    for isbn in fetched.isbns {
        if !current
            .isbns
            .iter()
            .any(|id| id.lookup_key() == isbn.lookup_key())
        {
            current.isbns.push(isbn);
        }
    }
    for issn in fetched.issns {
        if !current
            .issns
            .iter()
            .any(|id| id.lookup_key() == issn.lookup_key())
        {
            current.issns.push(issn);
        }
    }
    for keyword in fetched.keywords {
        if !current
            .keywords
            .iter()
            .any(|local| local.eq_ignore_ascii_case(&keyword))
        {
            current.keywords.push(keyword);
        }
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_identifiers_without_guessing() {
        assert_eq!(
            detect("978-0-306-40615-7"),
            Ok(IdentifierInput::Isbn("9780306406157".into()))
        );
        assert_eq!(
            detect("https://doi.org/10.1000/example"),
            Ok(IdentifierInput::Doi("10.1000/example".into()))
        );
        assert!(detect("9780306406158").is_err());
        assert!(detect("not an identifier").is_err());
    }
    #[test]
    fn maps_provider_fields() {
        let book = serde_json::json!({"title":"Example", "publishers":[{"name":"Press"}], "number_of_pages":120, "publish_date":"2001"});
        let new = openlibrary(&book, "9780306406157").unwrap().resource;
        assert_eq!(new.title.as_str(), "Example");
        assert_eq!(new.publisher.as_deref(), Some("Press"));
        assert_eq!(new.isbns[0].lookup_key(), "9780306406157");
        let work = serde_json::json!({"title":["Paper"], "type":"journal-article", "container-title":["Journal"], "publisher":"Society", "published":{"date-parts":[[2020,5,1]]}});
        let new = crossref(&work, "10.1000/example").unwrap().resource;
        assert_eq!(new.kind, ResourceKind::JournalArticle);
        assert_eq!(new.doi.as_deref(), Some("10.1000/example"));
        assert!(new.published.is_some());
    }

    #[test]
    fn maps_openlibrary_book_details_and_other_isbns() {
        let book = serde_json::json!({
            "title": "Example", "subtitle": "A subtitle", "publish_date": "April 15, 2001",
            "publish_places": [{"name": "New York"}], "url": "https://openlibrary.org/books/OL1M",
            "identifiers": {"isbn_10": ["0306406152"], "isbn_13": ["9780306406157", "9780306406157"]},
            "subjects": [{"name": "Physics"}, {"name": "History"}, {"name": "Physics"}]
        });
        let new = openlibrary(&book, "9780306406157").unwrap().resource;
        assert_eq!(
            new.subtitle.as_ref().map(NonEmptyStr::as_str),
            Some("A subtitle")
        );
        assert_eq!(new.place.as_deref(), Some("New York"));
        assert_eq!(
            new.url.as_deref(),
            Some("https://openlibrary.org/books/OL1M")
        );
        assert_eq!(
            new.isbns
                .iter()
                .map(Identifier::lookup_key)
                .collect::<Vec<_>>(),
            vec!["9780306406157", "0306406152"]
        );
        assert_eq!(new.keywords, vec!["Physics", "History"]);
        assert_eq!(
            new.published.as_ref().map(BibliographicDate::precision),
            Some(DatePrecision::Day)
        );
    }

    #[test]
    fn maps_crossref_bibliographic_details_and_precise_date() {
        let work = serde_json::json!({
            "title": ["Paper"], "type": "journal-article", "container-title": ["The Journal"],
            "short-container-title": ["J. Tests"], "ISSN": ["1234-5679"],
            "page": "S10-S20", "article-number": "e123", "subtitle": ["Extended"],
            "short-title": ["Short paper"], "language": "en",
            "abstract": "<jats:p>Alpha &amp; beta.</jats:p>",
            "subject": ["Mathematics", "mathematics", "Biology"],
            "published": {"date-parts": [[2020, 5, 12]]}
        });
        let fetched = crossref(&work, "10.1000/paper").unwrap();
        let new = fetched.resource;
        assert_eq!(
            new.pages.as_ref().map(ibex_core::Pages::as_str),
            Some("S10-S20")
        );
        assert_eq!(new.article_number.as_deref(), Some("e123"));
        assert_eq!(
            new.subtitle.as_ref().map(NonEmptyStr::as_str),
            Some("Extended")
        );
        assert_eq!(
            new.short_title.as_ref().map(NonEmptyStr::as_str),
            Some("Short paper")
        );
        assert_eq!(new.language.as_deref(), Some("en"));
        assert_eq!(new.abstract_text.as_deref(), Some("Alpha & beta."));
        assert_eq!(new.keywords, vec!["Mathematics", "Biology"]);
        assert_eq!(
            new.published.as_ref().map(BibliographicDate::precision),
            Some(DatePrecision::Day)
        );
        assert_eq!(
            fetched
                .journal
                .unwrap()
                .short_title
                .as_ref()
                .map(NonEmptyStr::as_str),
            Some("J. Tests")
        );
    }

    #[test]
    fn crossref_issued_date_is_used_when_published_is_missing() {
        let work = serde_json::json!({"title":["Book"], "type":"book", "issued":{"date-parts":[[2018, 7]]}});
        assert_eq!(
            crossref(&work, "10.1000/book")
                .unwrap()
                .resource
                .published
                .unwrap()
                .precision(),
            DatePrecision::Month
        );
    }

    #[test]
    fn crossref_editors_are_retained_for_creator_linking() {
        let work = serde_json::json!({
            "title":["Edited book"], "type":"book",
            "author":[{"given":"Ada","family":"Lovelace"}],
            "editor":[{"given":"Grace","family":"Hopper"}]
        });
        let lookup = crossref(&work, "10.1000/book").unwrap();
        assert_eq!(lookup.authors[0].last(), Some("Lovelace"));
        assert_eq!(lookup.editors[0].last(), Some("Hopper"));
    }

    #[test]
    fn doi_article_uses_container_title_and_issn_for_journal_not_article() {
        let work = serde_json::json!({
            "title": ["Paper"], "type": "journal-article",
            "container-title": ["The Journal"], "ISSN": ["1234-5679"]
        });
        let lookup = crossref(&work, "10.1000/paper").unwrap();
        assert!(lookup.resource.issns.is_empty());
        let journal = lookup.journal.unwrap();
        assert_eq!(journal.title.as_str(), "The Journal");
        assert_eq!(journal.issns[0].lookup_key(), "12345679");
    }

    #[test]
    fn doi_journal_article_without_container_title_is_not_saved_without_parent() {
        let work = serde_json::json!({"title":["Paper"],"type":"journal-article"});
        assert!(
            crossref(&work, "10.1000/paper")
                .unwrap_err()
                .contains("journal title")
        );
    }

    #[tokio::test]
    async fn routes_identifiers_to_free_providers() {
        fn serve(
            body: &'static str,
            status: &'static str,
        ) -> (String, std::thread::JoinHandle<String>) {
            use std::io::{Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = format!("http://{}", listener.local_addr().unwrap());
            let handle = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0; 4096];
                let n = stream.read(&mut buffer).unwrap();
                let request = String::from_utf8_lossy(&buffer[..n]).into_owned();
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                request
            });
            (address, handle)
        }
        let (url, handle) = serve(
            r#"{"ISBN:9780306406157":{"title":"Example","authors":[{"name":"Ada Lovelace"}]}}"#,
            "200 OK",
        );
        let book = lookup_at("9780306406157", &url, "http://unused.invalid")
            .await
            .unwrap();
        assert_eq!(book.resource.title.as_str(), "Example");
        assert_eq!(book.authors[0].last(), Some("Lovelace"));
        assert!(
            handle
                .join()
                .unwrap()
                .contains("bibkeys=ISBN%3A9780306406157")
        );

        let (url, handle) = serve(
            r#"{"message":{"title":["Paper"],"type":"journal-article","container-title":["Journal"],"author":[{"given":"Grace","family":"Hopper"}]}}"#,
            "200 OK",
        );
        let paper = lookup_at(
            "https://doi.org/10.1000/example",
            "http://unused.invalid",
            &url,
        )
        .await
        .unwrap();
        assert_eq!(paper.resource.doi.as_deref(), Some("10.1000/example"));
        assert_eq!(paper.authors[0].first(), Some("Grace"));
        assert!(handle.join().unwrap().contains("/10.1000%2Fexample"));

        let (url, handle) = serve("{}", "200 OK");
        assert!(
            lookup_at("9780306406157", &url, "http://unused.invalid")
                .await
                .unwrap_err()
                .contains("No metadata")
        );
        handle.join().unwrap();
    }

    #[test]
    fn refresh_fills_missing_fields_without_replacing_local_data() {
        let original = openlibrary(&serde_json::json!({"title":"My title"}), "9780306406157")
            .unwrap()
            .resource;
        let mut record = Resource::try_new(ibex_core::ResourceId::generate(), original).unwrap();
        record.publisher = Some("Local press".into());
        let fetched = openlibrary(&serde_json::json!({"title":"Remote title", "publishers":[{"name":"Remote press"}], "number_of_pages":42}), "9780306406157").unwrap().resource;
        let merged = fill_gaps(&record, fetched);
        assert_eq!(merged.title.as_str(), "My title");
        assert_eq!(merged.publisher.as_deref(), Some("Local press"));
        assert_eq!(merged.total_pages.as_deref(), Some("42"));
        assert_eq!(merged.isbns.len(), 1);
    }

    #[test]
    fn refresh_adds_new_short_title_and_keywords_without_removing_local_ones() {
        let mut original = empty_resource("My title", ResourceKind::Book).unwrap();
        original.keywords = vec!["Local".into(), "Biology".into()];
        let record = Resource::try_new(ibex_core::ResourceId::generate(), original).unwrap();
        let fetched = crossref(
            &serde_json::json!({
                "title":["Remote"], "type":"book", "short-title":["Short"],
                "subject":["biology", "Chemistry"]
            }),
            "10.1000/book",
        )
        .unwrap()
        .resource;
        let merged = fill_gaps(&record, fetched);
        assert_eq!(merged.title.as_str(), "My title");
        assert_eq!(
            merged.short_title.as_ref().map(NonEmptyStr::as_str),
            Some("Short")
        );
        assert_eq!(merged.keywords, vec!["Local", "Biology", "Chemistry"]);
    }

    #[test]
    fn refresh_keeps_local_authors_and_adds_missing_editor_role() {
        use ibex_core::{CreatorRole, CreatorSpec, PersonId};
        let local_author = PersonId::generate();
        let mut original = empty_resource("My title", ResourceKind::Book).unwrap();
        original.creators = vec![CreatorSpec {
            person_id: local_author,
            role: CreatorRole::Author,
            credited_as: None,
        }];
        let record = Resource::try_new(ibex_core::ResourceId::generate(), original).unwrap();
        let mut fetched = empty_resource("Remote title", ResourceKind::Book).unwrap();
        fetched.creators = vec![
            CreatorSpec {
                person_id: PersonId::generate(),
                role: CreatorRole::Author,
                credited_as: None,
            },
            CreatorSpec {
                person_id: PersonId::generate(),
                role: CreatorRole::Editor,
                credited_as: None,
            },
        ];
        let merged = fill_gaps(&record, fetched);
        assert_eq!(merged.creators.len(), 2);
        assert_eq!(merged.creators[0].person_id, local_author);
        assert_eq!(merged.creators[1].role, CreatorRole::Editor);
    }
}
pub async fn link_creators(
    state: &AppState,
    creators: Vec<(CreatorRole, Name)>,
    new: &mut NewResource,
) -> Result<(), String> {
    for (role, name) in creators {
        let found = state
            .execute(Command::FindPersonsByName(FindPersonsByName {
                name: name.clone(),
            }))
            .await
            .map_err(|e| e.to_string())?;
        let id = match found {
            Outcome::FindPersonsByName(people) if !people.is_empty() => people[0].id,
            Outcome::FindPersonsByName(_) => match state
                .execute(Command::CreatePerson(CreatePerson {
                    name,
                    aliases: vec![],
                }))
                .await
                .map_err(|e| e.to_string())?
            {
                Outcome::CreatePerson(person) => person.id,
                _ => return Err("unexpected outcome from create_person".into()),
            },
            _ => return Err("unexpected outcome from find_persons_by_name".into()),
        };
        if !new
            .creators
            .iter()
            .any(|creator| creator.person_id == id && creator.role == role)
        {
            new.creators.push(CreatorSpec {
                person_id: id,
                role,
                credited_as: None,
            });
        }
    }
    Ok(())
}

/// Resolve a Crossref journal to an existing container or create one before the article.
/// Never replace a manually chosen parent or guess among ambiguous matches.
pub async fn link_journal(
    state: &AppState,
    journal: NewResource,
    article: &mut NewResource,
) -> Result<(), String> {
    if article.parent_id.is_some() || article.kind != ibex_core::ResourceKind::JournalArticle {
        return Ok(());
    }
    // Title filtering would hide conflicting ISSN matches under a different title.
    let query = ResourceQuery {
        sort: None,
        filter: ResourceFilter::by_kind(ResourceKind::Journal),
    };
    let mut journals = Vec::new();
    let mut offset = 0;
    loop {
        let page = match state
            .execute(Command::ListResourcesQueryPage(ListResourcesQueryPage {
                query: query.clone(),
                offset,
                limit: 100,
            }))
            .await
            .map_err(|e| e.to_string())?
        {
            Outcome::ListResourcesQueryPage(page) => page,
            _ => return Err("unexpected outcome from list_resources_query_page".into()),
        };
        for row in page.rows {
            match state
                .execute(Command::GetResource(GetResource { id: row.id }))
                .await
                .map_err(|e| e.to_string())?
            {
                Outcome::GetResource(resource) => journals.push(resource),
                _ => return Err("unexpected outcome from get_resource".into()),
            }
        }
        let Some(next_offset) = page.next_offset else {
            break;
        };
        offset = next_offset;
    }
    let title = journal.title.as_str();
    let by_title: Vec<_> = journals
        .iter()
        .filter(|r| r.title.as_str().trim().eq_ignore_ascii_case(title.trim()))
        .collect();
    let by_issn: Vec<_> = journals
        .iter()
        .filter(|r| {
            r.issns.iter().any(|known| {
                journal
                    .issns
                    .iter()
                    .any(|incoming| known.lookup_key() == incoming.lookup_key())
            })
        })
        .collect();
    let matched = if journal.issns.is_empty() {
        match by_title.as_slice() {
            [] => None,
            [one] => Some(*one),
            _ => {
                return Err(format!(
                    "ambiguous journal title '{title}'; select a parent manually"
                ));
            }
        }
    } else {
        match by_issn.as_slice() {
            [one] if one.title.as_str().trim().eq_ignore_ascii_case(title.trim()) => Some(*one),
            [one] => {
                return Err(format!(
                    "ISSN matches journal '{}' but Crossref calls it '{title}'",
                    one.title.as_str()
                ));
            }
            [] => match by_title.as_slice() {
                [one] if one.issns.is_empty() => Some(*one),
                [] => None,
                _ => {
                    return Err(format!(
                        "ambiguous or conflicting journal '{title}'; select a parent manually"
                    ));
                }
            },
            _ => {
                return Err(format!(
                    "ambiguous ISSN for journal '{title}'; select a parent manually"
                ));
            }
        }
    };
    article.parent_id = Some(if let Some(existing) = matched {
        let mut enriched = existing.to_new();
        let mut changed = false;
        if enriched.issns.is_empty() && !journal.issns.is_empty() {
            enriched.issns = journal.issns;
            changed = true;
        }
        if enriched.short_title.is_none() && journal.short_title.is_some() {
            enriched.short_title = journal.short_title;
            changed = true;
        }
        if enriched.publisher.is_none() && journal.publisher.is_some() {
            enriched.publisher = journal.publisher;
            changed = true;
        }
        if changed {
            state
                .execute(Command::UpdateResource(UpdateResource {
                    id: existing.id,
                    new: enriched,
                }))
                .await
                .map_err(|e| e.to_string())?;
        }
        existing.id
    } else {
        match state
            .execute(Command::CreateResource(CreateResource { new: journal }))
            .await
            .map_err(|e| e.to_string())?
        {
            Outcome::CreateResource(resource) => resource.id,
            _ => return Err("unexpected outcome from create_resource".into()),
        }
    });
    Ok(())
}

/// Resolve an ISBN or DOI and store it, or fill only missing fields on `target`.
pub async fn fetch(
    state: &AppState,
    identifier: &str,
    target: Option<ResourceId>,
) -> Result<Resource, String> {
    fetch_at(
        state,
        identifier,
        target,
        "https://openlibrary.org/api/books",
        "https://api.crossref.org/works",
    )
    .await
}

/// As `fetch`, with injectable provider URLs for deterministic HTTP tests.
pub async fn fetch_at(
    state: &AppState,
    identifier: &str,
    target: Option<ResourceId>,
    books: &str,
    works: &str,
) -> Result<Resource, String> {
    let existing = if let Some(id) = target {
        match state
            .execute(Command::GetResource(GetResource { id }))
            .await
            .map_err(|e| e.to_string())?
        {
            Outcome::GetResource(record) => Some(record),
            _ => return Err("unexpected outcome from get_resource".into()),
        }
    } else {
        None
    };
    let fetched = lookup_at(identifier, books, works).await?;
    let mut new = fetched.resource;
    let has_role = |role| {
        existing.as_ref().is_some_and(|record: &Resource| {
            record.creators.iter().any(|creator| creator.role == role)
        })
    };
    let mut creators = Vec::new();
    if !has_role(CreatorRole::Author) {
        creators.extend(
            fetched
                .authors
                .into_iter()
                .map(|name| (CreatorRole::Author, name)),
        );
    }
    if !has_role(CreatorRole::Editor) {
        creators.extend(
            fetched
                .editors
                .into_iter()
                .map(|name| (CreatorRole::Editor, name)),
        );
    }
    link_creators(state, creators, &mut new).await?;
    if existing.as_ref().is_none_or(|r| r.parent_id.is_none()) {
        if let Some(journal) = fetched.journal {
            link_journal(state, journal, &mut new).await?;
        }
    }
    let outcome = if let Some(record) = existing {
        state
            .execute(Command::UpdateResource(UpdateResource {
                id: record.id,
                new: fill_gaps(&record, new),
            }))
            .await
    } else {
        state
            .execute(Command::CreateResource(CreateResource { new }))
            .await
    }
    .map_err(|e| e.to_string())?;
    match outcome {
        Outcome::CreateResource(resource) | Outcome::UpdateResource(resource) => Ok(resource),
        _ => Err("unexpected outcome from identifier fetch".into()),
    }
}
