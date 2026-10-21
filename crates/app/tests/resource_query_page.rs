#![cfg(feature = "testing")]

//! Store-port tests for `list_resources_query_page`: the order contract a
//! shell page may rely on, independent of which adapter serves it.

use oeuvre_app::command::{CreatePerson, CreateResource, ListResourcesQueryPage};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, CommandError, FieldValue, Outcome, Status, Surface};
use oeuvre_core::{
    BibliographicDate, CreatorRole, CreatorSpec, Name, NewResource, NonEmptyStr, PersonId,
    PublishedRange, ResourceFilter, ResourceId, ResourceKind, ResourceQuery, ResourceSort,
    SortDirection, Store, UtcMillis,
};

fn sorted(field: ResourceSort, direction: SortDirection) -> ResourceQuery {
    ResourceQuery::sorted(field, direction)
}

fn newest_first() -> ResourceQuery {
    ResourceQuery::newest_first()
}

/// An unsorted listing narrowed to titles containing `needle`.
fn titled(needle: &str) -> ResourceQuery {
    ResourceQuery {
        sort: None,
        filter: ResourceFilter {
            title: Some(NonEmptyStr::try_new(needle).expect("needle non-empty")),
            creator: None,
            published: None,
        },
    }
}

/// An unsorted listing narrowed to one person's works: any creator edge
/// to the person counts, sole or shared.
fn by_creator(person: PersonId) -> ResourceQuery {
    ResourceQuery {
        sort: None,
        filter: ResourceFilter {
            title: None,
            creator: Some(person),
            published: None,
        },
    }
}

/// An unsorted listing narrowed to a published window over raw instants;
/// `None` bounds stay open, and both-open windows narrow nothing.
fn ranged(from: Option<i64>, to: Option<i64>) -> ResourceQuery {
    ResourceQuery {
        sort: None,
        filter: ResourceFilter {
            title: None,
            creator: None,
            published: Some(PublishedRange {
                from: from.map(UtcMillis::from_unix_ms),
                to: to.map(UtcMillis::from_unix_ms),
            }),
        },
    }
}

fn book_with(authors: Vec<(PersonId, Option<&str>)>, title: &str) -> NewResource {
    let mut new = book(None, title);
    new.creators = authors
        .into_iter()
        .map(|(person_id, credited_as)| CreatorSpec {
            person_id,
            role: CreatorRole::Author,
            credited_as: credited_as.map(|s| NonEmptyStr::try_new(s).expect("credited non-empty")),
        })
        .collect();
    new
}

async fn person(store: &MemoryStore, first: Option<&str>, last: Option<&str>) -> PersonId {
    store
        .create_person(
            Name::try_new(first, last, None::<&str>).expect("demo names are valid"),
            vec![],
        )
        .await
        .expect("create person")
        .id
}

fn book(author: Option<PersonId>, title: &str) -> NewResource {
    resource(ResourceKind::Book, author, title)
}

/// A creatorless container: kinds that may stand without creators.
fn container(title: &str) -> NewResource {
    resource(ResourceKind::Journal, None, title)
}

fn resource(kind: ResourceKind, author: Option<PersonId>, title: &str) -> NewResource {
    NewResource {
        kind,
        title: NonEmptyStr::try_new(title).expect("titles are non-empty"),
        creators: author
            .map(|person_id| {
                vec![CreatorSpec {
                    person_id,
                    role: CreatorRole::Author,
                    credited_as: None,
                }]
            })
            .unwrap_or_default(),
        publisher: None,
        published: None,
        accessed: None,
        parent_id: None,
        series_id: None,
        series_number: None,
    }
}

fn titles(page: &oeuvre_core::SortedResourcePage) -> Vec<String> {
    page.rows
        .iter()
        .map(|row| row.title.as_str().to_owned())
        .collect()
}

/// A date whose label is deliberately free-form: only the instant orders.
fn published_with(instant: i64, label: &str) -> Option<BibliographicDate> {
    Some(
        BibliographicDate::try_new(
            UtcMillis::from_unix_ms(instant),
            Some(NonEmptyStr::try_new(label).expect("labels are non-empty")),
        )
        .expect("instant valid"),
    )
}

