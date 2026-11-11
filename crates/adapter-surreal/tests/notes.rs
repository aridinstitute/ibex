use ibex_adapter_surreal::SurrealStore;
use ibex_app::command::{CreateNote, DeleteNote, ListNoteFeed, ListNotes, UpdateNote};
use ibex_app::testing::MemorySink;
use ibex_app::{AppState, Command, Outcome, Surface};
use ibex_core::{
    CitationLocation, LocationKind, NewResource, NonEmptyStr, NoteContent, NoteKind, ResourceId,
    ResourceKind, Store,
};

fn book(title: &str) -> NewResource {
    NewResource {
        kind: ResourceKind::Book,
        title: NonEmptyStr::try_new(title).unwrap(),
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
    }
}

#[tokio::test]
async fn creates_and_lists_resource_linked_markdown_notes() {
    let store = SurrealStore::open_memory().await.unwrap();
    let resource = store.create_resource(book("Source")).await.unwrap();
    let other = store.create_resource(book("Other source")).await.unwrap();
    let sink = MemorySink::new();
    let state = AppState::builder(store)
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();
    let body = "    indented code\n\n**A thought**\n";
    let content = NoteContent {
        kind: NoteKind::Quote,
        body_markdown: body.into(),
        locations: vec![
            CitationLocation {
                kind: LocationKind::Page,
                value: "12–15".into(),
            },
            CitationLocation {
                kind: LocationKind::Section,
                value: "2.1".into(),
            },
        ],
    };
    let Outcome::CreateNote(note) = state
        .execute(Command::CreateNote(CreateNote {
            resource_id: resource.id,
            content: content.clone(),
        }))
        .await
        .unwrap()
    else {
        panic!("expected created note")
    };
    assert_eq!(note.resource_id, Some(resource.id));
    assert_eq!(note.content, content);
    assert_eq!(note.created_at, note.updated_at);
    let Outcome::ListNotes(notes) = state
        .execute(Command::ListNotes(ListNotes {
            resource_id: resource.id,
        }))
        .await
        .unwrap()
    else {
        panic!("expected notes")
    };
    assert_eq!(notes, vec![note]);
    let Outcome::ListNotes(notes) = state
        .execute(Command::ListNotes(ListNotes {
            resource_id: other.id,
        }))
        .await
        .unwrap()
    else {
        panic!("expected notes")
    };
    assert!(notes.is_empty());
    let events = sink.take();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].command(), "create_note");
    let logged = format!("{events:?}");
    assert!(!logged.contains("A thought"));
}

