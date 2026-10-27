//! Deterministic demo library for in-memory runs.
//!
//! Seeds the store directly rather than through commands: demo data is not
//! user action, and command execution would emit one wide event per row.
//! The same count produces the same library. Call once per fresh store;
//! seeding is not idempotent.

use oeuvre_core::{
    BibliographicDate, CreatorRole, CreatorSpec, Name, NewResource, NonEmptyStr, PersonId,
    ResourceKind, Store, StoreError, UtcMillis,
};

/// Demo library size: large enough that scrolling, filtering, and sorting
/// behave like they do against a real library, while still seeding in well
/// under a second so `--memory` starts promptly.
pub const DEFAULT_DEMO_LIBRARY_SIZE: usize = 5_000;

const DEMO_PEOPLE: [(&str, &str); 8] = [
    ("Ada", "Lovelace"),
    ("Grace", "Hopper"),
    ("Alan", "Turing"),
    ("Barbara", "Liskov"),
    ("Donald", "Knuth"),
    ("Claude", "Shannon"),
    ("Kathleen", "Booth"),
    ("Edsger", "Dijkstra"),
];

/// Populate a fresh store with a demo library: a few people, the containers
/// parented kinds need, nested collections, tags, and `count` resources.
pub async fn seed_demo_library(store: &impl Store, count: usize) -> Result<(), StoreError> {
    let people: Vec<PersonId> = {
        let mut ids = Vec::with_capacity(DEMO_PEOPLE.len());
        for (first, last) in DEMO_PEOPLE {
            let name = Name::try_new(Some(first), Some(last), None::<&str>)
                .map_err(|e| StoreError::invalid(e.to_string()))?;
            let person = store.create_person(name, vec![]).await?;
            ids.push(person.id);
        }
        ids
    };

    // Containers and the parent book must exist before the items that
    // reference them.
    let journal = store
        .create_resource(container(
            ResourceKind::Journal,
            "Journal of Citation Studies",
        ))
        .await?
        .id;
    let podcast = store
        .create_resource(container(ResourceKind::Podcast, "The Bibliographic Hour"))
        .await?
        .id;
    let companion = store
        .create_resource(NewResource {
            kind: ResourceKind::Book,
            title: non_empty("The Reference Companion"),
            creators: vec![CreatorSpec {
                person_id: people[0],
                role: CreatorRole::Author,
                credited_as: None,
            }],
            publisher: Some("Loom Press".into()),
            published: published(1984),
            accessed: None,
            parent_id: None,
            series_id: None,
            series_number: None,
            tag_ids: vec![],
            collection_ids: vec![],
            subtitle: Some(non_empty("A guide to research provenance")),
            title_addon: Some(non_empty("Revised edition")),
            short_title: Some(non_empty("Provenance")),
            place: Some("London".into()),
            volume: None,
            issue: None,
            pages: None,
            total_pages: Some("xii+250".into()),
            article_number: None,
            edition: Some("Second".into()),
            doi: None,
            url: None,
            eprint: None,
            isbns: vec!["978-0-306-40615-7".into()],
            issns: vec![],
            isrns: vec![],
            original_date: published(1960),
            event_date: None,
            language: Some("en".into()),
            document_type: None,
            publication_method: None,
            institution: None,
            organization: None,
            school: None,
            abstract_text: None,
            keywords: vec!["provenance".into(), "research methods".into()],
        })
        .await?
        .id;

    let mut tags = Vec::new();
    for text in ["methods", "review", "to-read", "reference"] {
        tags.push(store.create_tag(non_empty(text)).await?.id);
    }
    let research = store
        .create_collection(non_empty("Research"), None)
        .await?
        .id;
    let methods = store
        .create_collection(non_empty("Methods"), Some(research))
        .await?
        .id;
    let networks = store
        .create_collection(non_empty("Citation networks"), Some(methods))
        .await?
        .id;
    let reading = store
        .create_collection(non_empty("Reading list"), None)
        .await?
        .id;
    let to_read = store
        .create_collection(non_empty("To read"), Some(reading))
        .await?
        .id;
    store
        .create_collection(non_empty("Empty collection"), None)
        .await?;
    let collections = [methods, networks, to_read, research];

    for n in 0..count {
        let year = 1962 + (n % 64) as u16;
        let mut new = match n % 8 {
            0 => {
                let mut creators = authors_for(n, &people);
                // Exercise credited_as on a predictable slice of rows.
                if n % 5 == 0 {
                    creators[0].credited_as = Some(non_empty("A. Lovelace"));
                }
                NewResource {
                    kind: ResourceKind::JournalArticle,
                    title: non_empty(&format!("Citation Networks {n:03}")),
                    creators,
                    publisher: Some("Citation Society Press".into()),
                    published: published(year),
                    accessed: None,
                    parent_id: Some(journal),
                    series_id: None,
                    series_number: None,
                    tag_ids: vec![],
                    collection_ids: vec![],
                    subtitle: None,
                    title_addon: None,
                    short_title: None,
                    place: None,
                    volume: Some((1 + n / 8).to_string()),
                    issue: Some((1 + n % 4).to_string()),
                    pages: Some("1–12".into()),
                    total_pages: None,
                    article_number: Some(format!("e{n:04}")),
                    edition: None,
                    doi: Some(format!("10.5555/demo.citations.{n}")),
                    url: Some(format!("https://example.org/demo/articles/{n}")),
                    eprint: None,
                    isbns: vec![],
                    issns: vec!["1234-5678".into()],
                    isrns: vec![],
                    original_date: None,
                    event_date: published(year),
                    language: Some("en".into()),
                    document_type: None,
                    publication_method: None,
                    institution: None,
                    organization: None,
                    school: None,
                    abstract_text: Some(
                        "A synthetic study of citation networks and research provenance.".into(),
                    ),
                    keywords: vec!["citation networks".into(), "bibliometrics".into()],
                }
            }
            1 => NewResource {
                kind: ResourceKind::Book,
                title: non_empty(&format!("Provenance Studies {n:03}")),
                creators: authors_for(n, &people),
                publisher: Some("Loom Press".into()),
                published: published(year),
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
                tag_ids: vec![],
                collection_ids: vec![],
                subtitle: Some(non_empty("A guide to research provenance")),
                title_addon: Some(non_empty("Revised edition")),
                short_title: Some(non_empty("Provenance")),
                place: Some("London".into()),
                volume: None,
                issue: None,
                pages: None,
                total_pages: Some("xii+250".into()),
                article_number: None,
                edition: Some("Second".into()),
                doi: None,
                url: None,
                eprint: None,
                isbns: vec!["978-0-306-40615-7".into()],
                issns: vec![],
                isrns: vec![],
                original_date: published(1960),
                event_date: None,
                language: Some("en".into()),
                document_type: None,
                publication_method: None,
                institution: None,
                organization: None,
                school: None,
                abstract_text: None,
                keywords: vec!["provenance".into(), "research methods".into()],
            },
            2 => NewResource {
                kind: ResourceKind::BookSection,
                title: non_empty(&format!("Reference Patterns {n:03}")),
                creators: authors_for(n, &people),
                publisher: Some("Loom Press".into()),
                published: published(year),
                accessed: None,
                parent_id: Some(companion),
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
                pages: Some("25–42".into()),
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
                language: Some("en".into()),
                document_type: None,
                publication_method: None,
                institution: None,
                organization: None,
                school: None,
                abstract_text: None,
                keywords: vec![],
            },
            3 => NewResource {
                kind: ResourceKind::Preprint,
                title: non_empty(&format!("Preprint Findings {n:03}")),
                creators: authors_for(n, &people),
                publisher: None,
                published: published(year),
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
                url: Some(format!("https://example.org/demo/preprints/{n}")),
                eprint: Some(format!("demo-preprint-{n:04}")),
                isbns: vec![],
                issns: vec![],
                isrns: vec![],
                original_date: None,
                event_date: None,
                language: None,
                document_type: None,
                publication_method: Some("Preprint repository".into()),
                institution: None,
                organization: None,
                school: None,
                abstract_text: None,
                keywords: vec!["open research".into()],
            },
            4 => NewResource {
                kind: ResourceKind::PodcastEpisode,
                title: non_empty(&format!("Interview with a Cataloger {n:03}")),
                creators: vec![CreatorSpec {
                    person_id: people[n % people.len()],
                    role: CreatorRole::Host,
                    credited_as: None,
                }],
                publisher: None,
                published: published(year),
                accessed: None,
                parent_id: Some(podcast),
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
            },
            5 => NewResource {
                kind: ResourceKind::Report,
                title: non_empty(&format!("Field Report {n:03}")),
                creators: authors_for(n, &people),
                publisher: Some("Bureau of Standards".into()),
                published: published(year),
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
                isrns: vec![format!("DEMO-REPORT-{n:04}")],
                original_date: None,
                event_date: None,
                language: None,
                document_type: Some("Technical report".into()),
                publication_method: None,
                institution: Some("Institute for Citation Studies".into()),
                organization: Some("Citation Standards Committee".into()),
                school: Some("School of Information Science".into()),
                abstract_text: None,
                keywords: vec![],
            },
            6 => NewResource {
                kind: ResourceKind::Dataset,
                title: non_empty(&format!("Open Shelf Records {n:03}")),
                creators: authors_for(n, &people),
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
            },
            _ => NewResource {
                kind: ResourceKind::Webpage,
                title: non_empty(&format!("Margin Notes {n:03}")),
                creators: authors_for(n, &people),
                publisher: None,
                published: None,
                accessed: published(2025),
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
                url: Some(format!("https://example.org/demo/margin-notes/{n}")),
                eprint: None,
                isbns: vec![],
                issns: vec![],
                isrns: vec![],
                original_date: None,
                event_date: None,
                language: Some("en".into()),
                document_type: None,
                publication_method: Some("Online".into()),
                institution: None,
                organization: None,
                school: None,
                abstract_text: None,
                keywords: vec![],
            },
        };
        // Keep some rows unassigned to exercise empty membership and tag filters.
        if n % 5 != 4 {
            new.tag_ids = vec![tags[n % tags.len()]];
            if n % 3 == 0 {
                new.tag_ids.push(tags[(n + 1) % tags.len()]);
            }
            new.collection_ids = vec![collections[n % collections.len()]];
        }
        store.create_resource(new).await?;
    }
    Ok(())
}

