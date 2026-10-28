use oeuvre_adapter_surreal::SurrealStore;
use oeuvre_app::command::ListResourceGridPage;
use oeuvre_app::testing::{MemorySink, StubStore};
use oeuvre_app::{AppState, Command, CommandError, FieldValue, Outcome, Status, Surface};
use oeuvre_core::{
    BibliographicDate, CreatorRole, CreatorSpec, Name, NewResource, NonEmptyStr, PublishedRange,
    ResourceFilter, ResourceGridParent, ResourceId, ResourceKind, ResourceQuery, ResourceSort,
    SortDirection, Store, UtcMillis,
};

fn nes(s: &str) -> NonEmptyStr {
    NonEmptyStr::try_new(s).unwrap()
}

fn bare(kind: ResourceKind, title: &str, parent_id: Option<ResourceId>) -> NewResource {
    NewResource {
        kind,
        title: nes(title),
        creators: vec![],
        publisher: None,
        published: None,
        accessed: None,
        parent_id,
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
    }
}

#[tokio::test]
async fn grid_combined_filters_keep_ancestors_and_emit_one_event() {
    let store = SurrealStore::open_memory().await.unwrap();
    let journal = store
        .create_resource(bare(ResourceKind::Journal, "Context", None))
        .await
        .unwrap();
    let issue = store
        .create_resource(bare(ResourceKind::Issue, "Issue", Some(journal.id)))
        .await
        .unwrap();
    let author = store
        .create_person(
            Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
            vec![],
        )
        .await
        .unwrap();
    let tag = store.create_tag(nes("selected")).await.unwrap();
    let collection = store.create_collection(nes("Parent"), None).await.unwrap();
    let descendant = store
        .create_collection(nes("Child"), Some(collection.id))
        .await
        .unwrap();
    let mut article = bare(ResourceKind::JournalArticle, "Needle paper", Some(issue.id));
    article.publisher = Some("Needle Press".into());
    article.creators = vec![CreatorSpec {
        person_id: author.id,
        role: CreatorRole::Author,
        credited_as: None,
    }];
    article.tag_ids = vec![tag.id];
    article.collection_ids = vec![descendant.id];
    article.published =
        Some(BibliographicDate::try_new(UtcMillis::from_unix_ms(2000), None).unwrap());
    let article = store.create_resource(article).await.unwrap();
    store.set_resource_favorite(article.id, true).await.unwrap();
    let sink = MemorySink::new();
    let state = AppState::builder(store)
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();
    let query = ResourceQuery {
        sort: Some((ResourceSort::Title, SortDirection::Ascending)),
        filter: ResourceFilter {
            search: Some(nes("press")),
            title: Some(nes("needle")),
            creator: Some(author.id),
            kind: Some(ResourceKind::JournalArticle),
            favorite: Some(true),
            tag: Some(tag.id),
            collection: Some(collection.id),
            published: Some(PublishedRange {
                from: Some(UtcMillis::from_unix_ms(1000)),
                to: Some(UtcMillis::from_unix_ms(3000)),
            }),
        },
    };
    for (parent, expected, has_children) in [
        (ResourceGridParent::Roots, journal.id, true),
        (ResourceGridParent::Children(journal.id), issue.id, true),
        (ResourceGridParent::Children(issue.id), article.id, false),
    ] {
        let Outcome::ResourceGridPage(page) = state
            .execute(Command::ListResourceGridPage(ListResourceGridPage {
                parent,
                query: query.clone(),
                offset: 0,
                limit: 1,
            }))
            .await
            .unwrap()
        else {
            panic!("expected grid page");
        };
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].summary.id, expected);
        assert_eq!(page.rows[0].has_children, has_children);
        assert_eq!(page.next_offset, None);
        let events = sink.take();
        assert_eq!(events.len(), 1);
        let ev = &events[0];
        assert_eq!(ev.command(), "list_resource_grid_page");
        assert_eq!(ev.status(), Status::Ok);
        assert_eq!(ev.get("resource.page_limit"), Some(&FieldValue::from(1u32)));
        assert_eq!(
            ev.get("resource.filter_search"),
            Some(&FieldValue::from("press"))
        );
        assert_eq!(ev.get("resource.grid_rows"), Some(&FieldValue::from(1u64)));
        assert_eq!(
            ev.get("resource.grid_has_more"),
            Some(&FieldValue::from(false))
        );
        if let ResourceGridParent::Children(id) = parent {
            assert_eq!(
                ev.get("resource.parent_id"),
                Some(&FieldValue::from(id.to_string()))
            );
        }
    }
}