#[tokio::test]
async fn title_ascending_walks_case_insensitive_pages() {
    let store = MemoryStore::new();
    let grace = person(&store, Some("Grace"), Some("Hopper")).await;

    // Mixed case pins the ordering intent: "Beta" must land between "alpha"
    // and "charlie", not before "alpha" the way a case-sensitive scan would.
    for title in ["alpha waves", "Beta modes", "charlie nodes"] {
        store
            .create_resource(book(Some(grace), title))
            .await
            .expect("create resource");
    }
    // A creatorless container still has a title, so it joins the order.
    store
        .create_resource(container("Journal of Nothing"))
        .await
        .expect("create container");

    let first = store
        .list_resources_query_page(
            &sorted(ResourceSort::Title, SortDirection::Ascending),
            0,
            2,
        )
        .await
        .expect("first title page");
    assert_eq!(titles(&first), ["alpha waves", "Beta modes"]);
    assert_eq!(first.next_offset, Some(2));

    let second = store
        .list_resources_query_page(
            &sorted(ResourceSort::Title, SortDirection::Ascending),
            2,
            2,
        )
        .await
        .expect("second title page");
    assert_eq!(titles(&second), ["charlie nodes", "Journal of Nothing"]);
    assert_eq!(second.next_offset, None);
}

#[tokio::test]
async fn unsorted_query_walks_newest_first_by_id() {
    let store = MemoryStore::new();
    let grace = person(&store, Some("Grace"), Some("Hopper")).await;

    let mut created: Vec<ResourceId> = Vec::new();
    for title in ["alpha waves", "Beta modes", "charlie nodes"] {
        let id = store
            .create_resource(book(Some(grace), title))
            .await
            .expect("create resource")
            .id;
        created.push(id);
    }
    // `sort: None` is the default listing order — newest first by id — and
    // ids settle same-millisecond ties, so the expectation follows the ids.
    created.sort_by(|a, b| b.as_ulid().cmp(&a.as_ulid()));

    let first = store
        .list_resources_query_page(&newest_first(), 0, 2)
        .await
        .expect("first unsorted page");
    assert_eq!(
        first.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        &created[..2]
    );
    assert_eq!(first.next_offset, Some(2));

    let second = store
        .list_resources_query_page(&newest_first(), 2, 2)
        .await
        .expect("second unsorted page");
    assert_eq!(
        second.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        &created[2..]
    );
    assert_eq!(second.next_offset, None);
}

