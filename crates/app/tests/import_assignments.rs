#![cfg(feature = "testing")]

//! Batch collection/tag assignment through the shared import save path.

use ibex_app::command::{
    CreateCollection, CreateTag, GetResource, ImportBiblatex, ListCollectionForest, ListResources,
    ListTags, SetResourceFavorite,
};
use ibex_app::testing::{MemorySink, MemoryStore};
use ibex_app::{AppState, Command, CommandError, Outcome, Surface};
use ibex_core::{
    CollectionId, CollectionTreeNode, EntryChoice, ExistingIdOrName, ImportAssignments,
    ImportChoices, ImportReport, NonEmptyStr, Resource, ResourceId, TagId, import_codes,
};

fn app_with(store: MemoryStore) -> AppState {
    AppState::builder(store)
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build()
}

fn app() -> AppState {
    app_with(MemoryStore::new())
}

fn name(s: &str) -> NonEmptyStr {
    NonEmptyStr::try_new(s).unwrap()
}

fn named<Id>(s: &str) -> ExistingIdOrName<Id> {
    ExistingIdOrName::Name { name: name(s) }
}

fn choices(
    collections: Vec<ExistingIdOrName<CollectionId>>,
    tags: Vec<ExistingIdOrName<TagId>>,
) -> ImportChoices {
    ImportChoices {
        assignments: ImportAssignments { collections, tags },
        ..Default::default()
    }
}

async fn try_import(
    state: &AppState,
    source: &str,
    choices: ImportChoices,
) -> Result<ImportReport, CommandError> {
    match state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: source.into(),
            choices,
        }))
        .await?
    {
        Outcome::ImportBiblatex(report) => Ok(report),
        other => panic!("unexpected outcome {other:?}"),
    }
}

async fn import(state: &AppState, source: &str, choices: ImportChoices) -> ImportReport {
    try_import(state, source, choices).await.unwrap()
}

async fn resource(state: &AppState, id: ResourceId) -> Resource {
    let Outcome::GetResource(r) = state
        .execute(Command::GetResource(GetResource { id }))
        .await
        .unwrap()
    else {
        panic!("get");
    };
    r
}

async fn create_tag(state: &AppState, text: &str) -> TagId {
    let Outcome::CreateTag(tag) = state
        .execute(Command::CreateTag(CreateTag { text: name(text) }))
        .await
        .unwrap()
    else {
        panic!("tag");
    };
    tag.id
}

async fn create_collection(state: &AppState, n: &str) -> CollectionId {
    let Outcome::CreateCollection(c) = state
        .execute(Command::CreateCollection(CreateCollection {
            name: name(n),
            parent_id: None,
        }))
        .await
        .unwrap()
    else {
        panic!("collection");
    };
    c.id
}

async fn tag_texts(state: &AppState) -> Vec<String> {
    let Outcome::ListTags(tags) = state.execute(Command::ListTags(ListTags)).await.unwrap() else {
        panic!("tags");
    };
    tags.iter().map(|t| t.text.as_str().to_owned()).collect()
}

async fn collection_names(state: &AppState) -> Vec<String> {
    let Outcome::ListCollectionForest(forest) = state
        .execute(Command::ListCollectionForest(ListCollectionForest))
        .await
        .unwrap()
    else {
        panic!("forest");
    };
    fn walk(nodes: &[CollectionTreeNode], out: &mut Vec<String>) {
        for n in nodes {
            out.push(n.collection.name.as_str().to_owned());
            walk(&n.children, out);
        }
    }
    let mut out = Vec::new();
    walk(&forest, &mut out);
    out.sort();
    out
}

async fn resource_count(state: &AppState) -> usize {
    let Outcome::ListResources(rows) = state
        .execute(Command::ListResources(ListResources))
        .await
        .unwrap()
    else {
        panic!("list");
    };
    rows.len()
}

const TWO_BOOKS: &str = r#"
    @book{a, title={First}, author={Ada, Lovelace}, year={1843}}
    @book{b, title={Second}, author={Hopper, Grace}, year={1952}}
"#;

#[tokio::test]
async fn every_imported_entry_gets_all_targets_with_existing_and_new_names() {
    let state = app();
    let review = create_tag(&state, "review").await;
    let methods = create_collection(&state, "Methods").await;

    let report = import(
        &state,
        TWO_BOOKS,
        choices(
            vec![
                named("Research"),
                ExistingIdOrName::Existing { id: methods },
                ExistingIdOrName::Existing { id: methods },
            ],
            vec![named("review"), named("2026"), named("2026")],
        ),
    )
    .await;
    assert_eq!(report.imported.len(), 2, "{report:?}");

    let tags = tag_texts(&state).await;
    assert_eq!(tags, vec!["2026", "review"], "no duplicate tags");
    assert_eq!(collection_names(&state).await, vec!["Methods", "Research"]);

    for entry in &report.imported {
        let r = resource(&state, entry.resource_id).await;
        assert_eq!(r.tag_ids.len(), 2, "{r:?}");
        assert!(r.tag_ids.contains(&review));
        assert_eq!(r.collection_ids.len(), 2, "{r:?}");
        assert!(r.collection_ids.contains(&methods));
    }

    let created: Vec<_> = report
        .assignments
        .tags
        .iter()
        .map(|t| (t.name.as_str(), t.created))
        .collect();
    assert_eq!(created, vec![("review", false), ("2026", true)]);
    let created: Vec<_> = report
        .assignments
        .collections
        .iter()
        .map(|c| (c.name.as_str(), c.created))
        .collect();
    assert_eq!(created, vec![("Research", true), ("Methods", false)]);
}

