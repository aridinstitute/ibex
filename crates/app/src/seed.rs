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

/// Demo library size: enough rows to force several scroll pages at 50 rows
/// per page.
pub const DEFAULT_DEMO_LIBRARY_SIZE: usize = 200;

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
/// parented kinds need, and `count` resources cycling kinds and creators.
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
        .create_resource(container(ResourceKind::Journal, "Journal of Citation Studies"))
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
        })
        .await?
        .id;

    for n in 0..count {
        let year = 1962 + (n % 64) as u16;
        let new = match n % 8 {
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
            },
        };
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
    Some(BibliographicDate::try_new(
        // Real year instants, not one shared anchor: published-range
        // filters bracket whole years, so the seed must actually spread
        // across the years its labels claim.
        UtcMillis::jan_1_utc(i64::from(year)).expect("seed years are civil"),
        Some(non_empty(&year.to_string())),
    )
    .expect("year instants are valid"))
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
            row.container_title.as_ref().map(NonEmptyStr::as_str)
                == Some("The Bibliographic Hour")
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
        assert!(rows
            .iter()
            .filter_map(|row| row.published_label.as_ref())
            .all(|label| label.as_str().parse::<u16>().is_ok()));
    }
}
