//! The "Import bibliography" dialog: entry summary, batch collection and tag
//! selection, and conflict handling in one place.
//!
//! Selections stay local until the user confirms. New names are staged as
//! names only; the shared import pipeline creates them with the import, so
//! cancelling writes nothing.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::combobox::{Combobox, ComboboxEvent, ComboboxState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use ibex_core::{
    Collection, CollectionId, DiagnosticSeverity, EntryProposal, ExistingIdOrName,
    ImportAssignments, ImportPreview, NonEmptyStr, Tag, TagId,
};

use crate::collections::{CollectionChoice, CollectionDelegate};
use crate::tags::{TagChoice, TagDelegate};

/// One staged batch target: an existing library object, or a new name the
/// import will create.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Staged<Id> {
    Existing { id: Id, name: String },
    New { name: NonEmptyStr },
}

impl<Id: Copy + std::fmt::Display> Staged<Id> {
    pub fn name(&self) -> &str {
        match self {
            Staged::Existing { name, .. } => name,
            Staged::New { name } => name.as_str(),
        }
    }

    /// Stable element-id suffix for this selection's controls.
    fn key(&self) -> String {
        match self {
            Staged::Existing { id, .. } => id.to_string(),
            Staged::New { name } => format!("new-{}", name.as_str()),
        }
    }

    fn target(&self) -> ExistingIdOrName<Id> {
        match self {
            Staged::Existing { id, .. } => ExistingIdOrName::Existing { id: *id },
            Staged::New { name } => ExistingIdOrName::Name { name: name.clone() },
        }
    }
}

/// How entries that need review are handled when the preview has any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictPolicy {
    SkipConflicted,
    ImportDuplicates,
}

/// Emitted once when the user confirms; the owner closes the dialog and
/// runs the import.
#[derive(Clone, Debug)]
pub struct ImportConfirmed {
    pub policy: Option<ConflictPolicy>,
    pub assignments: ImportAssignments,
}

pub struct ImportDialog {
    preview: ImportPreview,
    library_tags: Vec<Tag>,
    library_collections: Vec<Collection>,
    tags: Vec<Staged<TagId>>,
    collections: Vec<Staged<CollectionId>>,
    tag_picker: Entity<ComboboxState<TagDelegate>>,
    collection_picker: Entity<ComboboxState<CollectionDelegate>>,
    /// Set on confirm so repeated clicks or keys cannot start a second import.
    submitted: bool,
}

impl EventEmitter<ImportConfirmed> for ImportDialog {}

impl ImportDialog {
    pub fn new(
        preview: ImportPreview,
        library_tags: Vec<Tag>,
        library_collections: Vec<Collection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tag_picker = cx.new(|cx| {
            ComboboxState::new(TagDelegate::new(&library_tags, &[]), Vec::new(), window, cx)
                .searchable(true)
        });
        cx.subscribe_in(
            &tag_picker,
            window,
            |this, picker, event: &ComboboxEvent<TagDelegate>, window, cx| {
                let ComboboxEvent::Change(values) = event else {
                    return;
                };
                let Some(choice) = values.first().cloned() else {
                    return;
                };
                picker.update(cx, |state, cx| {
                    state.set_selected_indices(Vec::new(), window, cx);
                    state.set_query("", window, cx);
                });
                this.stage_tag(choice, window, cx);
            },
        )
        .detach();
        let collection_picker = cx.new(|cx| {
            ComboboxState::new(
                CollectionDelegate::creatable(&library_collections, &[]),
                Vec::new(),
                window,
                cx,
            )
            .searchable(true)
        });
        cx.subscribe_in(
            &collection_picker,
            window,
            |this, picker, event: &ComboboxEvent<CollectionDelegate>, window, cx| {
                let ComboboxEvent::Change(values) = event else {
                    return;
                };
                let Some(choice) = values.first().cloned() else {
                    return;
                };
                picker.update(cx, |state, cx| {
                    state.set_selected_indices(Vec::new(), window, cx);
                    state.set_query("", window, cx);
                });
                this.stage_collection(choice, window, cx);
            },
        )
        .detach();
        Self {
            preview,
            library_tags,
            library_collections,
            tags: Vec::new(),
            collections: Vec::new(),
            tag_picker,
            collection_picker,
            submitted: false,
        }
    }

    pub fn needs_review(&self) -> bool {
        self.preview.needs_review()
    }

    pub fn stage_tag(&mut self, choice: TagChoice, window: &mut Window, cx: &mut Context<Self>) {
        let staged = match choice {
            TagChoice::Existing(id) => match self.library_tags.iter().find(|t| t.id == id) {
                Some(tag) => Staged::Existing {
                    id,
                    name: tag.text.as_str().to_owned(),
                },
                None => return,
            },
            TagChoice::Create(raw) => {
                let Ok(name) = NonEmptyStr::try_new(raw) else {
                    return;
                };
                match self.library_tags.iter().find(|t| t.text == name) {
                    Some(tag) => Staged::Existing {
                        id: tag.id,
                        name: tag.text.as_str().to_owned(),
                    },
                    None => Staged::New { name },
                }
            }
        };
        if !self.tags.contains(&staged) {
            self.tags.push(staged);
        }
        self.sync_pickers(window, cx);
        cx.notify();
    }

