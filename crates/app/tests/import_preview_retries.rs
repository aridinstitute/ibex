#![cfg(feature = "testing")]

//! Task 06: preview, conflict choices, retry idempotency, partial reports.

use oeuvre_app::command::{
    CreatePerson, DeletePerson, GetResource, ImportBiblatex, ListResources, PreviewBiblatex,
};
use oeuvre_app::testing::{MemorySink, MemoryStore};
use oeuvre_app::{AppState, Command, Outcome, Surface};
use oeuvre_core::{EntryChoice, ImportChoices, PersonChoice, import_codes};

fn app() -> AppState {
    AppState::builder(MemoryStore::new())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build()
}

async fn import(state: &AppState, source: &str) -> oeuvre_core::ImportReport {
    let Outcome::ImportBiblatex(report) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: source.into(),
            choices: Default::default(),
        }))
        .await
        .unwrap()
    else {
        panic!("import");
    };
    report
}

async fn import_with_choices(
    state: &AppState,
    source: &str,
    choices: ImportChoices,
) -> oeuvre_core::ImportReport {
    let Outcome::ImportBiblatex(report) = state
        .execute(Command::ImportBiblatex(ImportBiblatex {
            source_text: source.into(),
            choices,
        }))
        .await
        .unwrap()
    else {
        panic!("import");
    };
    report
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

#[tokio::test]
async fn retrying_same_source_does_not_duplicate_committed_entries() {
    let state = app();
    let source = r#"
        @book{a, title={First}, author={Ada, Lovelace}, year={1843}}
        @book{b, title={Second}, author={Hopper, Grace}, year={1952}}
    "#;
    let first = import(&state, source).await;
    assert_eq!(first.imported.len(), 2);
    let after_first = resource_count(&state).await;

    let second = import(&state, source).await;
    assert!(
        second.imported.is_empty(),
        "retry must not create new resources: {:?}",
        second.imported
    );
    assert_eq!(
        second.reused.len(),
        2,
        "committed entries reused: {second:?}"
    );
    assert_eq!(resource_count(&state).await, after_first);
}

#[tokio::test]
async fn publication_duplicate_from_different_source_requires_policy() {
    let state = app();
    let first = import(
        &state,
        "@book{one, title={Notes}, author={Ada, Lovelace}, doi={10.1000/notes}}",
    )
    .await;
    assert_eq!(first.imported.len(), 1);
    let count = resource_count(&state).await;

    let second = import(
        &state,
        "@book{otherkey, title={Notes elsewhere}, author={Ada, Lovelace}, doi={10.1000/notes}}",
    )
    .await;
    assert!(
        second.imported.is_empty(),
        "must not silently create a DOI duplicate"
    );
    assert!(
        second
            .conflicted
            .iter()
            .any(|i| i.code.as_deref() == Some(import_codes::PUBLICATION_DUPLICATE)),
        "expected publication_duplicate conflict: {:?}",
        second.conflicted
    );
    assert_eq!(resource_count(&state).await, count);
}

#[tokio::test]
async fn intentional_duplicate_creates_second_copy() {
    let state = app();
    import(
        &state,
        "@book{one, title={Notes}, author={Ada, Lovelace}, doi={10.1000/notes}}",
    )
    .await;
    let before = resource_count(&state).await;

    let mut choices = ImportChoices::default();
    choices
        .entries
        .insert("otherkey".into(), EntryChoice::IntentionalDuplicate);
    let report = import_with_choices(
        &state,
        "@book{otherkey, title={Notes copy}, author={Ada, Lovelace}, doi={10.1000/notes}}",
        choices,
    )
    .await;
    assert_eq!(report.imported.len(), 1);
    assert_eq!(resource_count(&state).await, before + 1);
}

#[tokio::test]
async fn identical_citekeys_in_unrelated_sources_do_not_collide() {
    let state = app();
    let first = import(
        &state,
        "@book{shared, title={Alpha}, author={Ada, Lovelace}, year={1843}}",
    )
    .await;
    assert_eq!(first.imported.len(), 1);
    let id_a = first.imported[0].resource_id;

    let second = import(
        &state,
        "@book{shared, title={Beta}, author={Hopper, Grace}, year={1952}}",
    )
    .await;
    assert_eq!(
        second.imported.len(),
        1,
        "different source fingerprint must allow same citekey: {second:?}"
    );
    assert_ne!(second.imported[0].resource_id, id_a);
    assert_eq!(resource_count(&state).await, 2);
}

#[tokio::test]
async fn ambiguous_people_remain_distinct_until_explicit_choice() {
    let state = app();
    let Outcome::CreatePerson(p1) = state
        .execute(Command::CreatePerson(CreatePerson {
            name: oeuvre_core::Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
            aliases: vec![],
        }))
        .await
        .unwrap()
    else {
        panic!("p1");
    };
    let Outcome::CreatePerson(p2) = state
        .execute(Command::CreatePerson(CreatePerson {
            name: oeuvre_core::Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
            aliases: vec![],
        }))
        .await
        .unwrap()
    else {
        panic!("p2");
    };
    assert_ne!(p1.id, p2.id);

    let source = "@book{ambig, title={Notes}, author={Lovelace, Ada}, year={1843}}";
    let Outcome::PreviewBiblatex(preview) = state
        .execute(Command::PreviewBiblatex(PreviewBiblatex {
            source_text: source.into(),
        }))
        .await
        .unwrap()
    else {
        panic!("preview");
    };
    assert!(preview.needs_review());
    let entry = &preview.entries[0];
    assert!(
        entry
            .conflicts
            .iter()
            .any(|c| c.code == import_codes::AMBIGUOUS_CREATOR)
    );

    let without = import(&state, source).await;
    assert!(
        without.imported.is_empty(),
        "ambiguous people must not auto-merge"
    );
    assert!(
        without
            .conflicted
            .iter()
            .any(|i| i.code.as_deref() == Some(import_codes::AMBIGUOUS_CREATOR)),
        "{:?}",
        without.conflicted
    );

    let mut choices = ImportChoices::default();
    choices.people.insert(
        ImportChoices::person_key("ambig", 0),
        PersonChoice::UseExisting {
            person_id: p1.id,
            expected_name_key: p1.name.key().as_str().to_owned(),
        },
    );
    let with = import_with_choices(&state, source, choices).await;
    assert_eq!(with.imported.len(), 1);
    let Outcome::GetResource(book) = state
        .execute(Command::GetResource(GetResource {
            id: with.imported[0].resource_id,
        }))
        .await
        .unwrap()
    else {
        panic!("book");
    };
    assert_eq!(book.creators[0].person_id, p1.id);
}

#[tokio::test]
async fn stale_preview_person_choice_is_rejected() {
    let state = app();
    let Outcome::CreatePerson(p1) = state
        .execute(Command::CreatePerson(CreatePerson {
            name: oeuvre_core::Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
            aliases: vec![],
        }))
        .await
        .unwrap()
    else {
        panic!("p1");
    };
    let _ = state
        .execute(Command::CreatePerson(CreatePerson {
            name: oeuvre_core::Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
            aliases: vec![],
        }))
        .await
        .unwrap();

    let mut choices = ImportChoices::default();
    choices.people.insert(
        ImportChoices::person_key("ambig", 0),
        PersonChoice::UseExisting {
            person_id: p1.id,
            expected_name_key: p1.name.key().as_str().to_owned(),
        },
    );
    state
        .execute(Command::DeletePerson(DeletePerson { id: p1.id }))
        .await
        .unwrap();

    let report = import_with_choices(
        &state,
        "@book{ambig, title={Notes}, author={Lovelace, Ada}, year={1843}}",
        choices,
    )
    .await;
    assert!(report.imported.is_empty());
    assert!(
        report
            .conflicted
            .iter()
            .any(|i| i.code.as_deref() == Some(import_codes::STALE_PREVIEW)),
        "{:?}",
        report.conflicted
    );
}

#[tokio::test]
async fn stale_reuse_choice_for_deleted_resource_is_rejected() {
    let store = MemoryStore::new();
    let state = AppState::builder(store.clone())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();
    let first = import(
        &state,
        "@book{one, title={Notes}, author={Ada, Lovelace}, doi={10.1000/notes}}",
    )
    .await;
    let id = first.imported[0].resource_id;
    store.test_delete_resource(id);

    let mut choices = ImportChoices::default();
    choices
        .entries
        .insert("otherkey".into(), EntryChoice::Reuse { resource_id: id });
    let report = import_with_choices(
        &state,
        "@book{otherkey, title={Notes}, author={Ada, Lovelace}, doi={10.1000/notes}}",
        choices,
    )
    .await;
    assert!(report.imported.is_empty());
    assert!(
        report
            .conflicted
            .iter()
            .any(|i| i.code.as_deref() == Some(import_codes::STALE_PREVIEW)),
        "{:?}",
        report.conflicted
    );
}

#[tokio::test]
async fn partial_report_agrees_with_persisted_records_after_failure() {
    let store = MemoryStore::new();
    store.fail_imported_create_at(1);
    let state = AppState::builder(store.clone())
        .surface(Surface::Test)
        .sink(MemorySink::new())
        .build();

    let source = r#"
        @book{a, title={First}, author={Ada, Lovelace}, year={1843}}
        @book{b, title={Second}, author={Hopper, Grace}, year={1952}}
        @book{c, title={Third}, author={Turing, Alan}, year={1936}}
    "#;
    let report = import(&state, source).await;
    assert_eq!(
        report.imported.len(),
        1,
        "only first unit commits: {report:?}"
    );
    assert_eq!(report.imported[0].entry_key, "a");
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].entry_key.as_deref(), Some("b"));
    assert_eq!(report.not_attempted.len(), 1);
    assert_eq!(report.not_attempted[0].entry_key.as_deref(), Some("c"));

    let Outcome::ListResources(rows) = state
        .execute(Command::ListResources(ListResources))
        .await
        .unwrap()
    else {
        panic!("list");
    };
    let titles: Vec<_> = rows.iter().map(|r| r.title.as_str().to_owned()).collect();
    assert!(
        titles.iter().any(|t| t == "First"),
        "persisted titles: {titles:?}"
    );
    assert!(
        !titles.iter().any(|t| t == "Second" || t == "Third"),
        "failed/not-attempted must not persist: {titles:?}"
    );

    store.clear_import_fail();
    let retry = import(&state, source).await;
    assert_eq!(retry.reused.len(), 1);
    assert_eq!(retry.reused[0].entry_key, "a");
    assert_eq!(retry.imported.len(), 2);
}