#[tokio::test]
async fn primary_creator_ascending_keys_on_first_creators_family_name() {
    let store = MemoryStore::new();
    let hopper = person(&store, Some("Grace"), Some("Hopper")).await;
    let lovelace = person(&store, Some("Ada"), Some("Lovelace")).await;
    let augustine = person(&store, None, Some("Augustine")).await;

    // Two authors pin "primary": the label shows both, but the order key
    // must come from the first creator only (Lovelace, not Hopper).
    let coauthored = (
        "Citation basics",
        book_with(vec![(lovelace, None), (hopper, None)], "Citation basics"),
    );
    let by_hopper = (
        "Compiler origins",
        book_with(vec![(hopper, None)], "Compiler origins"),
    );
    let by_augustine = (
        "Confessions",
        book_with(vec![(augustine, None)], "Confessions"),
    );
    // A credited string keys on its last word, so this ties the coauthored
    // book on "lovelace" and exercises the newest-first tiebreak.
    let credited = (
        "Annotated algorithms",
        book_with(
            vec![(lovelace, Some("A. Lovelace"))],
            "Annotated algorithms",
        ),
    );

    let mut created: Vec<(ResourceId, &str)> = Vec::new();
    for (title, new) in [coauthored, by_hopper, by_augustine, credited] {
        let id = store
            .create_resource(new)
            .await
            .expect("create resource")
            .id;
        created.push((id, title));
    }
    // The container has no creators, so it carries no key and lands last.
    let container_id = store
        .create_resource(container("Journal of Nothing"))
        .await
        .expect("create container")
        .id;

    // Creation can tie within one ULID millisecond, so derive the expected
    // order of the "lovelace" pair from the ids the contract ties on.
    let lovelace_ids: Vec<ResourceId> = {
        let mut pair = vec![created[0].0, created[3].0];
        pair.sort_by_key(|id| std::cmp::Reverse(id.as_ulid()));
        pair
    };
    let title_of = |id: ResourceId| -> String {
        if id == container_id {
            return "Journal of Nothing".to_owned();
        }
        created
            .iter()
            .find(|(row_id, _)| *row_id == id)
            .map(|(_, title)| (*title).to_owned())
            .expect("id came from the created rows")
    };
    let expected: Vec<String> = vec![
        "Confessions".to_owned(),
        "Compiler origins".to_owned(),
        title_of(lovelace_ids[0]),
        title_of(lovelace_ids[1]),
        "Journal of Nothing".to_owned(),
    ];

    let first = store
        .list_resources_query_page(
            &sorted(ResourceSort::PrimaryCreator, SortDirection::Ascending),
            0,
            3,
        )
        .await
        .expect("first primary-creator page");
    assert_eq!(titles(&first), expected[..3]);
    assert_eq!(first.next_offset, Some(3));

    let second = store
        .list_resources_query_page(
            &sorted(ResourceSort::PrimaryCreator, SortDirection::Ascending),
            3,
            3,
        )
        .await
        .expect("second primary-creator page");
    assert_eq!(titles(&second), expected[3..]);
    assert_eq!(second.next_offset, None);
}

#[tokio::test]
async fn published_ascending_orders_by_instant_not_label() {
    let store = MemoryStore::new();
    let grace = person(&store, Some("Grace"), Some("Hopper")).await;

    // Labels are display strings, not chronological keys: "Zzz era" carries
    // the oldest instant and would sort last under string order. The page
    // must follow the instants.
    for (title, label, instant) in [
        ("Modern notes", "1999", 1_900_000_000_000),
        ("Middle era", "B-movie era", 1_500_000_000_000),
        ("Old chronicle", "Zzz era", 1_000_000_000_000),
    ] {
        let mut new = book(Some(grace), title);
        new.published = published_with(instant, label);
        store.create_resource(new).await.expect("create resource");
    }
    // The container has no date, so it lands last in ascending order.
    store
        .create_resource(container("Journal of Nothing"))
        .await
        .expect("create container");

    let page = store
        .list_resources_query_page(
            &sorted(ResourceSort::Published, SortDirection::Ascending),
            0,
            10,
        )
        .await
        .expect("published page");
    assert_eq!(
        titles(&page),
        [
            "Old chronicle",
            "Middle era",
            "Modern notes",
            "Journal of Nothing"
        ]
    );
    assert_eq!(page.next_offset, None);
}

#[tokio::test]
async fn descending_reverses_keys_and_moves_no_data_rows_to_the_front() {
    let store = MemoryStore::new();
    let hopper = person(&store, Some("Grace"), Some("Hopper")).await;
    let lovelace = person(&store, Some("Ada"), Some("Lovelace")).await;

    let lovelace_book = book_with(vec![(lovelace, None)], "Citation basics");
    let hopper_book = book_with(vec![(hopper, None)], "Compiler origins");
    for new in [lovelace_book, hopper_book] {
        store.create_resource(new).await.expect("create resource");
    }
    // The container has no creators: last ascending, first descending.
    store
        .create_resource(container("Journal of Nothing"))
        .await
        .expect("create container");

    let page = store
        .list_resources_query_page(
            &sorted(ResourceSort::PrimaryCreator, SortDirection::Descending),
            0,
            10,
        )
        .await
        .expect("descending primary-creator page");
    assert_eq!(
        titles(&page),
        ["Journal of Nothing", "Citation basics", "Compiler origins"]
    );
    assert_eq!(page.next_offset, None);
}

