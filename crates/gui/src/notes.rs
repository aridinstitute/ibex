//! Resource-linked note editing. Drafts never write until Save.
use crate::db::DbWorker;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::text::{TextView, TextViewState, TextViewStyle};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ibex_core::{CitationLocation, LocationKind, Note, NoteContent, NoteId, NoteKind, ResourceId};

gpui_kit::actions!([AddNote]);

/// Text color for a note type. Light variants stay deep enough to read on
/// the white card; dark variants lift the same hue. The four hues sit well
/// apart so Summary, Quote, Paraphrase, and Note stay distinct.
fn note_kind_color(kind: NoteKind, dark: bool) -> Hsla {
    let hex = match (kind, dark) {
        (NoteKind::Note, false) => 0x0369a1,
        (NoteKind::Note, true) => 0x38bdf8,
        (NoteKind::Summary, false) => 0x15803d,
        (NoteKind::Summary, true) => 0x4ade80,
        (NoteKind::Paraphrase, false) => 0xc2410c,
        (NoteKind::Paraphrase, true) => 0xfb923c,
        (NoteKind::Quote, false) => 0x7e22ce,
        (NoteKind::Quote, true) => 0xc084fc,
    };
    rgb(hex).into()
}

struct NoteView {
    note: Note,
    markdown: Entity<TextViewState>,
}
struct LocationInput {
    id: usize,
    kind: LocationKind,
    value: Entity<InputState>,
}
struct NoteEditor {
    id: Option<NoteId>,
    kind: NoteKind,
    body: Entity<TextareaState>,
    locations: Vec<LocationInput>,
    next_location: usize,
}

pub struct NotesPanel {
    worker: DbWorker,
    resource: Option<ResourceId>,
    generation: u64,
    notes: Vec<NoteView>,
    editor: Option<NoteEditor>,
    loading: bool,
    pending: bool,
    error: Option<String>,
    load_error: Option<String>,
    confirming_delete: Option<NoteId>,
    return_focus: Option<FocusHandle>,
}