fn container(kind: ResourceKind, title: &str) -> NewResource {
    NewResource {
        kind,
        title: non_empty(title),
        creators: vec![],
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
    }
}

fn authors_for(n: usize, people: &[PersonId]) -> Vec<CreatorSpec> {
    (0..(n % 3) + 1)
        .map(|k| CreatorSpec {
            person_id: people[(n + k) % people.len()],
            role: CreatorRole::Author,
            credited_as: None,
        })
        .collect()
}

fn published(year: u16) -> Option<BibliographicDate> {
    Some(
        BibliographicDate::try_new(
            // Real year instants, not one shared anchor: published-range
            // filters bracket whole years, so the seed must actually spread
            // across the years its labels claim.
            UtcMillis::jan_1_utc(i64::from(year)).expect("seed years are civil"),
            Some(non_empty(&year.to_string())),
        )
        .expect("year instants are valid"),
    )
}

fn non_empty(s: &str) -> NonEmptyStr {
    NonEmptyStr::try_new(s).expect("demo literals are non-empty")
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use crate::testing::MemoryStore;
    use oeuvre_core::{NonEmptyStr, ResourceKind};

    #[tokio::test]
    async fn demo_library_includes_bibliographic_metadata() {
        let store = MemoryStore::new();
        seed_demo_library(&store, 8).await.unwrap();
        let rows = store.list_resources().await.unwrap();
        let article = rows
            .iter()
            .find(|r| r.title.as_str() == "Citation Networks 000")
            .unwrap();
        let article = store.get_resource(article.id).await.unwrap();
        assert_eq!(article.doi.as_deref(), Some("10.5555/demo.citations.0"));
        assert_eq!(article.volume.as_deref(), Some("1"));
        assert_eq!(article.issue.as_deref(), Some("1"));
        assert_eq!(article.pages.as_deref(), Some("1–12"));
        assert_eq!(article.keywords, ["citation networks", "bibliometrics"]);
        assert!(!article.tag_ids.is_empty());
        assert!(article.abstract_text.is_some());
        let book = rows
            .iter()
            .find(|r| r.title.as_str() == "Provenance Studies 001")
            .unwrap();
        let book = store.get_resource(book.id).await.unwrap();
        assert_eq!(book.isbns, ["978-0-306-40615-7"]);
        assert_eq!(book.edition.as_deref(), Some("Second"));
        assert_eq!(book.place.as_deref(), Some("London"));
        assert!(book.original_date.is_some());
        let preprint = rows
            .iter()
            .find(|r| r.kind == ResourceKind::Preprint)
            .unwrap();
        let preprint = store.get_resource(preprint.id).await.unwrap();
        assert_eq!(preprint.eprint.as_deref(), Some("demo-preprint-0003"));
        let report = rows
            .iter()
            .find(|r| r.kind == ResourceKind::Report)
            .unwrap();
        let report = store.get_resource(report.id).await.unwrap();
        assert_eq!(report.document_type.as_deref(), Some("Technical report"));
        assert_eq!(report.isrns, ["DEMO-REPORT-0005"]);
        let webpage = rows
            .iter()
            .find(|r| r.kind == ResourceKind::Webpage)
            .unwrap();
        let webpage = store.get_resource(webpage.id).await.unwrap();
        assert_eq!(
            webpage.url.as_deref(),
            Some("https://example.org/demo/margin-notes/7")
        );
    }

    #[tokio::test]
    async fn demo_library_has_nested_collections_and_tagged_resources() {
        let store = MemoryStore::new();
        seed_demo_library(&store, 12).await.unwrap();

        let tags = store.list_tags().await.unwrap();
        assert_eq!(tags.len(), 4);
        assert!(tags.iter().any(|tag| tag.text.as_str() == "methods"));
        let forest = store.list_collection_forest().await.unwrap();
        assert_eq!(forest.len(), 3);
        let research = forest
            .iter()
            .find(|node| node.collection.name.as_str() == "Research")
            .unwrap();
        let methods = research
            .children
            .iter()
            .find(|node| node.collection.name.as_str() == "Methods")
            .unwrap();
        assert_eq!(
            methods.children[0].collection.name.as_str(),
            "Citation networks"
        );
        assert!(
            forest
                .iter()
                .any(|node| node.collection.name.as_str() == "Empty collection"
                    && node.children.is_empty())
        );

        let rows = store.list_resources().await.unwrap();
        let first = rows
            .iter()
            .find(|row| row.title.as_str() == "Citation Networks 000")
            .unwrap();
        let resource = store.get_resource(first.id).await.unwrap();
        assert!(!resource.tag_ids.is_empty());
        assert_eq!(resource.collection_ids, vec![methods.collection.id]);
        let unassigned = rows
            .iter()
            .find(|row| row.title.as_str() == "Interview with a Cataloger 004")
            .unwrap();
        let resource = store.get_resource(unassigned.id).await.unwrap();
        assert!(resource.tag_ids.is_empty());
        assert!(resource.collection_ids.is_empty());
    }

    #[tokio::test]
    async fn demo_library_shows_expected_rows() {
        let store = MemoryStore::new();
        seed_demo_library(&store, 12).await.unwrap();

        let rows = store.list_resources().await.unwrap();
        // 12 demo items plus the journal, podcast, and parent book that
        // container kinds need.
        assert_eq!(rows.len(), 12 + 3);

        let articles: Vec<_> = rows
            .iter()
            .filter(|row| row.kind == ResourceKind::JournalArticle)
            .collect();
        assert_eq!(articles.len(), 2);
        assert!(articles.iter().all(|row| {
            row.container_title.as_ref().map(NonEmptyStr::as_str)
                == Some("Journal of Citation Studies")
        }));
        assert!(articles.iter().all(|row| !row.creators_label.is_empty()));

        let episodes: Vec<_> = rows
            .iter()
            .filter(|row| row.kind == ResourceKind::PodcastEpisode)
            .collect();
        assert_eq!(episodes.len(), 1);
        assert!(episodes.iter().all(|row| {
            row.container_title.as_ref().map(NonEmptyStr::as_str) == Some("The Bibliographic Hour")
        }));

        let first = rows
            .iter()
            .find(|row| row.title.as_str() == "Citation Networks 000")
            .expect("seeded article present");
        assert_eq!(first.creators_label, "A. Lovelace");
        assert_eq!(
            first.published_label.as_ref().map(NonEmptyStr::as_str),
            Some("1962")
        );

        // Years are fuzzy display labels, so they stay exact without
        // calendar math.
        assert!(
            rows.iter()
                .filter_map(|row| row.published_label.as_ref())
                .all(|label| label.as_str().parse::<u16>().is_ok())
        );
    }
}