#[tokio::test]
async fn edits_and_deletes_notes_without_touching_the_resource() {
    let store = SurrealStore::open_memory().await.unwrap();
    let resource = store
        .create_resource(book("Unchanged source"))
        .await
        .unwrap();
    let first = store
        .create_note(
            resource.id,
            NoteContent {
                body_markdown: "Original".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let second = store
        .create_note(
            resource.id,
            NoteContent {
                kind: NoteKind::Paraphrase,
                body_markdown: "Keep me".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let sink = MemorySink::new();
    let state = AppState::builder(store)
        .surface(Surface::Test)
        .sink(sink.clone())
        .build();
    let revised = NoteContent {
        kind: NoteKind::Summary,
        body_markdown: "# Revised\n\n- one\n- two".into(),
        locations: vec![
            CitationLocation {
                kind: LocationKind::Chapter,
                value: "IV".into(),
            },
            CitationLocation {
                kind: LocationKind::Timestamp,
                value: "01:23–02:10".into(),
            },
        ],
    };
    let Outcome::UpdateNote(updated) = state
        .execute(Command::UpdateNote(UpdateNote {
            id: first.id,
            content: revised.clone(),
        }))
        .await
        .unwrap()
    else {
        panic!("expected updated note")
    };
    assert_eq!(updated.id, first.id);
    assert_eq!(updated.resource_id, Some(resource.id));
    assert_eq!(updated.created_at, first.created_at);
    assert!(updated.updated_at.as_unix_ms() >= first.updated_at.as_unix_ms());
    assert_eq!(updated.content, revised);
    let Outcome::ListNotes(notes) = state
        .execute(Command::ListNotes(ListNotes {
            resource_id: resource.id,
        }))
        .await
        .unwrap()
    else {
        panic!("expected notes")
    };
    assert_eq!(notes, vec![updated, second.clone()]);
    state
        .execute(Command::DeleteNote(DeleteNote { id: first.id }))
        .await
        .unwrap();
    let Outcome::ListNotes(notes) = state
        .execute(Command::ListNotes(ListNotes {
            resource_id: resource.id,
        }))
        .await
        .unwrap()
    else {
        panic!("expected notes")
    };
    assert_eq!(notes, vec![second]);
    assert!(matches!(
        state
            .execute(Command::DeleteNote(DeleteNote { id: first.id }))
            .await,
        Err(ibex_app::CommandError::NotFound(_))
    ));
    assert!(matches!(
        state
            .execute(Command::UpdateNote(UpdateNote {
                id: first.id,
                content: revised
            }))
            .await,
        Err(ibex_app::CommandError::NotFound(_))
    ));
    let Outcome::GetResource(loaded) = state
        .execute(Command::GetResource(ibex_app::command::GetResource {
            id: resource.id,
        }))
        .await
        .unwrap()
    else {
        panic!("expected source")
    };
    assert_eq!(loaded.title.as_str(), "Unchanged source");
    assert_eq!(loaded.date_edited, resource.date_edited);
    let events = sink.take();
    assert_eq!(events.len(), 7);
    assert_eq!(events[0].command(), "update_note");
    assert_eq!(events[2].command(), "delete_note");
    assert_eq!(events[4].status(), ibex_app::Status::Error);
}

async fn validate_notes(store: impl Store) {
    let resource = store.create_resource(book("Source")).await.unwrap();
    let good = NoteContent {
        body_markdown: "Saved".into(),
        ..Default::default()
    };
    assert!(
        store
            .create_note(ResourceId::generate(), good.clone())
            .await
            .unwrap_err()
            .is_not_found()
    );
    let saved = store.create_note(resource.id, good).await.unwrap();
    for content in [
        NoteContent {
            body_markdown: " \n\t".into(),
            ..Default::default()
        },
        NoteContent {
            body_markdown: "Valid body".into(),
            locations: vec![CitationLocation {
                kind: LocationKind::Page,
                value: " ".into(),
            }],
            ..Default::default()
        },
    ] {
        assert!(
            store
                .create_note(resource.id, content.clone())
                .await
                .unwrap_err()
                .is_invalid()
        );
        assert!(
            store
                .update_note(saved.id, content)
                .await
                .unwrap_err()
                .is_invalid()
        );
        assert_eq!(
            store.list_notes(resource.id).await.unwrap(),
            vec![saved.clone()]
        );
    }
    assert!(
        store
            .list_notes(ResourceId::generate())
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn surreal_and_memory_store_enforce_the_same_note_contract() {
    validate_notes(SurrealStore::open_memory().await.unwrap()).await;
    validate_notes(ibex_app::testing::MemoryStore::new()).await;
}

#[tokio::test]
async fn note_feed_lists_across_resources_newest_first_and_filters() {
    let store = SurrealStore::open_memory().await.unwrap();
    let alpha = store.create_resource(book("Alpha")).await.unwrap();
    let beta = store.create_resource(book("Beta")).await.unwrap();
    let older = store
        .create_note(
            alpha.id,
            NoteContent {
                body_markdown: "Older on Alpha".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let newer = store
        .create_note(
            beta.id,
            NoteContent {
                kind: NoteKind::Quote,
                body_markdown: "Newer on Beta".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let bumped = store
        .update_note(
            older.id,
            NoteContent {
                body_markdown: "Edited Alpha note".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(bumped.updated_at.as_unix_ms() >= newer.updated_at.as_unix_ms());

    let state = AppState::builder(store).surface(Surface::Test).build();
    let Outcome::ListNoteFeed(feed) = state
        .execute(Command::ListNoteFeed(ListNoteFeed { resource_id: None }))
        .await
        .unwrap()
    else {
        panic!("expected note feed")
    };
    assert_eq!(feed.len(), 2);
    assert_eq!(feed[0].note.id, bumped.id);
    assert_eq!(
        feed[0].resource_title.as_ref().map(|t| t.as_str()),
        Some("Alpha")
    );
    assert_eq!(feed[1].note.id, newer.id);
    assert_eq!(
        feed[1].resource_title.as_ref().map(|t| t.as_str()),
        Some("Beta")
    );

    let Outcome::ListNoteFeed(filtered) = state
        .execute(Command::ListNoteFeed(ListNoteFeed {
            resource_id: Some(beta.id),
        }))
        .await
        .unwrap()
    else {
        panic!("expected filtered feed")
    };
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].note.id, newer.id);
    assert_eq!(
        filtered[0].resource_title.as_ref().map(|t| t.as_str()),
        Some("Beta")
    );

    let memory = ibex_app::testing::MemoryStore::new();
    let alpha = memory.create_resource(book("Alpha")).await.unwrap();
    let beta = memory.create_resource(book("Beta")).await.unwrap();
    let first = memory
        .create_note(
            alpha.id,
            NoteContent {
                body_markdown: "A".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let second = memory
        .create_note(
            beta.id,
            NoteContent {
                body_markdown: "B".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    memory
        .update_note(
            first.id,
            NoteContent {
                body_markdown: "A2".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let feed = memory.list_note_feed(None).await.unwrap();
    assert_eq!(
        feed.iter().map(|item| item.note.id).collect::<Vec<_>>(),
        vec![first.id, second.id]
    );
    assert_eq!(
        memory
            .list_note_feed(Some(beta.id))
            .await
            .unwrap()
            .iter()
            .map(|item| item.note.id)
            .collect::<Vec<_>>(),
        vec![second.id]
    );
}

#[tokio::test]
async fn notes_survive_close_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library");
    let store = SurrealStore::open(&path).await.unwrap();
    let resource = store
        .create_resource(book("Persistent source"))
        .await
        .unwrap();
    let note = store
        .create_note(
            resource.id,
            NoteContent {
                kind: NoteKind::Quote,
                body_markdown: "> Lasting words\n".into(),
                locations: vec![CitationLocation {
                    kind: LocationKind::Page,
                    value: "iv–xii, 12–15".into(),
                }],
            },
        )
        .await
        .unwrap();
    store.close().await.unwrap();
    let reopened = loop {
        match SurrealStore::open(&path).await {
            Ok(store) => break store,
            Err(e) if e.is_in_use() => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await
            }
            Err(e) => panic!("reopen: {e}"),
        }
    };
    assert_eq!(reopened.list_notes(resource.id).await.unwrap(), vec![note]);
    reopened.close().await.unwrap();
}