#[tokio::test]
async fn preview_lists_matches_and_conflicts_without_writing() {
    let state = app();
    import(
        &state,
        "@book{seed, title={Notes}, author={Ada, Lovelace}, doi={10.1000/notes}}",
    )
    .await;
    let before = resource_count(&state).await;

    let Outcome::PreviewBiblatex(preview) = state
        .execute(Command::PreviewBiblatex(PreviewBiblatex {
            source_text: "@book{other, title={Dup}, author={Ada, Lovelace}, doi={10.1000/notes}}"
                .into(),
        }))
        .await
        .unwrap()
    else {
        panic!("preview");
    };
    assert!(preview.needs_review());
    assert!(matches!(
        preview.entries[0].proposal,
        oeuvre_core::EntryProposal::PublicationDuplicate { .. }
    ));
    assert_eq!(resource_count(&state).await, before);
}

#[tokio::test]
async fn reuse_choice_attaches_without_overwrite() {
    let state = app();
    let first = import(
        &state,
        "@book{one, title={Original Title}, author={Ada, Lovelace}, doi={10.1000/notes}}",
    )
    .await;
    let id = first.imported[0].resource_id;

    let mut choices = ImportChoices::default();
    choices
        .entries
        .insert("two".into(), EntryChoice::Reuse { resource_id: id });
    let report = import_with_choices(
        &state,
        "@book{two, title={Different Title}, author={Ada, Lovelace}, doi={10.1000/notes}}",
        choices,
    )
    .await;
    assert_eq!(report.reused.len(), 1);
    assert_eq!(report.reused[0].resource_id, id);
    let Outcome::GetResource(book) = state
        .execute(Command::GetResource(GetResource { id }))
        .await
        .unwrap()
    else {
        panic!("book");
    };
    assert_eq!(book.title.as_str(), "Original Title");
}