#[tokio::test]
async fn offset_past_the_end_yields_an_empty_page() {
    let store = MemoryStore::new();
    let grace = person(&store, Some("Grace"), Some("Hopper")).await;
    store
        .create_resource(book(Some(grace), "Only book"))
        .await
        .expect("create resource");

    let page = store
        .list_resources_query_page(
            &sorted(ResourceSort::Title, SortDirection::Ascending),
            100,
            10,
        )
        .await
        .expect("page past the end");
    assert!(page.rows.is_empty());
    assert_eq!(page.next_offset, None);
}

#[tokio::test]
async fn title_filter_matches_case_insensitive_substring() {
    let store = MemoryStore::new();
    let grace = person(&store, Some("Grace"), Some("Hopper")).await;

    for title in ["alpha waves", "Beta modes", "GAMMA nodes"] {
        store
            .create_resource(book(Some(grace), title))
            .await
            .expect("create resource");
    }
    store
        .create_resource(container("Journal of Nothing"))
        .await
        .expect("create container");

    // "MOD" must reach inside "Beta modes" regardless of case.
    let page = store
        .list_resources_query_page(&titled("MOD"), 0, 10)
        .await
        .expect("filtered page");
    assert_eq!(titles(&page), ["Beta modes"]);
    assert_eq!(page.next_offset, None);

    // A needle nothing carries narrows to an empty page with a closed end.
    let none = store
        .list_resources_query_page(&titled("zzz-not-there"), 0, 10)
        .await
        .expect("no-match page");
    assert!(none.rows.is_empty());
    assert_eq!(none.next_offset, None);
}

#[tokio::test]
async fn title_filter_keeps_the_sort_order_within_matches() {
    let store = MemoryStore::new();
    let grace = person(&store, Some("Grace"), Some("Hopper")).await;

    for title in ["alpha waves", "Beta modes", "GAMMA nodes"] {
        store
            .create_resource(book(Some(grace), title))
            .await
            .expect("create resource");
    }

    // "DE" matches "Beta modes" and "GAMMA nodes"; the sort orders exactly
    // those matches, case-insensitively, and the window still pages.
    let query = ResourceQuery {
        sort: Some((ResourceSort::Title, SortDirection::Ascending)),
        filter: ResourceFilter {
            title: Some(NonEmptyStr::try_new("DE").expect("needle non-empty")),
            creator: None,
            published: None,
        },
    };
    let first = store
        .list_resources_query_page(&query, 0, 1)
        .await
        .expect("filtered sorted page");
    assert_eq!(titles(&first), ["Beta modes"]);
    assert_eq!(first.next_offset, Some(1));

    let second = store
        .list_resources_query_page(&query, 1, 1)
        .await
        .expect("second filtered sorted page");
    assert_eq!(titles(&second), ["GAMMA nodes"]);
    assert_eq!(second.next_offset, None);
}

// --- Command seam --------------------------------------------------------

#[tokio::test]
async fn creator_filter_matches_only_that_persons_works() {
    let store = MemoryStore::new();
    let ada = person(&store, Some("Ada"), Some("Lovelace")).await;
    let grace = person(&store, Some("Grace"), Some("Hopper")).await;

    let alpha = store
        .create_resource(book_with(vec![(ada, None)], "Alpha Waves"))
        .await
        .expect("create resource")
        .id;
    store
        .create_resource(book_with(vec![(grace, None)], "Beta Modes"))
        .await
        .expect("create resource");
    let shared = store
        .create_resource(book_with(
            vec![(grace, None), (ada, None)],
            "Shared Authorship",
        ))
        .await
        .expect("create resource")
        .id;
    // Creatorless containers never match a creator predicate.
    store
        .create_resource(container("Journal of Nothing"))
        .await
        .expect("create container");

    // Co-authorship counts as much as sole authorship, and the matches
    // keep the newest-first order, so the expectation follows the ids.
    let mut expected = vec![shared, alpha];
    expected.sort_by(|a, b| b.as_ulid().cmp(&a.as_ulid()));
    let page = store
        .list_resources_query_page(&by_creator(ada), 0, 10)
        .await
        .expect("creator page");
    assert_eq!(
        page.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        expected
    );
    assert_eq!(page.next_offset, None);

    // An unknown person matches nothing rather than erroring: filters
    // are predicates, not lookups.
    let empty = store
        .list_resources_query_page(&by_creator(PersonId::generate()), 0, 10)
        .await
        .expect("unknown creator page");
    assert!(empty.rows.is_empty());
    assert_eq!(empty.next_offset, None);
}

