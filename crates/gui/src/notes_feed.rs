//! Cross-resource notes feed: every note, newest activity first.
//!
//! Typography-first cards: colored kind label, resource title beside it with a
//! middle dot, and the resource as a quiet click-to-filter affordance.
use crate::db::DbWorker;
use crate::notes::note_kind_color;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::text::{TextView, TextViewState, TextViewStyle};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ibex_core::{NoteFeedItem, ResourceId, format_recorded_date};

struct FeedEntry {
    item: NoteFeedItem,
    markdown: Entity<TextViewState>,
}

#[derive(Clone)]
struct FilterOption {
    id: ResourceId,
    title: SharedString,
}

pub struct NotesFeed {
    worker: DbWorker,
    generation: u64,
    entries: Vec<FeedEntry>,
    filter_options: Vec<FilterOption>,
    filter_resource: Option<ResourceId>,
    loading: bool,
    error: Option<String>,
}

impl NotesFeed {
    pub fn new(worker: DbWorker) -> Self {
        Self {
            worker,
            generation: 0,
            entries: Vec::new(),
            filter_options: Vec::new(),
            filter_resource: None,
            loading: false,
            error: None,
        }
    }

    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_resource = None;
        self.reload(window, cx);
    }

    pub fn set_resource_filter(
        &mut self,
        resource: Option<ResourceId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.filter_resource == resource {
            return;
        }
        self.filter_resource = resource;
        self.reload(window, cx);
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let filter = self.filter_resource;
        let worker = self.worker.clone();
        self.loading = true;
        self.error = None;
        cx.spawn_in(window, async move |view, window| {
            let result = worker.list_note_feed(filter).await;
            // When filtered, also refresh the option list from the full feed
            // so the picker still lists every resource that has notes.
            let options_result = if filter.is_some() {
                Some(worker.list_note_feed(None).await)
            } else {
                None
            };
            _ = view.update_in(window, |this, _, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(items) => {
                        if let Some(Ok(all)) = options_result.as_ref() {
                            this.filter_options = filter_options_from(all);
                        } else if filter.is_none() {
                            this.filter_options = filter_options_from(&items);
                        }
                        this.entries = items
                            .into_iter()
                            .map(|item| {
                                let markdown = cx.new(|cx| {
                                    TextViewState::markdown(&item.note.content.body_markdown, cx)
                                });
                                FeedEntry { item, markdown }
                            })
                            .collect();
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn filter_options_from(items: &[NoteFeedItem]) -> Vec<FilterOption> {
    let mut seen = std::collections::HashSet::new();
    let mut options = Vec::new();
    for item in items {
        let Some(id) = item.note.resource_id else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        let title = item
            .resource_title
            .as_ref()
            .map(|t| t.as_str())
            .unwrap_or("Untitled resource");
        options.push(FilterOption {
            id,
            title: SharedString::from(title.to_owned()),
        });
    }
    options.sort_by_key(|a| a.title.to_lowercase());
    options
}

impl Render for NotesFeed {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.weak_entity();
        let dark = cx.theme().is_dark();
        let filter_label = self
            .filter_resource
            .and_then(|id| {
                self.filter_options
                    .iter()
                    .find(|option| option.id == id)
                    .map(|option| option.title.to_string())
            })
            .unwrap_or_else(|| "All resources".into());

        let mut content = v_flex()
            .id("notes-feed")
            .test_support()
            .size_full()
            .gap_4()
            .p_5()
            .child(
                h_flex()
                    .justify_between()
                    .items_start()
                    .gap_3()
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Notes"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Sorted by date modified"),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new("notes-feed-resource-filter")
                                    .ghost()
                                    .small()
                                    .label(filter_label)
                                    .dropdown_menu_with_anchor(
                                        Anchor::TopRight,
                                        move |mut menu, window, cx| {
                                            let Some(owner) = owner.upgrade() else {
                                                return menu;
                                            };
                                            menu = menu.item(
                                                PopupMenuItem::new("All resources").on_click(
                                                    window.listener_for(
                                                        &owner,
                                                        |this, _, window, cx| {
                                                            this.set_resource_filter(
                                                                None, window, cx,
                                                            );
                                                        },
                                                    ),
                                                ),
                                            );
                                            let options = owner.read(cx).filter_options.clone();
                                            for option in options {
                                                let id = option.id;
                                                menu = menu.item(
                                                    PopupMenuItem::new(option.title).on_click(
                                                        window.listener_for(
                                                            &owner,
                                                            move |this, _, window, cx| {
                                                                this.set_resource_filter(
                                                                    Some(id),
                                                                    window,
                                                                    cx,
                                                                );
                                                            },
                                                        ),
                                                    ),
                                                );
                                            }
                                            menu
                                        },
                                    ),
                            )
                            .when(self.filter_resource.is_some(), |row| {
                                row.child(
                                    Button::new("notes-feed-clear-filter")
                                        .ghost()
                                        .xsmall()
                                        .label("Clear")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.set_resource_filter(None, window, cx);
                                        })),
                                )
                            }),
                    ),
            );

        if self.loading && self.entries.is_empty() {
            content = content.child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child("Loading notes…"),
            );
        } else if self.entries.is_empty() && self.error.is_none() {
            content = content.child(div().text_color(cx.theme().muted_foreground).child(
                if self.filter_resource.is_some() {
                    "No notes for this resource."
                } else {
                    "No notes yet."
                },
            ));
        }

        for entry in &self.entries {
            let id = entry.item.note.id;
            let resource_id = entry.item.note.resource_id;
            let resource_title = entry
                .item
                .resource_title
                .as_ref()
                .map(|t| t.as_str().to_owned())
                .unwrap_or_else(|| "Untitled resource".into());
            let stamp = format_recorded_date(entry.item.note.updated_at);
            let kind = entry.item.note.content.kind;
            let locations = entry.item.note.content.locations.clone();
            let muted = cx.theme().muted_foreground;
            let hover = cx.theme().foreground;
            let meta = h_flex()
                .gap_1()
                .items_center()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(note_kind_color(kind, dark))
                        .child(kind.label()),
                )
                .child(div().text_size(px(12.)).text_color(muted).child("·"))
                .child(match resource_id {
                    Some(resource_id) => div()
                        .id(SharedString::from(format!("feed-resource-{id}")))
                        .test_support()
                        .min_w_0()
                        .flex_shrink(1.)
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(12.))
                        .text_color(muted)
                        .cursor_pointer()
                        .hover(move |style| style.text_color(hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.set_resource_filter(Some(resource_id), window, cx);
                        }))
                        .child(resource_title)
                        .into_any_element(),
                    None => div()
                        .min_w_0()
                        .flex_shrink(1.)
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(12.))
                        .text_color(muted)
                        .child(resource_title)
                        .into_any_element(),
                });
            content = content.child(
                v_flex()
                    .id(SharedString::from(format!("feed-note-{id}")))
                    .test_support()
                    .gap_2()
                    .w_full()
                    .flex_shrink_0()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .px_4()
                    .py_3()
                    .child(
                        h_flex().gap_2().items_center().min_w_0().child(meta).child(
                            div()
                                .text_size(px(11.))
                                .text_color(cx.theme().muted_foreground)
                                .child(stamp),
                        ),
                    )
                    .child(TextView::new(&entry.markdown).style(
                        TextViewStyle::default().heading_font_size(|level, base| {
                            base * match level {
                                1 => 1.15,
                                2 => 1.08,
                                _ => 1.0,
                            }
                        }),
                    ))
                    .children(locations.into_iter().map(|location| {
                        div()
                            .text_size(px(11.))
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{} {}", location.kind.label(), location.value))
                    })),
            );
        }

        content.when_some(self.error.clone(), |content, error| {
            content
                .child(
                    div()
                        .id("notes-feed-error")
                        .aria_label(error.clone())
                        .test_support()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
                .child(
                    Button::new("notes-feed-retry")
                        .small()
                        .label("Retry")
                        .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
                )
        })
    }
}