impl NotesPanel {
    pub fn new(worker: DbWorker) -> Self {
        Self {
            worker,
            resource: None,
            generation: 0,
            notes: Vec::new(),
            editor: None,
            loading: false,
            pending: false,
            error: None,
            load_error: None,
            confirming_delete: None,
            return_focus: None,
        }
    }
    pub fn is_editing(&self) -> bool {
        self.editor.is_some()
    }
    pub fn select(&mut self, resource: ResourceId, window: &mut Window, cx: &mut Context<Self>) {
        if self.resource == Some(resource) {
            return;
        }
        self.resource = Some(resource);
        self.generation += 1;
        self.notes.clear();
        self.editor = None;
        self.pending = false;
        self.confirming_delete = None;
        self.error = None;
        self.reload(window, cx);
    }
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(resource) = self.resource else {
            return;
        };
        let generation = self.generation;
        let worker = self.worker.clone();
        self.loading = true;
        self.load_error = None;
        self.error = None;
        cx.spawn_in(window, async move |view, window| {
            let result = worker.list_notes(resource).await;
            _ = view.update_in(window, |this, _, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(notes) => {
                        this.notes = notes
                            .into_iter()
                            .map(|note| {
                                let markdown = cx.new(|cx| {
                                    TextViewState::markdown(&note.content.body_markdown, cx)
                                });
                                NoteView { note, markdown }
                            })
                            .collect();
                    }
                    Err(error) => this.load_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub fn begin_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.resource.is_none() || self.pending {
            return;
        }
        if let Some(editor) = &self.editor {
            let handle = editor.body.focus_handle(cx);
            window.focus(&handle, cx);
            return;
        }
        self.begin_edit(None, NoteContent::default(), window, cx);
    }
    fn begin_edit(
        &mut self,
        id: Option<NoteId>,
        content: NoteContent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending {
            return;
        }
        self.return_focus = window.focused(cx);
        let body = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(10)
                .placeholder("Write Markdown…")
                .default_value(content.body_markdown)
        });
        let handle = body.focus_handle(cx);
        window.focus(&handle, cx);
        let locations: Vec<_> = content
            .locations
            .into_iter()
            .enumerate()
            .map(|(id, location)| LocationInput {
                id,
                kind: location.kind,
                value: cx.new(|cx| InputState::new(window, cx).default_value(location.value)),
            })
            .collect();
        let next_location = locations.len();
        self.editor = Some(NoteEditor {
            id,
            kind: content.kind,
            body,
            locations,
            next_location,
        });
        self.confirming_delete = None;
        self.error = None;
        cx.notify();
    }
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        self.editor = None;
        self.error = None;
        if let Some(handle) = self.return_focus.take() {
            window.focus(&handle, cx);
        }
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let (Some(resource), Some(editor)) = (self.resource, self.editor.as_ref()) else {
            return;
        };
        let content = NoteContent {
            kind: editor.kind,
            body_markdown: editor.body.read(cx).value().to_string(),
            locations: editor
                .locations
                .iter()
                .map(|l| CitationLocation {
                    kind: l.kind,
                    value: l.value.read(cx).value().to_string(),
                })
                .collect(),
        };
        if let Err(error) = content.validate() {
            self.error = Some(error.message);
            cx.notify();
            return;
        }
        self.pending = true;
        self.error = None;
        let id = editor.id;
        let generation = self.generation;
        let worker = self.worker.clone();
        cx.spawn_in(window, async move |view, window| {
            let result = match id {
                Some(id) => worker.update_note(id, content).await,
                None => worker.create_note(resource, content).await,
            };
            _ = view.update_in(window, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.pending = false;
                match result {
                    Ok(_) => {
                        this.cancel(window, cx);
                        this.reload(window, cx);
                    }
                    Err(error) => {
                        this.error = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }
    fn delete(&mut self, id: NoteId, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        self.pending = true;
        self.error = None;
        let generation = self.generation;
        let worker = self.worker.clone();
        cx.spawn_in(window, async move |view, window| {
            let result = worker.delete_note(id).await;
            _ = view.update_in(window, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.pending = false;
                this.confirming_delete = None;
                match result {
                    Ok(()) => this.reload(window, cx),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn add_location(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let id = editor.next_location;
        editor.next_location += 1;
        let value = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. 12–15"));
        editor.locations.push(LocationInput {
            id,
            kind: LocationKind::Page,
            value,
        });
        cx.notify();
    }
    fn render_editor(
        &self,
        editor: &NoteEditor,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let owner = cx.weak_entity();
        v_flex()
            .id("note-editor")
            .test_support()
            .aria_label("Markdown note")
            .gap_2()
            .min_w_0()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Note type"))
                    .child(
                        Button::new("note-kind")
                            .small()
                            .label(editor.kind.label())
                            .disabled(self.pending)
                            .dropdown_menu_with_anchor(
                                Anchor::TopLeft,
                                move |mut menu, window, _| {
                                    let Some(owner) = owner.upgrade() else {
                                        return menu;
                                    };
                                    for kind in NoteKind::ALL {
                                        menu =
                                            menu.item(PopupMenuItem::new(kind.label()).on_click(
                                                window.listener_for(
                                                    &owner,
                                                    move |this, _, _, cx| {
                                                        if let Some(editor) = &mut this.editor {
                                                            editor.kind = kind;
                                                        }
                                                        cx.notify();
                                                    },
                                                ),
                                            ));
                                    }
                                    menu
                                },
                            ),
                    ),
            )
            .child(
                Textarea::new(&editor.body)
                    .aria_label("Markdown note body")
                    .disabled(self.pending),
            )
            .children(editor.locations.iter().map(|location| {
                let id = location.id;
                let owner = cx.weak_entity();
                h_flex()
                    .gap_1()
                    .min_w_0()
                    .child(
                        Button::new(SharedString::from(format!("location-kind-{id}")))
                            .small()
                            .label(location.kind.label())
                            .disabled(self.pending)
                            .dropdown_menu_with_anchor(
                                Anchor::TopLeft,
                                move |mut menu, window, _| {
                                    let Some(owner) = owner.upgrade() else {
                                        return menu;
                                    };
                                    for kind in LocationKind::ALL {
                                        menu =
                                            menu.item(PopupMenuItem::new(kind.label()).on_click(
                                                window.listener_for(
                                                    &owner,
                                                    move |this, _, _, cx| {
                                                        if let Some(location) =
                                                            this.editor.as_mut().and_then(|e| {
                                                                e.locations
                                                                    .iter_mut()
                                                                    .find(|l| l.id == id)
                                                            })
                                                        {
                                                            location.kind = kind;
                                                        }
                                                        cx.notify();
                                                    },
                                                ),
                                            ));
                                    }
                                    menu
                                },
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .id(SharedString::from(format!("location-value-{id}")))
                            .test_support()
                            .child(
                                Input::new(&location.value)
                                    .small()
                                    .aria_label("Citation location")
                                    .disabled(self.pending),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("remove-location-{id}")))
                            .ghost()
                            .small()
                            .label("×")
                            .accessibility_label("Remove citation location")
                            .disabled(self.pending)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(editor) = &mut this.editor {
                                    editor.locations.retain(|l| l.id != id);
                                }
                                cx.notify();
                            })),
                    )
            }))
            .child(
                Button::new("note-add-location")
                    .ghost()
                    .small()
                    .label("Add citation location")
                    .disabled(self.pending)
                    .on_click(cx.listener(|this, _, window, cx| this.add_location(window, cx))),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("note-save")
                            .primary()
                            .small()
                            .label("Save")
                            .loading(self.pending)
                            .disabled(self.pending)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    )
                    .child(
                        Button::new("note-cancel")
                            .small()
                            .label("Cancel")
                            .disabled(self.pending)
                            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                    ),
            )
    }
}
impl Render for NotesPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = v_flex()
            .id("resource-notes")
            .test_support()
            .gap_3()
            .min_w_0()
            .child(
                h_flex()
                    .justify_between()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(
                        if self.editor.is_some() {
                            "Edit note"
                        } else {
                            "Notes"
                        },
                    ))
                    .when(self.editor.is_none(), |row| {
                        row.child(
                            Button::new("note-new")
                                .ghost()
                                .small()
                                .icon(IconName::Plus)
                                .accessibility_label("Add note")
                                .tooltip("Add note (⌘⇧N)")
                                .disabled(self.pending || self.resource.is_none())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.begin_new(window, cx)),
                                ),
                        )
                    }),
            );
        if let Some(editor) = &self.editor {
            content = content.child(self.render_editor(editor, cx));
        } else {
            if self.loading {
                content = content.child("Loading notes…");
            } else if self.notes.is_empty() && self.error.is_none() && self.load_error.is_none() {
                content = content.child("No notes yet.");
            }
            for entry in &self.notes {
                let id = entry.note.id;
                let edit_content = entry.note.content.clone();
                let confirming = self.confirming_delete == Some(id);
                content = content.child(
                    v_flex()
                        .id(SharedString::from(format!("note-{id}")))
                        .test_support()
                        .gap_2()
                        .border_t_1()
                        .border_color(cx.theme().input)
                        .pt_2()
                        .child(
                            h_flex()
                                .gap_1()
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_color(note_kind_color(
                                            entry.note.content.kind,
                                            cx.theme().is_dark(),
                                        ))
                                        .child(entry.note.content.kind.label()),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("edit-note-{id}")))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::PenLine)
                                        .accessibility_label("Edit note")
                                        .tooltip("Edit note")
                                        .disabled(self.pending)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.begin_edit(
                                                Some(id),
                                                edit_content.clone(),
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("delete-note-{id}")))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Trash)
                                        .accessibility_label("Delete note")
                                        .tooltip("Delete note")
                                        .disabled(self.pending)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.confirming_delete = Some(id);
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(TextView::new(&entry.markdown).style(
                            TextViewStyle::default().heading_font_size(|level, base| {
                                base * match level {
                                    1 => 1.2,
                                    2 => 1.1,
                                    _ => 1.0,
                                }
                            }),
                        ))
                        .children(entry.note.content.locations.iter().map(|location| {
                            div().text_color(cx.theme().muted_foreground).child(format!(
                                "{} {}",
                                location.kind.label(),
                                location.value
                            ))
                        }))
                        .when(confirming, |card| {
                            card.child(
                                v_flex().gap_2().child("Delete this note?").child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "confirm-delete-note-{id}"
                                            )))
                                            .small()
                                            .label("Delete note")
                                            .disabled(self.pending)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.delete(id, window, cx)
                                            })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "cancel-delete-note-{id}"
                                            )))
                                            .small()
                                            .label("Cancel")
                                            .disabled(self.pending)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.confirming_delete = None;
                                                cx.notify();
                                            })),
                                        ),
                                ),
                            )
                        }),
                );
            }
        }
        content.when_some(
            self.error.clone().or_else(|| {
                self.editor
                    .is_none()
                    .then(|| self.load_error.clone())
                    .flatten()
            }),
            |content, error| {
                content
                    .child(
                        div()
                            .id("note-error")
                            .aria_label(error.clone())
                            .test_support()
                            .text_color(cx.theme().danger)
                            .child(error),
                    )
                    .when(self.editor.is_none() && !self.pending, |content| {
                        content.child(
                            Button::new("notes-retry").small().label("Retry").on_click(
                                cx.listener(|this, _, window, cx| this.reload(window, cx)),
                            ),
                        )
                    })
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{NotesPanel, note_kind_color};
    use crate::db::DbWorker;
    use gpui_kit::component::ThemeMode;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext};
    use ibex_core::{NoteContent, NoteKind, Store as _};

    #[test]
    fn note_kinds_use_distinct_colors() {
        let light: Vec<_> = NoteKind::ALL
            .into_iter()
            .map(|kind| note_kind_color(kind, false))
            .collect();
        for (i, color) in light.iter().enumerate() {
            for other in &light[i + 1..] {
                assert_ne!(color, other);
            }
        }
        for kind in NoteKind::ALL {
            let light = note_kind_color(kind, false);
            let dark = note_kind_color(kind, true);
            assert!(dark.l > light.l, "{kind:?} must lighten in dark mode");
            assert!(
                (dark.h - light.h).abs() < 0.04,
                "{kind:?} must keep its hue across modes"
            );
        }
    }

    #[gpui_kit::test]
    fn saved_notes_render_markdown_in_both_themes(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let store = ibex_app::testing::MemoryStore::new();
        let resource = rt.block_on(async {
            // Index 0 stays empty: demo notes attach only to the newest tail.
            ibex_app::seed::seed_demo_library(&store, 12).await.unwrap();
            store
                .list_resources()
                .await
                .unwrap()
                .into_iter()
                .find(|row| row.title.as_str() == "Citation Networks 000")
                .unwrap()
                .id
        });
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(store)
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        rt.block_on(worker.create_note(resource, NoteContent {
            body_markdown: "# Finding\n\nA **bold** claim with [a link](https://example.com).\n\n> Quoted text".into(),
            ..Default::default()
        })).unwrap();
        let (handle, panel) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut panel = NotesPanel::new(worker.clone());
                    panel.select(resource, window, cx);
                    panel
                })
            })
            .unwrap()
        });
        for _ in 0..3 {
            cx.run_until_parked();
            worker.list_page_blocking(None, 1).unwrap();
        }
        cx.run_until_parked();
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            cx.update_window(handle, |_, window, cx| {
                crate::theme::apply(mode, &crate::settings::ThemeSettings::default(), cx);
                window.render_frame(cx);
                let panel = panel.read(cx);
                assert_eq!(panel.notes.len(), 1);
                let rendered = panel.notes[0].markdown.read(cx).rendered_text();
                assert!(rendered.as_str().contains("Finding"));
                assert!(rendered.as_str().contains("A bold claim with a link."));
                assert!(rendered.as_str().contains("Quoted text"));
                assert!(!rendered.as_str().contains("**"));
                assert!(!rendered.as_str().contains("# Finding"));
            })
            .unwrap();
        }
    }
}