#[tokio::test]
async fn creator_filter_keeps_the_sort_order_within_matches() {
    let store = MemoryStore::new();
    let ada = person(&store, Some("Ada"), Some("Lovelace")).await;
    let grace = person(&store, Some("Grace"), Some("Hopper")).await;

    store
        .create_resource(book_with(vec![(ada, None)], "Gamma Nodes"))
        .await
        .expect("create resource");
    store
        .create_resource(book_with(vec![(grace, None)], "Beta Modes"))
        .await
        .expect("create resource");
    store
        .create_resource(book_with(vec![(ada, None)], "Alpha Waves"))
        .await
        .expect("create resource");

    let query = ResourceQuery {
        sort: Some((ResourceSort::Title, SortDirection::Ascending)),
        filter: ResourceFilter {
            title: None,
            creator: Some(ada),
            published: None,
        },
    };
    let first = store
        .list_resources_query_page(&query, 0, 1)
        .await
        .expect("first creator-sorted page");
    assert_eq!(titles(&first), ["Alpha Waves"]);
    assert_eq!(first.next_offset, Some(1));

    let second = store
        .list_resources_query_page(&query, 1, 1)
        .await
        .expect("second creator-sorted page");
    assert_eq!(titles(&second), ["Gamma Nodes"]);
    assert_eq!(second.next_offset, None);
}

#[tokio::test]
async fn published_range_filters_by_instant_not_label() {
    let store = MemoryStore::new();
    let grace = person(&store, Some("Grace"), Some("Hopper")).await;

    // Labels deliberately disagree with instants: only the instant
    // decides, never the fuzzy display.
    let mut alpha = book_with(vec![(grace, None)], "alpha waves");
    alpha.published = published_with(1_000, "1999");
    let alpha_id = store
        .create_resource(alpha)
        .await
        .expect("create resource")
        .id;
    let mut beta = book_with(vec![(grace, None)], "Beta modes");
    beta.published = published_with(2_000, "1900");
    let beta_id = store
        .create_resource(beta)
        .await
        .expect("create resource")
        .id;
    let mut charlie = book_with(vec![(grace, None)], "charlie nodes");
    charlie.published = published_with(3_000, "2000");
    let charlie_id = store
        .create_resource(charlie)
        .await
        .expect("create resource")
        .id;
    // Undated rows only match a window that bounds nothing.
    store
        .create_resource(book_with(vec![(grace, None)], "Delta notes"))
        .await
        .expect("create resource");

    // Bounded: only the instants between the bounds; the newest-first
    // expectation follows the ids, since ids settle same-millisecond ties.
    let mut bounded_expected = vec![beta_id, charlie_id];
    bounded_expected.sort_by(|a, b| b.as_ulid().cmp(&a.as_ulid()));
    let bounded = store
        .list_resources_query_page(&ranged(Some(1_500), Some(3_500)), 0, 10)
        .await
        .expect("bounded page");
    assert_eq!(
        bounded.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        bounded_expected
    );
    assert_eq!(bounded.next_offset, None);

    // Open "from": everything before the bound, never the undated row.
    let mut open_expected = vec![alpha_id, beta_id];
    open_expected.sort_by(|a, b| b.as_ulid().cmp(&a.as_ulid()));
    let open_from = store
        .list_resources_query_page(&ranged(None, Some(2_500)), 0, 10)
        .await
        .expect("open-from page");
    assert_eq!(
        open_from.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        open_expected
    );

    // Open "to": everything from the bound on.
    let open_to = store
        .list_resources_query_page(&ranged(Some(2_500), None), 0, 10)
        .await
        .expect("open-to page");
    assert_eq!(titles(&open_to), ["charlie nodes"]);

    // Both open: the window narrows nothing, undated included.
    let unset = store
        .list_resources_query_page(&ranged(None, None), 0, 10)
        .await
        .expect("unset window page");
    assert_eq!(unset.rows.len(), 4);
}

