//! Library grid: resource summary rows, one page at a time.

use gpui_kit::component::table::{Column, DataTable, TableDelegate, TableState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, Size, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use oeuvre_core::{ResourceId, ResourcePage, ResourceSummary};

use crate::db::DbWorker;
use crate::kind::{kind_icon, kind_label};

/// Rows fetched per scroll trigger. Small enough to feel incremental, large
/// enough that the demo library spans several pages.
pub const PAGE_SIZE: u32 = 50;

const COL_KIND: usize = 0;
const COL_TITLE: usize = 1;
const COL_CREATORS: usize = 2;
const COL_PUBLISHED: usize = 3;
const COLUMNS: usize = 4;

const CELL_TEXT_SIZE: Pixels = px(13.);
/// Header labels are one step below the cell text: the table default is 9px
/// (`text_xs` at the 12px rem), which reads as an afterthought.
const HEADER_TEXT_SIZE: Pixels = px(11.);

pub struct LibraryDelegate {
    worker: DbWorker,
    rows: Vec<ResourceSummary>,
    loading: bool,
    next_before: Option<ResourceId>,
    error: Option<String>,
}

impl LibraryDelegate {
    /// Start from a first page fetched before the window opens, so the grid
    /// never renders an empty bootstrap state.
    pub fn new(worker: DbWorker, first: ResourcePage) -> Self {
        Self {
            worker,
            rows: first.rows,
            loading: false,
            next_before: first.next_before,
            error: None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl TableDelegate for LibraryDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        COLUMNS
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        match col_ix {
            COL_KIND => Column::new("kind", "")
                .width(px(28.))
                .resizable(false)
                .text_center(),
            COL_TITLE => Column::new("title", "Title").width(px(620.)).min_width(px(160.)),
            COL_CREATORS => Column::new("creators", "Authors").width(px(260.)).min_width(px(80.)),
            COL_PUBLISHED => Column::new("published", "Published").width(px(80.)).min_width(px(50.)),
            _ => unreachable!("columns_count is {COLUMNS}"),
        }
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        // The default header cell is a full-height block div, so its label
        // sits at the top of the header row. Center it vertically, and center
        // the kind column horizontally to match its cells.
        let header = h_flex()
            .size_full()
            .items_center()
            .text_size(HEADER_TEXT_SIZE);
        let header = if col_ix == COL_KIND {
            header.justify_center()
        } else {
            header
        };
        header.child(self.column(col_ix, cx).name.clone())
    }
    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        // The table's cell is a plain block div, so content sits at the top
        // unless each cell centers it itself.
        let cell = h_flex()
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .items_center()
            .text_size(CELL_TEXT_SIZE);
        let Some(row) = self.rows.get(row_ix) else {
            return cell;
        };
        let muted = cx.theme().muted_foreground;
        match col_ix {
            COL_KIND => {
                let label: SharedString = kind_label(row.kind).into();
                cell.justify_center().child(
                    div()
                        .id(("kind", row_ix))
                        .text_color(muted)
                        .child(Icon::new(kind_icon(row.kind)).with_size(Size::Small))
                        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx)),
                )
            }
            COL_TITLE => cell.child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(row.title.as_str().to_owned()),
            ),
            COL_CREATORS => cell.text_color(muted).child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(row.creators_label.clone()),
            ),
            _ => cell
                .text_color(muted)
                .children(row.published_label.as_ref().map(|p| p.as_str().to_owned())),
        }
    }
    fn cell_text(&self, row_ix: usize, col_ix: usize, _cx: &App) -> String {
        let Some(row) = self.rows.get(row_ix) else {
            return String::new();
        };
        match col_ix {
            COL_KIND => kind_label(row.kind),
            COL_TITLE => row.title.as_str().to_owned(),
            COL_CREATORS => row.creators_label.clone(),
            _ => row
                .published_label
                .as_ref()
                .map(|p| p.as_str().to_owned())
                .unwrap_or_default(),
        }
    }

    fn loading(&self, _cx: &App) -> bool {
        // The grid swaps every row for a skeleton while this is true, so a
        // background page fetch must not report it once rows are showing.
        self.rows.is_empty() && self.loading
    }

    fn has_more(&self, _cx: &App) -> bool {
        !self.loading && self.next_before.is_some()
    }

    fn load_more_threshold(&self) -> usize {
        10
    }

    fn load_more(&mut self, window: &mut Window, cx: &mut Context<TableState<Self>>) {
        // The grid fires load_more repeatedly near the bottom, so loading
        // acts as the lock and a None cursor marks the end.
        if self.loading || self.next_before.is_none() {
            return;
        }
        self.loading = true;
        self.error = None;
        let worker = self.worker.clone();
        let before = self.next_before;

        cx.spawn_in(window, async move |table, window| {
            let page = worker.list_page(before, PAGE_SIZE).await;
            _ = table.update_in(window, |table, _window, cx| {
                let delegate = table.delegate_mut();
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
    table: Entity<TableState<LibraryDelegate>>,
}

impl LibraryApp {
    pub fn new(table: Entity<TableState<LibraryDelegate>>) -> Self {
        Self { table }
    }
}

impl Render for LibraryApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let error = self.table.read(cx).delegate().error().map(str::to_owned);
        let danger = cx.theme().danger;

        v_flex()
            .size_full()
            .child(
                DataTable::new(&self.table)
                    .with_size(Size::XSmall)
                    .bordered(false)
                    .stripe(true),
            )
            .when_some(error, |el, error| {
                el.child(
                    div()
                        .flex_shrink_0()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(danger)
                        .child(error),
                )
            })
    }
}