    pub fn stage_collection(
        &mut self,
        choice: CollectionChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let staged = match choice {
            CollectionChoice::Existing(id) => {
                match self.library_collections.iter().find(|c| c.id == id) {
                    Some(collection) => Staged::Existing {
                        id,
                        name: collection.name.as_str().to_owned(),
                    },
                    None => return,
                }
            }
            CollectionChoice::Create(raw) => {
                let Ok(name) = NonEmptyStr::try_new(raw) else {
                    return;
                };
                Staged::New { name }
            }
        };
        if !self.collections.contains(&staged) {
            self.collections.push(staged);
        }
        self.sync_pickers(window, cx);
        cx.notify();
    }

    fn remove_tag(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.tags.retain(|t| t.key() != key);
        self.sync_pickers(window, cx);
        cx.notify();
    }

    fn remove_collection(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.collections.retain(|c| c.key() != key);
        self.sync_pickers(window, cx);
        cx.notify();
    }

    /// Rebuild both pickers so staged objects leave the lists and staged
    /// names are not offered for creation again.
    fn sync_pickers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let staged_tags: Vec<TagId> = self
            .tags
            .iter()
            .filter_map(|t| match t {
                Staged::Existing { id, .. } => Some(*id),
                Staged::New { .. } => None,
            })
            .collect();
        let available: Vec<Tag> = self
            .library_tags
            .iter()
            .filter(|t| !staged_tags.contains(&t.id))
            .cloned()
            .collect();
        let blocked: Vec<String> = self.tags.iter().map(|t| t.name().to_lowercase()).collect();
        self.tag_picker.update(cx, |state, cx| {
            state.set_items(TagDelegate::new(&available, &blocked), window, cx);
        });

        let staged_collections: Vec<CollectionId> = self
            .collections
            .iter()
            .filter_map(|c| match c {
                Staged::Existing { id, .. } => Some(*id),
                Staged::New { .. } => None,
            })
            .collect();
        let available: Vec<Collection> = self
            .library_collections
            .iter()
            .filter(|c| !staged_collections.contains(&c.id))
            .cloned()
            .collect();
        let blocked: Vec<String> = self
            .collections
            .iter()
            .map(|c| c.name().to_lowercase())
            .collect();
        self.collection_picker.update(cx, |state, cx| {
            state.set_items(
                CollectionDelegate::creatable(&available, &blocked),
                window,
                cx,
            );
        });
    }

    pub fn assignments(&self) -> ImportAssignments {
        ImportAssignments {
            collections: self.collections.iter().map(Staged::target).collect(),
            tags: self.tags.iter().map(Staged::target).collect(),
        }
    }

    pub fn confirm(&mut self, policy: Option<ConflictPolicy>, cx: &mut Context<Self>) {
        if self.submitted {
            return;
        }
        self.submitted = true;
        cx.emit(ImportConfirmed {
            policy,
            assignments: self.assignments(),
        });
        cx.notify();
    }

    /// One-line entry summary, also the dialog's accessible label.
    pub fn summary(&self) -> String {
        let entries = &self.preview.entries;
        let count =
            |f: fn(&EntryProposal) -> bool| entries.iter().filter(|e| f(&e.proposal)).count();
        let new = count(|p| matches!(p, EntryProposal::Create));
        let existing = count(|p| matches!(p, EntryProposal::AlreadyCommitted { .. }));
        // Entries the parser rejects only appear as diagnostics.
        let rejected = self
            .preview
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error && d.entry_key.is_some())
            .count();
        let skipped = count(|p| matches!(p, EntryProposal::Skip { .. })) + rejected;
        let review = entries.iter().filter(|e| e.needs_review()).count();
        let total = entries.len() + rejected;
        let mut parts = vec![format!(
            "{total} entr{}",
            if total == 1 { "y" } else { "ies" }
        )];
        parts.push(format!("{new} new"));
        if existing > 0 {
            parts.push(format!("{existing} already imported"));
        }
        if review > 0 {
            parts.push(format!(
                "{review} need{} review",
                if review == 1 { "s" } else { "" }
            ));
        }
        if skipped > 0 {
            parts.push(format!("{skipped} will be skipped"));
        }
        parts.join(" · ")
    }

    fn review_lines(&self) -> Vec<String> {
        self.preview
            .entries
            .iter()
            .filter(|e| e.needs_review())
            .flat_map(|e| {
                let mut lines: Vec<String> = e
                    .conflicts
                    .iter()
                    .map(|c| format!("{} — {}: {}", e.entry_key, c.code, c.message))
                    .collect();
                if let EntryProposal::PublicationDuplicate { reason, .. } = &e.proposal
                    && lines.is_empty()
                {
                    lines.push(format!("{} — {reason}", e.entry_key));
                }
                lines
            })
            .take(100)
            .collect()
    }

    /// Footer actions. Built by the dialog owner on every render so the
    /// buttons track the submitted state.
    pub fn footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let submitted = self.submitted;
        let cancel = Button::new("import-cancel")
            .label("Cancel")
            .disabled(submitted)
            .on_click(|_, window, cx| window.close_dialog(cx));
        let mut row = h_flex().w_full().justify_end().gap_2().child(cancel);
        if self.needs_review() {
            row = row
                .child(
                    Button::new("import-intentional-dups")
                        .label("Import duplicates")
                        .disabled(submitted)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm(Some(ConflictPolicy::ImportDuplicates), cx)
                        })),
                )
                .child(
                    Button::new("import-skip-conflicts")
                        .primary()
                        .label("Skip conflicted")
                        .loading(submitted)
                        .disabled(submitted)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm(Some(ConflictPolicy::SkipConflicted), cx)
                        })),
                );
        } else {
            row = row.child(
                Button::new("import-confirm")
                    .primary()
                    .label("Import")
                    .loading(submitted)
                    .disabled(submitted)
                    .on_click(cx.listener(|this, _, _, cx| this.confirm(None, cx))),
            );
        }
        row.into_any_element()
    }
}