#[tokio::test]
async fn published_range_keeps_sort_order_within_matches() {
    let store = MemoryStore::new();
    let grace = person(&store, Some("Grace"), Some("Hopper")).await;

    let mut gamma = book_with(vec![(grace, None)], "GAMMA nodes");
    gamma.published = published_with(3_000, "Spring 2001");
    store.create_resource(gamma).await.expect("create resource");
    let mut beta = book_with(vec![(grace, None)], "Beta modes");
    beta.published = published_with(2_000, "1900");
    store.create_resource(beta).await.expect("create resource");
    let mut alpha = book_with(vec![(grace, None)], "alpha waves");
    alpha.published = published_with(1_000, "1999");
    store.create_resource(alpha).await.expect("create resource");
    let mut delta = book_with(vec![(grace, None)], "Delta notes");
    delta.published = published_with(4_000, "2002");
    store.create_resource(delta).await.expect("create resource");

    let query = ResourceQuery {
        sort: Some((ResourceSort::Title, SortDirection::Ascending)),
        filter: ResourceFilter {
            title: None,
            creator: None,
            published: Some(PublishedRange {
                from: Some(UtcMillis::from_unix_ms(1_500)),
                to: Some(UtcMillis::from_unix_ms(3_500)),
            }),
        },
    };
    let first = store
        .list_resources_query_page(&query, 0, 1)
        .await
        .expect("first range-sorted page");
    assert_eq!(titles(&first), ["Beta modes"]);
    assert_eq!(first.next_offset, Some(1));

    let second = store
        .list_resources_query_page(&query, 1, 1)
        .await
        .expect("second range-sorted page");
    assert_eq!(titles(&second), ["GAMMA nodes"]);
    assert_eq!(second.next_offset, None);
}

#[tokio::test]
async fn filters_and_together() {
    let store = MemoryStore::new();
    let ada = person(&store, Some("Ada"), Some("Lovelace")).await;
    let grace = person(&store, Some("Grace"), Some("Hopper")).await;

    // Every predicate alone matches several rows; only their intersection
    // names one.
    let mut waves = book_with(vec![(ada, None)], "alpha waves");
    waves.published = published_with(1_000, "1999");
    store.create_resource(waves).await.expect("create resource");
    let mut nodes = book_with(vec![(ada, None)], "alpha nodes");
    nodes.published = published_with(2_000, "2001");
    store.create_resource(nodes).await.expect("create resource");
    let mut notes = book_with(vec![(grace, None)], "alpha notes");
    notes.published = published_with(2_000, "2000");
    store.create_resource(notes).await.expect("create resource");
    let mut beta = book_with(vec![(grace, None)], "beta waves");
    beta.published = published_with(1_000, "1998");
    store.create_resource(beta).await.expect("create resource");

    let query = ResourceQuery {
        sort: None,
        filter: ResourceFilter {
            title: Some(NonEmptyStr::try_new("ALPHA").expect("needle non-empty")),
            creator: Some(ada),
            published: Some(PublishedRange {
                from: Some(UtcMillis::from_unix_ms(1_500)),
                to: Some(UtcMillis::from_unix_ms(3_500)),
            }),
        },
    };
    let page = store
        .list_resources_query_page(&query, 0, 10)
        .await
        .expect("anded page");
    assert_eq!(titles(&page), ["alpha nodes"]);
    assert_eq!(page.next_offset, None);
}

fn state_with_sink(sink: &MemorySink) -> AppState {
    AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build()
}

