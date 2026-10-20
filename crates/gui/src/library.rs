//! Library grid: resource summary rows, one page at a time.

use gpui_kit::component::table::{Column, DataTable, TableDelegate, TableEvent, TableState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, Size, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use oeuvre_core::{ResourceId, ResourcePage, ResourceSort, ResourceSummary, SortDirection};

use crate::db::DbWorker;
use crate::detail::{render_panel, DetailFetch, DetailView};
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

/// Continuation for the next scroll page, keyed to the listing mode: the
/// newest-first listing walks id cursors, a sorted listing walks offsets.
#[derive(Debug, Clone, Copy)]
enum PageCursor {
    NewestFirst(Option<ResourceId>),
    Sorted(Option<u64>),
}

pub struct LibraryDelegate {
    worker: DbWorker,
    rows: Vec<ResourceSummary>,
    loading: bool,
    next: PageCursor,
    /// Column sort driving the listing; `None` is the newest-first default.
    sort: Option<(ResourceSort, SortDirection)>,
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
            next: PageCursor::NewestFirst(first.next_before),
            sort: None,
            error: None,
        }
    }

    /// Apply a double-clicked header's next cycle state (see [`next_sort`]).
    /// A changed order reloads the grid from the top of the new listing, and
    /// the row highlight follows the selected resource by id once the first
    /// page lands.
    fn request_sort(
        &mut self,
        field: ResourceSort,
        selected_id: Option<ResourceId>,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        // One reload in flight is enough: a double-click during the fetch
        // lands on the completed order instead of stacking requests.
        if self.loading {
            return;
        }
        self.sort = next_sort(self.sort, field);
        self.loading = true;
        self.error = None;
        let worker = self.worker.clone();
        let sort = self.sort;
        cx.spawn_in(window, async move |table, window| {
            let reply = match sort {
                Some((field, direction)) => worker
                    .list_sorted_page(field, direction, 0, PAGE_SIZE)
                    .await
                    .map(|page| (page.rows, PageCursor::Sorted(page.next_offset))),
                None => worker
                    .list_page(None, PAGE_SIZE)
                    .await
                    .map(|page| (page.rows, PageCursor::NewestFirst(page.next_before))),
            };
            _ = table.update_in(window, |table, _window, cx| {
                let delegate = table.delegate_mut();
                delegate.loading = false;
                match reply {
                    Ok((rows, next)) => {
                        delegate.rows = rows;
                        delegate.next = next;
                    }
                    Err(err) => delegate.error = Some(err),
                }
                // The highlight follows the selected resource to its new
                // row; a resource outside the first page loses it and the
                // detail panel returns to empty.
                let selected_ix = selected_id
                    .and_then(|id| table.delegate().rows.iter().position(|row| row.id == id));
                match selected_ix {
                    Some(ix) => table.set_selected_row(ix, cx),
                    None => table.clear_selection(cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The id backing a row, so selection events can drive id-keyed fetches.
    pub fn row_id(&self, row_ix: usize) -> Option<ResourceId> {
        self.rows.get(row_ix).map(|row| row.id)
    }

    pub fn worker(&self) -> &DbWorker {
        &self.worker
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
        let name = self.column(col_ix, cx).name.clone();
        let Some(field) = sort_field_for_col(col_ix) else {
            return header.child(name).into_any_element();
        };
        // Which direction of this column's cycle currently drives the order;
        // `None` leaves the header without a marker.
        let direction = match self.sort {
            Some((active, direction)) if active == field => Some(direction),
            _ => None,
        };
        // Double-click steps the cycle; a single click keeps flowing to the
        // table's column selection.
        header
            .id(("th-sort", col_ix))
            .on_click(cx.listener(move |table, event: &ClickEvent, window, cx| {
                if event.click_count() != 2 {
                    return;
                }
                // The second click drives the sort, not another selection.
                cx.stop_propagation();
                let selected_id =
                    table.selected_row().and_then(|ix| table.delegate().row_id(ix));
                table
                    .delegate_mut()
                    .request_sort(field, selected_id, window, cx);
            }))
            .when_some(
                direction.map(|direction| sort_marker(direction, cx)),
                |el, marker| el.child(marker),
            )
            .child(name)
            .into_any_element()
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
        !self.loading
            && match self.next {
                PageCursor::NewestFirst(next) => next.is_some(),
                PageCursor::Sorted(next) => next.is_some(),
            }
    }

    fn load_more_threshold(&self) -> usize {
        10
    }

    fn load_more(&mut self, window: &mut Window, cx: &mut Context<TableState<Self>>) {
        // The grid fires load_more repeatedly near the bottom, so loading
        // acts as the lock and an exhausted cursor marks the end.
        if self.loading
            || !matches!(
                self.next,
                PageCursor::NewestFirst(Some(_)) | PageCursor::Sorted(Some(_))
            )
        {
            return;
        }
        self.loading = true;
        self.error = None;
        let worker = self.worker.clone();
        match (self.sort, self.next) {
            (None, PageCursor::NewestFirst(Some(before))) => {
                cx.spawn_in(window, async move |table, window| {
                    let page = worker.list_page(Some(before), PAGE_SIZE).await;
                    _ = table.update_in(window, |table, _window, cx| {
                        let delegate = table.delegate_mut();
                        delegate.loading = false;
                        match page {
                            Ok(page) => {
                                delegate.rows.extend(page.rows);
                                delegate.next = PageCursor::NewestFirst(page.next_before);
                            }
                            Err(err) => delegate.error = Some(err),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            (Some((field, direction)), PageCursor::Sorted(Some(offset))) => {
                cx.spawn_in(window, async move |table, window| {
                    let page = worker.list_sorted_page(field, direction, offset, PAGE_SIZE).await;
                    _ = table.update_in(window, |table, _window, cx| {
                        let delegate = table.delegate_mut();
                        delegate.loading = false;
                        match page {
                            Ok(page) => {
                                delegate.rows.extend(page.rows);
                                delegate.next = PageCursor::Sorted(page.next_offset);
                            }
                            Err(err) => delegate.error = Some(err),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            // Cursor mode and sort stay in lockstep (request_sort swaps
            // both); a mismatch is a state bug, so release the lock and
            // stop rather than wedge future loads.
            _ => self.loading = false,
        }
    }
}

/// Column that double-click cycles, by header index; `None` for the kind
/// column, whose icon header stays inert.
fn sort_field_for_col(col_ix: usize) -> Option<ResourceSort> {
    match col_ix {
        COL_TITLE => Some(ResourceSort::Title),
        COL_CREATORS => Some(ResourceSort::PrimaryCreator),
        COL_PUBLISHED => Some(ResourceSort::Published),
        _ => None,
    }
}

/// One header's three-state cycle: newest-first → ascending → descending →
/// back to newest-first. Double-clicking a different column always restarts
/// that column ascending.
fn next_sort(
    current: Option<(ResourceSort, SortDirection)>,
    field: ResourceSort,
) -> Option<(ResourceSort, SortDirection)> {
    match current {
        Some((active, SortDirection::Ascending)) if active == field => {
            Some((field, SortDirection::Descending))
        }
        Some((active, SortDirection::Descending)) if active == field => None,
        _ => Some((field, SortDirection::Ascending)),
    }
}

/// Small direction marker beside the header label of the column that
/// currently drives the order.
fn sort_marker(direction: SortDirection, cx: &App) -> Icon {
    Icon::new(match direction {
        SortDirection::Ascending => IconName::SortAscending,
        SortDirection::Descending => IconName::SortDescending,
    })
    .with_size(Size::Small)
    .text_color(cx.theme().muted_foreground)
}

gpui_kit::actions!([ToggleDetailPanel]);

pub struct LibraryApp {
    table: Entity<TableState<LibraryDelegate>>,
    detail_open: bool,
    /// Row selection, tracked here so the panel shows one resource exactly
    /// as long as its row stays selected.
    selected: Option<ResourceId>,
    /// Detail for the selected resource, keyed by id: a late reply from a
    /// superseded selection is dropped instead of flashing into the panel.
    detail: Option<(ResourceId, DetailFetch)>,
}

impl LibraryApp {
    pub fn new(
        table: Entity<TableState<LibraryDelegate>>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe_in(&table, window, |this, _table, event: &TableEvent, window, cx| {
            match event {
                TableEvent::SelectRow(row_ix) => this.select_row(*row_ix, window, cx),
                TableEvent::ClearSelection => this.clear_selection(cx),
                _ => {}
            }
        })
        .detach();
        Self {
            table,
            detail_open: false,
            selected: None,
            detail: None,
        }
    }

    fn select_row(&mut self, row_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.table.read(cx).delegate().row_id(row_ix) else {
            return;
        };
        self.selected = Some(id);
        if self.detail.as_ref().is_some_and(|(loaded, _)| *loaded == id) {
            cx.notify();
            return;
        }
        self.detail = Some((id, DetailFetch::Loading));
        cx.notify();

        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let reply = worker.resource_detail(id).await;
            _ = app.update_in(window, |app, _window, cx| {
                // A newer selection wins; a stale reply is dropped.
                if app.selected == Some(id) {
                    app.detail = Some((
                        id,
                        match reply {
                            Ok(detail) => DetailFetch::Loaded(Box::new(detail)),
                            Err(error) => DetailFetch::Failed(error),
                        },
                    ));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        cx.notify();
    }

    fn toggle_detail_panel(&mut self, cx: &mut Context<Self>) {
        // The detail is fetched on selection whether or not the panel shows,
        // so opening it never waits on a fetch.
        self.detail_open = !self.detail_open;
        cx.notify();
    }

    fn detail_view(&self) -> DetailView<'_> {
        match (self.selected, self.detail.as_ref()) {
            (Some(id), Some((loaded, fetch))) if *loaded == id => match fetch {
                DetailFetch::Loading => DetailView::Loading,
                DetailFetch::Loaded(detail) => DetailView::Fields(detail),
                DetailFetch::Failed(error) => DetailView::Failed(error),
            },
            // select_row always starts a fetch, so this only covers the gap
            // between the two statements; the panel just shows loading.
            (Some(_), _) => DetailView::Loading,
            (None, _) => DetailView::Empty,
        }
    }
}

impl Render for LibraryApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let error = self.table.read(cx).delegate().error().map(str::to_owned);
        let danger = cx.theme().danger;

        v_flex()
            .size_full()
            // The overlaid panel anchors to this root.
            .relative()
            // `]` toggles the detail panel; the context must cover the
            // focused table, which renders below this root.
            .key_context("LibraryApp")
            .on_action(cx.listener(|this, _: &ToggleDetailPanel, _window, cx| {
                this.toggle_detail_panel(cx);
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        DataTable::new(&self.table)
                            .with_size(Size::XSmall)
                            .bordered(false)
                            .stripe(true),
                    ),
            )
            // The panel overlays the grid instead of sharing a row with it,
            // so narrow windows hide columns rather than cut the panel off.
            .when(self.detail_open, |el| {
                el.child(render_panel(self.detail_view(), cx))
            })
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

#[cfg(test)]
mod sort_cycle_tests {
    // No `use super::*`: the glob would re-import gpui's `test` proc
    // macro (via the module's `gpui_kit::*`), hijacking `#[test]`.
    use super::{next_sort, sort_field_for_col, COL_CREATORS, COL_KIND, COL_PUBLISHED, COL_TITLE};
    use oeuvre_core::{ResourceSort, SortDirection};

    #[test]
    fn one_header_cycles_ascending_descending_then_reverts() {
        let field = ResourceSort::Title;
        let asc = next_sort(None, field);
        assert_eq!(asc, Some((field, SortDirection::Ascending)));
        let desc = next_sort(asc, field);
        assert_eq!(desc, Some((field, SortDirection::Descending)));
        let reverted = next_sort(desc, field);
        assert_eq!(reverted, None);
    }

    #[test]
    fn switching_columns_restarts_ascending() {
        for current in [
            Some((ResourceSort::Title, SortDirection::Ascending)),
            Some((ResourceSort::Title, SortDirection::Descending)),
        ] {
            assert_eq!(
                next_sort(current, ResourceSort::PrimaryCreator),
                Some((ResourceSort::PrimaryCreator, SortDirection::Ascending))
            );
        }
    }

    #[test]
    fn header_columns_map_to_sort_fields() {
        assert_eq!(sort_field_for_col(COL_TITLE), Some(ResourceSort::Title));
        assert_eq!(
            sort_field_for_col(COL_CREATORS),
            Some(ResourceSort::PrimaryCreator)
        );
        assert_eq!(
            sort_field_for_col(COL_PUBLISHED),
            Some(ResourceSort::Published)
        );
        assert_eq!(sort_field_for_col(COL_KIND), None);
    }
}
