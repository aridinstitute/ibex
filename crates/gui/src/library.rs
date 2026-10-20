//! Library list: resource summary rows, one page at a time.

use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::{ActiveTheme as _, IndexPath, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use oeuvre_core::{ResourceId, ResourcePage, ResourceSummary};

use crate::db::DbWorker;

/// Rows fetched per scroll trigger. Small enough to feel incremental, large
/// enough that the demo library spans several pages.
pub const PAGE_SIZE: u32 = 50;

pub struct LibraryDelegate {
    worker: DbWorker,
    rows: Vec<ResourceSummary>,
    selected: Option<IndexPath>,
    loading: bool,
    next_before: Option<ResourceId>,
    error: Option<String>,
}

impl LibraryDelegate {
    /// Start from a first page fetched before the window opens, so the list
    /// never renders an empty bootstrap state.
    pub fn new(worker: DbWorker, first: ResourcePage) -> Self {
        Self {
            worker,
            rows: first.rows,
            selected: None,
            loading: false,
            next_before: first.next_before,
            error: None,
        }
    }

    pub fn count(&self) -> usize {
        self.rows.len()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl ListDelegate for LibraryDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.rows.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let row = self.rows.get(ix.row)?;
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let border = theme.border;

        Some(
            ListItem::new(ix)
                .selected(self.selected == Some(ix))
                .accessibility_label(format!(
                    "{} {}",
                    row.title.as_str(),
                    row.creators_label
                ))
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .min_w_0()
                        .child(
                            div()
                                .flex_shrink_0()
                                .px_1()
                                .rounded_sm()
                                .text_xs()
                                .border_1()
                                .border_color(border)
                                .text_color(muted)
                                .child(row.kind.as_str()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .overflow_x_hidden()
                                .child(row.title.as_str().to_owned()),
                        )
                        .when(!row.creators_label.is_empty(), |el| {
                            el.child(
                                div()
                                    .flex_shrink_0()
                                    .overflow_x_hidden()
                                    .text_color(muted)
                                    .child(row.creators_label.clone()),
                            )
                        })
                        .when_some(row.published_label.clone(), |el, year| {
                            el.child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(muted)
                                    .child(year.as_str().to_owned()),
                            )
                        }),
                ),
        )
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        self.selected = ix;
        cx.notify();
    }

    fn loading(&self, _cx: &App) -> bool {
        self.loading
    }

    fn has_more(&self, _cx: &App) -> bool {
        !self.loading && self.next_before.is_some()
    }

    fn load_more_threshold(&self) -> usize {
        10
    }

    fn load_more(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        // The list fires load_more repeatedly near the bottom, so loading
        // acts as the lock and a None cursor marks the end.
        if self.loading || self.next_before.is_none() {
            return;
        }
        self.loading = true;
        self.error = None;
        let worker = self.worker.clone();
        let before = self.next_before;
        cx.notify();

        cx.spawn_in(window, async move |view, window| {
            let page = worker.list_page(before, PAGE_SIZE).await;
            _ = view.update_in(window, |view, _window, cx| {
                let delegate = view.delegate_mut();
                delegate.loading = false;
                match page {
                    Ok(page) => {
                        delegate.rows.extend(page.rows);
                        delegate.next_before = page.next_before;
                    }
                    Err(err) => delegate.error = Some(err),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

pub struct LibraryApp {
    list: Entity<ListState<LibraryDelegate>>,
}

impl LibraryApp {
    pub fn new(list: Entity<ListState<LibraryDelegate>>) -> Self {
        Self { list }
    }
}

impl Render for LibraryApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let delegate = self.list.read(cx).delegate();
        let count = delegate.count();
        let error = delegate.error().map(str::to_owned);
        let theme = cx.theme();

        v_flex()
            .size_full()
            .p_3()
            .gap_2()
            .child(
                h_flex()
                    .items_baseline()
                    .gap_2()
                    .flex_shrink_0()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Oeuvre"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(format!("{count} resources")),
                    ),
            )
            .child(List::new(&self.list).flex_1().w_full().min_h_0())
            .when_some(error, |el, error| {
                el.child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(theme.danger)
                        .child(error),
                )
            })
    }
}