async fn seed_author(state: &AppState) -> PersonId {
    let name = Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap();
    let Outcome::CreatePerson(author) = state
        .execute(Command::CreatePerson(CreatePerson {
            name,
            aliases: vec![],
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreatePerson");
    };
    author.id
}

async fn create_book(state: &AppState, author_id: PersonId, title: &str) {
    let Outcome::CreateResource(_) = state
        .execute(Command::CreateResource(CreateResource {
            new: NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new(title).expect("titles are non-empty"),
                creators: vec![CreatorSpec {
                    person_id: author_id,
                    role: CreatorRole::Author,
                    credited_as: None,
                }],
                publisher: None,
                published: None,
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
            },
        }))
        .await
        .unwrap()
    else {
        panic!("expected CreateResource");
    };
}

fn query_page_call(query: ResourceQuery, offset: u64, limit: u32) -> Command {
    Command::ListResourcesQueryPage(ListResourcesQueryPage {
        query,
        offset,
        limit,
    })
}

#[tokio::test]
async fn query_pages_walk_the_library_through_the_command() {
    let sink = MemorySink::new();
    let state = state_with_sink(&sink);

    let author_id = seed_author(&state).await;
    for title in ["alpha waves", "Beta modes", "charlie nodes"] {
        create_book(&state, author_id, title).await;
    }
    let _ = sink.take();

    let Outcome::ListResourcesQueryPage(first) = state
        .execute(query_page_call(
            sorted(ResourceSort::Title, SortDirection::Ascending),
            0,
            2,
        ))
        .await
        .unwrap()
    else {
        panic!("expected ListResourcesQueryPage");
    };
    assert_eq!(titles(&first), ["alpha waves", "Beta modes"]);
    assert_eq!(first.next_offset, Some(2));

    let Outcome::ListResourcesQueryPage(second) = state
        .execute(query_page_call(
            sorted(ResourceSort::Title, SortDirection::Ascending),
            2,
            2,
        ))
        .await
        .unwrap()
    else {
        panic!("expected ListResourcesQueryPage");
    };
    assert_eq!(titles(&second), ["charlie nodes"]);
    assert_eq!(second.next_offset, None);
}

#[tokio::test]
async fn filtered_query_page_command_emits_filter_event() {
    let sink = MemorySink::new();
    let state = state_with_sink(&sink);

    let author_id = seed_author(&state).await;
    for title in ["alpha waves", "Beta modes", "charlie nodes"] {
        create_book(&state, author_id, title).await;
    }
    let _ = sink.take();

    let Outcome::ListResourcesQueryPage(page) = state
        .execute(query_page_call(titled("MOD"), 0, 10))
        .await
        .unwrap()
    else {
        panic!("expected ListResourcesQueryPage");
    };
    assert_eq!(titles(&page), ["Beta modes"]);

    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.get("resource.filter_title"), Some(&FieldValue::from("MOD")));

    // A creator-filtered listing names the person by id.
    let Outcome::ListResourcesQueryPage(creator_page) = state
        .execute(query_page_call(by_creator(author_id), 0, 10))
        .await
        .unwrap()
    else {
        panic!("expected ListResourcesQueryPage");
    };
    assert_eq!(creator_page.rows.len(), 3);
    let mut got = titles(&creator_page);
    got.sort();
    assert_eq!(got, ["Beta modes", "alpha waves", "charlie nodes"]);

    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(
        ev.get("resource.filter_creator"),
        Some(&FieldValue::from(author_id.to_string()))
    );

    // A published window names the bounds it carries; open bounds stay
    // out of the event.
    let Outcome::ListResourcesQueryPage(range_page) = state
        .execute(query_page_call(ranged(Some(1), Some(2)), 0, 10))
        .await
        .unwrap()
    else {
        panic!("expected ListResourcesQueryPage");
    };
    // The seeded books are undated, so the window matches nothing; the
    // event still carries both bounds.
    assert!(range_page.rows.is_empty());

    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(
        ev.get("resource.filter_published_from"),
        Some(&FieldValue::from(1i64))
    );
    assert_eq!(
        ev.get("resource.filter_published_to"),
        Some(&FieldValue::from(2i64))
    );

    // All three filters land on one listing and one event: every field
    // the query carries shows up together. The seeded books are undated,
    // so the active window excludes them and the narrowing itself is the
    // demonstration.
    let Outcome::ListResourcesQueryPage(and_page) = state
        .execute(query_page_call(
            ResourceQuery {
                sort: None,
                filter: ResourceFilter {
                    title: Some(NonEmptyStr::try_new("MODE").expect("needle non-empty")),
                    creator: Some(author_id),
                    published: Some(PublishedRange {
                        from: Some(UtcMillis::from_unix_ms(1)),
                        to: None,
                    }),
                },
            },
            0,
            10,
        ))
        .await
        .unwrap()
    else {
        panic!("expected ListResourcesQueryPage");
    };
    assert!(and_page.rows.is_empty());

    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.get("resource.filter_title"), Some(&FieldValue::from("MODE")));
    assert_eq!(
        ev.get("resource.filter_creator"),
        Some(&FieldValue::from(author_id.to_string()))
    );
    assert_eq!(
        ev.get("resource.filter_published_from"),
        Some(&FieldValue::from(1i64))
    );
    // An open to-bound never appears.
    assert_eq!(ev.get("resource.filter_published_to"), None);

    // An unfiltered listing carries no filter field.
    let Outcome::ListResourcesQueryPage(_) = state
        .execute(query_page_call(newest_first(), 0, 10))
        .await
        .unwrap()
    else {
        panic!("expected ListResourcesQueryPage");
    };
    let events = sink.take();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].get("resource.filter_title"), None);
}