#[tokio::test]
async fn generated_containers_are_not_batch_targets() {
    let state = app();
    let report = import(
        &state,
        "@article{art, title={Paper}, author={Ada, Lovelace}, journaltitle={Journal of Tests}, year={2020}}",
        choices(vec![named("Research")], vec![named("review")]),
    )
    .await;
    assert_eq!(report.imported.len(), 1, "{report:?}");
    let article = resource(&state, report.imported[0].resource_id).await;
    assert_eq!(article.tag_ids.len(), 1);
    assert_eq!(article.collection_ids.len(), 1);
    let parent = article.parent_id.expect("journal container created");
    let journal = resource(&state, parent).await;
    assert!(journal.tag_ids.is_empty(), "{journal:?}");
    assert!(journal.collection_ids.is_empty(), "{journal:?}");
}

#[tokio::test]
async fn retries_add_to_reused_resources_without_duplicates_or_field_changes() {
    let state = app();
    let first = import(&state, TWO_BOOKS, choices(vec![], vec![named("first")])).await;
    assert_eq!(first.imported.len(), 2);
    let id = first.imported[0].resource_id;
    state
        .execute(Command::SetResourceFavorite(SetResourceFavorite {
            id,
            favorite: true,
        }))
        .await
        .unwrap();
    let before = resource(&state, id).await;

    for _ in 0..2 {
        let retry = import(
            &state,
            TWO_BOOKS,
            choices(
                vec![named("Research")],
                vec![named("first"), named("second")],
            ),
        )
        .await;
        assert!(retry.imported.is_empty(), "{retry:?}");
        assert_eq!(retry.reused.len(), 2, "{retry:?}");
    }

    let after = resource(&state, id).await;
    assert_eq!(after.tag_ids.len(), 2, "existing kept, new added once");
    assert!(after.tag_ids.contains(&before.tag_ids[0]));
    assert_eq!(after.collection_ids.len(), 1);
    assert!(after.favorite, "favorite preserved");
    assert_eq!(after.title, before.title);
    assert_eq!(after.creators, before.creators);
    assert_eq!(after.date_edited, before.date_edited);
    assert_eq!(tag_texts(&state).await, vec!["first", "second"]);
    assert_eq!(collection_names(&state).await, vec!["Research"]);
    assert_eq!(resource_count(&state).await, 2);
}

#[tokio::test]
async fn explicit_reuse_choice_receives_targets() {
    let state = app();
    let first = import(
        &state,
        "@book{one, title={Notes}, author={Ada, Lovelace}, doi={10.1000/notes}}",
        ImportChoices::default(),
    )
    .await;
    let existing = first.imported[0].resource_id;
    let mut c = choices(vec![], vec![named("review")]);
    c.entries.insert(
        "other".into(),
        EntryChoice::Reuse {
            resource_id: existing,
        },
    );
    let report = import(
        &state,
        "@book{other, title={Notes elsewhere}, author={Ada, Lovelace}, doi={10.1000/notes}}",
        c,
    )
    .await;
    assert_eq!(report.reused.len(), 1, "{report:?}");
    assert_eq!(resource(&state, existing).await.tag_ids.len(), 1);
}

#[tokio::test]
async fn skipped_and_conflicted_entries_stay_unchanged() {
    let state = app();
    let first = import(
        &state,
        "@book{one, title={Notes}, author={Ada, Lovelace}, doi={10.1000/notes}}",
        ImportChoices::default(),
    )
    .await;
    let existing = first.imported[0].resource_id;

    let mut c = choices(vec![named("Research")], vec![named("review")]);
    c.entries.insert("skipme".into(), EntryChoice::Skip);
    let report = import(
        &state,
        r#"
        @book{dup, title={Notes elsewhere}, author={Ada, Lovelace}, doi={10.1000/notes}}
        @book{skipme, title={Skipped}, author={Ada, Lovelace}}
        @book{fresh, title={Fresh}, author={Ada, Lovelace}}
        "#,
        c,
    )
    .await;
    assert_eq!(report.imported.len(), 1, "{report:?}");
    assert!(
        report
            .conflicted
            .iter()
            .any(|i| i.code.as_deref() == Some(import_codes::PUBLICATION_DUPLICATE))
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|i| i.entry_key.as_deref() == Some("skipme"))
    );
    let untouched = resource(&state, existing).await;
    assert!(untouched.tag_ids.is_empty());
    assert!(untouched.collection_ids.is_empty());
    assert_eq!(resource_count(&state).await, 2);
}

