#![cfg(feature = "testing")]

//! Store-port tests for `list_resources_sorted_page`: the order contract a
//! shell page may rely on, independent of which adapter serves it.

use oeuvre_app::testing::MemoryStore;
use oeuvre_core::{
    BibliographicDate, CreatorRole, CreatorSpec, Name, NewResource, NonEmptyStr, PersonId,
    ResourceId, ResourceKind, ResourceSort, SortDirection, Store, UtcMillis,
};

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
        .list_resources_sorted_page(ResourceSort::Title, SortDirection::Ascending, 0, 2)
        .await
        .expect("first title page");
    assert_eq!(titles(&first), ["alpha waves", "Beta modes"]);
    assert_eq!(first.next_offset, Some(2));

    let second = store
        .list_resources_sorted_page(ResourceSort::Title, SortDirection::Ascending, 2, 2)
        .await
        .expect("second title page");
    assert_eq!(titles(&second), ["charlie nodes", "Journal of Nothing"]);
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
        .list_resources_sorted_page(ResourceSort::PrimaryCreator, SortDirection::Ascending, 0, 3)
        .await
        .expect("first primary-creator page");
    assert_eq!(titles(&first), expected[..3]);
    assert_eq!(first.next_offset, Some(3));

    let second = store
        .list_resources_sorted_page(ResourceSort::PrimaryCreator, SortDirection::Ascending, 3, 3)
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
        .list_resources_sorted_page(ResourceSort::Published, SortDirection::Ascending, 0, 10)
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
        .list_resources_sorted_page(
            ResourceSort::PrimaryCreator,
            SortDirection::Descending,
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
        .list_resources_sorted_page(ResourceSort::Title, SortDirection::Ascending, 100, 10)
        .await
        .expect("page past the end");
    assert!(page.rows.is_empty());
    assert_eq!(page.next_offset, None);
}