#[tokio::test]
async fn query_page_command_emits_wide_event_fields() {
    let sink = MemorySink::new();
    let state = state_with_sink(&sink);

    let author_id = seed_author(&state).await;
    for title in ["alpha waves", "Beta modes", "charlie nodes"] {
        create_book(&state, author_id, title).await;
    }
    let _ = sink.take();

    let Outcome::ListResourcesQueryPage(_) = state
        .execute(query_page_call(
            sorted(ResourceSort::PrimaryCreator, SortDirection::Descending),
            2,
            2,
        ))
        .await
        .unwrap()
    else {
        panic!("expected ListResourcesQueryPage");
    };

    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.command(), "list_resources_query_page");
    assert_eq!(ev.status(), Status::Ok);
    assert_eq!(
        ev.get("resource.sort_field"),
        Some(&FieldValue::from("primaryCreator"))
    );
    assert_eq!(
        ev.get("resource.sort_direction"),
        Some(&FieldValue::from("descending"))
    );
    assert_eq!(ev.get("resource.page_offset"), Some(&FieldValue::from(2u64)));
    assert_eq!(
        ev.get("resource.page_limit"),
        Some(&FieldValue::from(2u32))
    );

    // The default order carries no sort field, so the event stays honest
    // about what drove the listing.
    let Outcome::ListResourcesQueryPage(_) = state
        .execute(query_page_call(newest_first(), 0, 2))
        .await
        .unwrap()
    else {
        panic!("expected ListResourcesQueryPage");
    };
    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.get("resource.sort_field"), None);
    assert_eq!(ev.get("resource.sort_direction"), None);
    assert_eq!(ev.get("resource.page_offset"), Some(&FieldValue::from(0u64)));
    assert_eq!(ev.get("resource.page_limit"), Some(&FieldValue::from(2u32)));
}

#[tokio::test]
async fn query_page_zero_limit_is_invalid() {
    let sink = MemorySink::new();
    let state = state_with_sink(&sink);

    let err = state
        .execute(query_page_call(
            sorted(ResourceSort::Title, SortDirection::Ascending),
            0,
            0,
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Invalid(_)));

    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.status(), Status::Error);
    assert_eq!(ev.get("error.kind"), Some(&FieldValue::from("invalid")));
    // Input dimensions land on the event even though the run failed early.
    assert_eq!(ev.get("resource.sort_field"), Some(&FieldValue::from("title")));
}