#[tokio::test]
async fn targets_are_not_created_when_no_entry_receives_them() {
    let state = app();
    let mut c = choices(vec![named("Research")], vec![named("review")]);
    c.entries.insert("a".into(), EntryChoice::Skip);
    c.entries.insert("b".into(), EntryChoice::Skip);
    let report = import(&state, TWO_BOOKS, c).await;
    assert!(report.imported.is_empty());
    assert!(tag_texts(&state).await.is_empty());
    assert!(collection_names(&state).await.is_empty());
    assert!(report.assignments.tags.is_empty());
}

#[tokio::test]
async fn unknown_ids_reject_the_import_before_any_write() {
    let state = app();
    let err = try_import(
        &state,
        TWO_BOOKS,
        choices(
            vec![named("Research")],
            vec![
                named("review"),
                ExistingIdOrName::Existing {
                    id: TagId::generate(),
                },
            ],
        ),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, CommandError::NotFound(_)), "{err:?}");

    let err = try_import(
        &state,
        TWO_BOOKS,
        choices(
            vec![ExistingIdOrName::Existing {
                id: CollectionId::generate(),
            }],
            vec![],
        ),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, CommandError::NotFound(_)), "{err:?}");

    assert_eq!(resource_count(&state).await, 0);
    assert!(tag_texts(&state).await.is_empty());
    assert!(collection_names(&state).await.is_empty());
}

#[tokio::test]
async fn ambiguous_collection_name_is_rejected_with_id_guidance() {
    let state = app();
    let one = create_collection(&state, "Dup").await;
    create_collection(&state, "Dup").await;
    let err = try_import(
        &state,
        TWO_BOOKS,
        choices(vec![named("Dup")], vec![named("review")]),
    )
    .await
    .unwrap_err();
    let CommandError::Invalid(message) = err else {
        panic!("expected invalid, got {err:?}");
    };
    assert!(message.contains("ambiguous collection name"), "{message}");
    assert!(message.contains(&one.to_string()), "{message}");
    assert!(message.contains("by id"), "{message}");
    assert_eq!(resource_count(&state).await, 0);
    assert!(tag_texts(&state).await.is_empty());

    // An explicit id still works.
    let report = import(
        &state,
        TWO_BOOKS,
        choices(vec![ExistingIdOrName::Existing { id: one }], vec![]),
    )
    .await;
    assert_eq!(report.imported.len(), 2);
}

#[tokio::test]
async fn names_are_exact_and_reuse_the_shared_trim_rule() {
    let state = app();
    create_tag(&state, "Review").await;
    let report = import(&state, TWO_BOOKS, choices(vec![], vec![named("  review ")])).await;
    assert_eq!(report.imported.len(), 2);
    assert_eq!(tag_texts(&state).await, vec!["Review", "review"]);
}

#[tokio::test]
async fn storage_failure_reports_truthfully_and_retry_completes() {
    let store = MemoryStore::new();
    let state = app_with(store.clone());
    let source = r#"
        @book{a, title={First}, author={Ada, Lovelace}}
        @book{b, title={Second}, author={Ada, Lovelace}}
        @book{c, title={Third}, author={Ada, Lovelace}}
    "#;
    let assign = || choices(vec![named("Research")], vec![named("review")]);

    store.fail_imported_create_at(1);
    let report = import(&state, source, assign()).await;
    assert_eq!(report.imported.len(), 1, "{report:?}");
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.not_attempted.len(), 1);
    let committed = resource(&state, report.imported[0].resource_id).await;
    assert_eq!(committed.tag_ids.len(), 1);
    assert_eq!(committed.collection_ids.len(), 1);
    assert_eq!(report.assignments.tags.len(), 1);

    // Reused-entry label write fails: that entry fails, the rest wait.
    store.clear_import_fail();
    store.fail_add_resource_labels_at(0);
    let report = import(&state, source, assign()).await;
    assert!(report.reused.is_empty(), "{report:?}");
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.not_attempted.len(), 2);

    store.clear_import_fail();
    let report = import(&state, source, assign()).await;
    assert_eq!(report.reused.len(), 1, "{report:?}");
    assert_eq!(report.imported.len(), 2, "{report:?}");
    for entry in report.reused.iter().chain(&report.imported) {
        let r = resource(&state, entry.resource_id).await;
        assert_eq!(r.tag_ids.len(), 1);
        assert_eq!(r.collection_ids.len(), 1);
    }
    assert_eq!(tag_texts(&state).await, vec!["review"]);
    assert_eq!(collection_names(&state).await, vec!["Research"]);
}

#[test]
fn choices_without_assignments_still_deserialize() {
    let parsed: ImportChoices = serde_json::from_str(r#"{"entries":{},"people":{}}"#).unwrap();
    assert!(parsed.assignments.is_empty());
    let round = ImportChoices {
        assignments: ImportAssignments {
            collections: vec![named("Research")],
            tags: vec![ExistingIdOrName::Existing {
                id: TagId::generate(),
            }],
        },
        ..Default::default()
    };
    let json = serde_json::to_string(&round).unwrap();
    assert_eq!(serde_json::from_str::<ImportChoices>(&json).unwrap(), round);
}