#[tokio::test]
async fn grid_invalid_limit_and_store_failure_emit_one_error_event() {
    let sink = MemorySink::new();
    let state = AppState::builder(StubStore::healthy())
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();
    for limit in [0, 1] {
        let error = state
            .execute(Command::ListResourceGridPage(ListResourceGridPage {
                parent: ResourceGridParent::Roots,
                query: ResourceQuery::newest_first(),
                offset: 0,
                limit,
            }))
            .await
            .unwrap_err();
        if limit == 0 {
            assert!(matches!(error, CommandError::Invalid(_)));
        } else {
            assert!(matches!(error, CommandError::Store(_)));
        }
        let events = sink.take();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status(), Status::Error);
        assert_eq!(
            events[0].get("resource.grid_parent"),
            Some(&FieldValue::from("roots"))
        );
        assert_eq!(
            events[0].get("resource.page_limit"),
            Some(&FieldValue::from(limit))
        );
    }
}

#[tokio::test]
async fn grid_siblings_keep_existing_sort_missing_value_and_tie_semantics() {
    let store = SurrealStore::open_memory().await.unwrap();
    let journal = store
        .create_resource(bare(ResourceKind::Journal, "Journal", None))
        .await
        .unwrap();
    let mut ids = Vec::new();
    for (title, publisher, instant) in [
        ("beta", Some("Alpha Press"), Some(2000)),
        ("Alpha", Some("zeta Press"), Some(1000)),
        ("Gamma", None, None),
    ] {
        let mut issue = bare(ResourceKind::Issue, title, Some(journal.id));
        issue.publisher = publisher.map(str::to_owned);
        issue.published = instant
            .map(|ms| BibliographicDate::try_new(UtcMillis::from_unix_ms(ms), None).unwrap());
        ids.push(store.create_resource(issue).await.unwrap().id);
    }
    let parent = ResourceGridParent::Children(journal.id);
    for (field, direction, expected) in [
        (
            ResourceSort::Title,
            SortDirection::Ascending,
            vec![ids[1], ids[0], ids[2]],
        ),
        (
            ResourceSort::Title,
            SortDirection::Descending,
            vec![ids[2], ids[0], ids[1]],
        ),
        (
            ResourceSort::Publisher,
            SortDirection::Ascending,
            vec![ids[0], ids[1], ids[2]],
        ),
        (
            ResourceSort::Publisher,
            SortDirection::Descending,
            vec![ids[2], ids[1], ids[0]],
        ),
        (
            ResourceSort::Published,
            SortDirection::Ascending,
            vec![ids[1], ids[0], ids[2]],
        ),
        (
            ResourceSort::Published,
            SortDirection::Descending,
            vec![ids[2], ids[0], ids[1]],
        ),
    ] {
        let query = ResourceQuery::sorted(field, direction);
        let first = store
            .list_resource_grid_page(parent, &query, 0, 2)
            .await
            .unwrap();
        let second = store
            .list_resource_grid_page(parent, &query, 2, 2)
            .await
            .unwrap();
        assert_eq!(first.next_offset, Some(2));
        assert_eq!(second.next_offset, None);
        let got = first
            .rows
            .into_iter()
            .chain(second.rows)
            .map(|row| row.summary.id)
            .collect::<Vec<_>>();
        assert_eq!(got, expected, "{field:?} {direction:?}");
    }
    let mut newest = ids.clone();
    newest.sort_by_key(|id| std::cmp::Reverse(id.to_string()));
    for sort in [
        None,
        Some((ResourceSort::PrimaryCreator, SortDirection::Ascending)),
        Some((ResourceSort::PrimaryCreator, SortDirection::Descending)),
    ] {
        let query = ResourceQuery {
            sort,
            filter: ResourceFilter::none(),
        };
        let page = store
            .list_resource_grid_page(parent, &query, 0, 10)
            .await
            .unwrap();
        assert_eq!(
            page.rows
                .iter()
                .map(|row| row.summary.id)
                .collect::<Vec<_>>(),
            newest
        );
    }
    assert!(
        store
            .list_resource_grid_page(parent, &ResourceQuery::newest_first(), 0, 0)
            .await
            .unwrap_err()
            .is_invalid()
    );
}