fn selection_row(
    id: SharedString,
    name: String,
    tooltip: &'static str,
    disabled: bool,
    on_remove: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .child(div().flex_1().min_w_0().child(name))
        .child(
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::Close).xsmall())
                .tooltip(tooltip)
                .disabled(disabled)
                .on_click(on_remove),
        )
}

impl Render for ImportDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let summary = self.summary();
        let review_lines = self.review_lines();
        let muted = cx.theme().muted_foreground;
        let disabled = self.submitted;
        let label = |text: &'static str| div().text_size(rems(1.)).text_color(muted).child(text);

        let collection_rows: Vec<_> = self
            .collections
            .iter()
            .map(|c| {
                let key = c.key();
                let entity = cx.entity().downgrade();
                selection_row(
                    format!("import-remove-collection-{key}").into(),
                    match c {
                        Staged::New { name } => format!("{} (new)", name.as_str()),
                        Staged::Existing { name, .. } => name.clone(),
                    },
                    "Remove collection",
                    disabled,
                    move |_, window, cx| {
                        _ = entity.update(cx, |this, cx| this.remove_collection(&key, window, cx));
                    },
                )
            })
            .collect();
        let tag_rows: Vec<_> = self
            .tags
            .iter()
            .map(|t| {
                let key = t.key();
                let entity = cx.entity().downgrade();
                selection_row(
                    format!("import-remove-tag-{key}").into(),
                    match t {
                        Staged::New { name } => format!("{} (new)", name.as_str()),
                        Staged::Existing { name, .. } => name.clone(),
                    },
                    "Remove tag",
                    disabled,
                    move |_, window, cx| {
                        _ = entity.update(cx, |this, cx| this.remove_tag(&key, window, cx));
                    },
                )
            })
            .collect();

        v_flex()
            .id("import-bibliography")
            .test_support()
            .aria_label(summary.clone())
            .gap_3()
            .child(summary)
            .when(!review_lines.is_empty(), |el| {
                el.child(
                    div()
                        .id("biblatex-import-review-issues")
                        .max_h(px(200.))
                        .overflow_y_scroll()
                        .child(v_flex().gap_2().children(
                            review_lines
                                .into_iter()
                                .map(|line| div().text_size(rems(1.)).child(line)),
                        )),
                )
                .child(div().text_size(rems(1.)).text_color(muted).child(
                    "Skip conflicted leaves entries that need review out. Import duplicates creates new copies.",
                ))
            })
            .child(
                v_flex()
                    .gap_1()
                    .child(label("Collections"))
                    .child(
                        div().id("import-collections-picker").test_support().w_full().child(
                            Combobox::new(&self.collection_picker)
                                .small()
                                .placeholder("Add collection")
                                .search_placeholder("Search or create...")
                                .disabled(disabled)
                                .w_full(),
                        ),
                    )
                    .children(collection_rows),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(label("Tags"))
                    .child(
                        div().id("import-tags-picker").test_support().w_full().child(
                            Combobox::new(&self.tag_picker)
                                .small()
                                .placeholder("Add tag")
                                .search_placeholder("Search or create...")
                                .disabled(disabled)
                                .w_full(),
                        ),
                    )
                    .children(tag_rows),
            )
            .child(div().text_size(rems(1.)).text_color(muted).child(
                "Every imported or reused entry is added to these collections and tags. Existing memberships are kept. New collections are top-level.",
            ))
    }
}