#[tokio::test]
async fn expansion_returns_all_children_and_marks_only_non_leaves() {
    let store = SurrealStore::open_memory().await.unwrap();
    let mut new = bare(ResourceKind::Journal, "Journal", None);
    new.publisher = Some("Inherited Press".into());
    let root = store.create_resource(new).await.unwrap();
    let mut expected = Vec::new();
    for ix in 0..64 {
        let title = format!("Issue {ix:02}");
        let issue = store
            .create_resource(bare(ResourceKind::Issue, &title, Some(root.id)))
            .await
            .unwrap();
        expected.push((issue.id, ix % 2 == 0));
        if ix % 2 == 0 {
            store
                .create_resource(bare(
                    ResourceKind::JournalArticle,
                    "Article",
                    Some(issue.id),
                ))
                .await
                .unwrap();
        }
    }
    let page = store
        .list_resource_grid_page(
            ResourceGridParent::Children(root.id),
            &ResourceQuery {
                sort: Some((ResourceSort::Title, SortDirection::Ascending)),
                filter: ResourceFilter::none(),
            },
            0,
            u32::MAX,
        )
        .await
        .unwrap();
    assert_eq!(page.rows.len(), 64);
    assert!(page.next_offset.is_none());
    assert_eq!(
        page.rows
            .iter()
            .map(|row| (row.summary.id, row.has_children))
            .collect::<Vec<_>>(),
        expected
    );
    assert!(
        page.rows
            .iter()
            .all(|row| row.summary.publisher.as_deref() == Some("Inherited Press"))
    );
}

/// Run with `cargo test -p oeuvre-adapter-surreal --release --test resource_grid grid_loading_timings -- --ignored --nocapture`.
/// Measures fully materialized pages, excluding fixture creation and store opening.
#[tokio::test]
#[ignore = "opt-in loading performance comparison"]
async fn grid_loading_timings() {
    use std::time::Instant;

    let store = SurrealStore::open_memory().await.unwrap();
    let mut groups = Vec::new();
    // Many roots exercise initial loading; a wide group exercises all-child expansion.
    for ix in 0..1_000 {
        let mut new = bare(ResourceKind::Journal, &format!("Journal {ix:04}"), None);
        new.publisher = Some("Inherited Press".into());
        groups.push(store.create_resource(new).await.unwrap().id);
    }
    let parent = groups[0];
    for ix in 0..1_000 {
        let child = store
            .create_resource(bare(
                ResourceKind::Issue,
                &format!("Chapter {ix:04}"),
                Some(parent),
            ))
            .await
            .unwrap();
        // Many non-leaves also exercise has_children membership checks.
        if ix % 2 == 0 {
            store
                .create_resource(bare(
                    ResourceKind::JournalArticle,
                    "Nested chapter",
                    Some(child.id),
                ))
                .await
                .unwrap();
        }
    }

    for (name, scope, limit, expected) in [
        ("roots_50", ResourceGridParent::Roots, 50, 50),
        ("roots_200", ResourceGridParent::Roots, 200, 200),
        (
            "children_all",
            ResourceGridParent::Children(parent),
            u32::MAX,
            1_000,
        ),
    ] {
        for (sort_name, sort) in [
            ("newest", None),
            (
                "title",
                Some((ResourceSort::Title, SortDirection::Ascending)),
            ),
        ] {
            let query = ResourceQuery {
                sort,
                filter: ResourceFilter::none(),
            };
            let mut elapsed = Vec::new();
            for sample in 0..18 {
                let started = Instant::now();
                let page = store
                    .list_resource_grid_page(scope, &query, 0, limit)
                    .await
                    .unwrap();
                let duration = started.elapsed();
                assert_eq!(page.rows.len(), expected);
                if scope == ResourceGridParent::Children(parent) {
                    assert!(
                        page.next_offset.is_none(),
                        "expansion must return every child"
                    );
                    assert_eq!(page.rows.iter().filter(|row| row.has_children).count(), 500);
                    assert!(
                        page.rows
                            .iter()
                            .all(|row| row.summary.publisher.as_deref() == Some("Inherited Press"))
                    );
                } else {
                    assert!(page.next_offset.is_some());
                }
                std::hint::black_box(page);
                if sample >= 3 {
                    elapsed.push(duration);
                }
            }
            elapsed.sort();
            eprintln!(
                "grid_loading/{name}/{sort_name}: median={:?} min={:?} max={:?}",
                elapsed[7], elapsed[0], elapsed[14]
            );
        }
    }
}
