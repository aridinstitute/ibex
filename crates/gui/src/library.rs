//! Library grid: resource summary rows, one page at a time.

use std::{collections::{BTreeMap, HashMap, HashSet, VecDeque}, ops::Range, path::PathBuf};

use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::combobox::{Combobox, ComboboxEvent, ComboboxState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::table::{Column, DataTable, TableDelegate, TableEvent, TableState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, FocusableExt as _, Icon, IconName, IndexPath, Selectable as _, Sizable as _,
    Size, ThemeColor, ThemeMode, TitleBar, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use oeuvre_core::{
    NonEmptyStr, ParentRule, Person, PersonId, PublishedRange, Resource, ResourceDetail,
    ResourceFilter, ResourceId, ResourceKind, ResourcePage, ResourceQuery, ResourceSort,
    ResourceSummary, SortDirection, UtcMillis,
};

use crate::db::DbWorker;
use crate::detail::{
    render_panel, DetailFetch, DetailView, FIELD_LABEL_WIDTH, FIELD_TEXT_SIZE, PANEL_WIDTH,
};
use crate::edit::{EditDraft, KindItem, build_update, kind_items, prefill_draft};
use crate::icons::symbol;
use crate::kind::{kind_color, kind_label, kind_symbol};
use crate::settings::Settings;

/// Rows fetched per scroll trigger. Small enough to feel incremental, large
/// enough that the demo library spans several pages.
pub const PAGE_SIZE: u32 = 50;

/// First page of parent candidates the edit form fetches, already narrowed
/// to the edited kind's expected parent kind at the store. A parent beyond
/// the page is still fetched by id when the record carries one, so every
/// stored parent stays selectable; the cap keeps the form's extra fetch
/// bounded on large libraries.
const PARENT_OPTIONS_LIMIT: u32 = 100;

const COL_KIND: usize = 0;
const COL_TITLE: usize = 1;
const COL_CREATORS: usize = 2;
const COL_PUBLISHED: usize = 3;
const COLUMNS: usize = 4;

/// Stable key for each column, indexed like the COL_ constants above.
/// Settings persist column widths under these names, so widths survive any
/// future column reordering.
const COLUMN_KEYS: [&str; COLUMNS] = ["kind", "title", "creators", "published"];

/// The title column's built-in width, half of its former 620 px, so the
/// grid opens readable on narrower windows.
const DEFAULT_TITLE_WIDTH: f32 = 310.;

const CELL_TEXT_SIZE: Pixels = px(13.);
/// Header labels are one step below the cell text: the table default is 9px
/// (`text_xs` at the 12px rem), which reads as an afterthought.
const HEADER_TEXT_SIZE: Pixels = px(11.);

/// Continuation for the next scroll page, keyed to the listing mode: the
/// plain newest-first listing walks id cursors, a queried listing walks
/// offsets.
#[derive(Debug, Clone, Copy)]
enum PageCursor {
    NewestFirst(Option<ResourceId>),
    Query(Option<u64>),
}

pub struct LibraryDelegate {
    worker: DbWorker,
    rows: Vec<ResourceSummary>,
    loading: bool,
    next: PageCursor,
    /// Column sort driving the listing; `None` is the newest-first default.
    sort: Option<(ResourceSort, SortDirection)>,
    /// Predicates driving the listing; `none()` matches every resource.
    filter: ResourceFilter,
    /// Handles to the filter inputs the header filter row renders; attached
    /// once `LibraryApp` wires its entities, before any window renders.
    filter_inputs: Option<FilterInputs>,
    /// Whether the header shows its filter row; all controls or none.
    filters_visible: bool,
    error: Option<String>,
    /// Mirrors the table's selected row from its selection events, because
    /// `render_td` runs while the table state is borrowed and cannot ask it.
    selected_row: Option<usize>,
    /// Persisted GUI settings: column widths loaded from config.toml ride
    /// through `column`, and finished resizes are written back by
    /// `LibraryApp`.
    settings: Settings,
    /// Bumped on every accepted sort or filter change. Each fetch carries
    /// the generation it was issued for, so a reply landing after a newer
    /// change is dropped instead of clobbering the newer listing.
    listing_gen: u64,
}

impl LibraryDelegate {
    /// Start from a first page fetched before the window opens, so the grid
    /// never renders an empty bootstrap state.
    pub fn new(worker: DbWorker, first: ResourcePage, settings: Settings) -> Self {
        Self {
            worker,
            rows: first.rows,
            loading: false,
            next: PageCursor::NewestFirst(first.next_before),
            sort: None,
            filter: ResourceFilter::none(),
            filter_inputs: None,
            filters_visible: false,
            error: None,
            selected_row: None,
            settings,
            listing_gen: 0,
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
        self.reload_from_top(selected_id, window, cx);
    }

    /// Re-key the listing under a new title predicate. A keystroke must
    /// never be dropped, so unlike a sort click this reloads even while a
    /// fetch is in flight; the listing generation discards whichever reply
    /// the change made stale.
    fn request_title_filter(
        &mut self,
        needle: Option<NonEmptyStr>,
        selected_id: Option<ResourceId>,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        if self.filter.title == needle {
            return;
        }
        self.filter.title = needle;
        self.reload_from_top(selected_id, window, cx);
    }

    /// Re-key the listing under a new creator predicate; mirrors the title
    /// box's never-drop-a-change contract.
    fn request_creator_filter(
        &mut self,
        creator: Option<PersonId>,
        selected_id: Option<ResourceId>,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        if self.filter.creator == creator {
            return;
        }
        self.filter.creator = creator;
        self.reload_from_top(selected_id, window, cx);
    }

    /// Re-key the listing under a new kind predicate; mirrors the title
    /// box's never-drop-a-change contract.
    fn request_kind_filter(
        &mut self,
        kind: Option<ResourceKind>,
        selected_id: Option<ResourceId>,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        if self.filter.kind == kind {
            return;
        }
        self.filter.kind = kind;
        self.reload_from_top(selected_id, window, cx);
    }

    /// Re-key the listing under a new published window; mirrors the title
    /// box's never-drop-a-change contract.
    fn request_published_filter(
        &mut self,
        range: Option<PublishedRange>,
        selected_id: Option<ResourceId>,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        if self.filter.published == range {
            return;
        }
        self.filter.published = range;
        self.reload_from_top(selected_id, window, cx);
    }

    /// Fetch the first page under the current sort and filter and replace
    /// the rows. The fetch carries the generation it serves, and the row
    /// highlight follows the selected resource by id once it lands.
    fn reload_from_top(
        &mut self,
        selected_id: Option<ResourceId>,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        self.listing_gen += 1;
        self.loading = true;
        self.error = None;
        let worker = self.worker.clone();
        let sort = self.sort;
        let filter = self.filter.clone();
        let issued_gen = self.listing_gen;
        cx.spawn_in(window, async move |table, window| {
            // The plain id-cursor walk exists only for the unsorted,
            // unfiltered default; any sort or filter routes through the
            // query seam.
            let reply = if sort.is_none() && !filter.is_active() {
                worker
                    .list_page(None, PAGE_SIZE)
                    .await
                    .map(|page| (page.rows, PageCursor::NewestFirst(page.next_before)))
            } else {
                worker
                    .list_query_page(ResourceQuery { sort, filter }, 0, PAGE_SIZE)
                    .await
                    .map(|page| (page.rows, PageCursor::Query(page.next_offset)))
            };
            _ = table.update_in(window, |table, _window, cx| {
                let delegate = table.delegate_mut();
                // A newer sort or keystroke superseded this fetch; its
                // rows belong to a listing nobody is looking at.
                if delegate.listing_gen != issued_gen {
                    return;
                }
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

    /// The column's cell in the filter row: edge to edge, divided like a
    /// body row, with borderless controls filling it. Clicks into a control
    /// belong to the control: the header's column selection and
    /// double-click sort must never see them.
    fn filter_control(&self, col_ix: usize, cx: &App) -> AnyElement {
        let pad = Size::XSmall.table_cell_padding();
        let cell = h_flex()
            .id(("th-filter", col_ix))
            .w_full()
            .h(Size::XSmall.table_row_height())
            .items_center()
            .px(pad.left)
            .border_t_1()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, _, cx| cx.stop_propagation());
        let (Some(inputs), Some(field)) = (self.filter_inputs.as_ref(), filter_field_for_col(col_ix))
        else {
            return cell.into_any_element();
        };
        match field {
            FilterField::Kind => cell
                // The icon column is 28 px and the trigger carries its own
                // 4 px input padding, so this one cell tightens below the
                // row's usual padding to keep the funnel whole.
                .px(px(2.))
                .child(
                    Combobox::new(&inputs.kind)
                        .xsmall()
                        .appearance(false)
                        .search_placeholder("Search kinds")
                        // The trigger is a funnel icon in the 28 px icon
                        // column: muted with no kind picked, primary while
                        // one narrows the listing — the same metaphor and
                        // glyph as the toolbar's Filters button. A custom
                        // trigger drops the kit's chevron and clear button,
                        // so picking the selected kind again is the clear.
                        .render_trigger(|trigger, _window, cx| {
                            let active = !trigger.selection().is_empty();
                            symbol("line.3.horizontal.decrease", IconName::Settings2)
                                .xsmall()
                                .when(active, |icon| {
                                    icon.text_color(cx.theme().primary)
                                })
                                .when(!active, |icon| {
                                    icon.text_color(cx.theme().muted_foreground)
                                })
                        })
                        // The popup reads kinds, not the funnel it hangs
                        // off: a fixed width keeps it from collapsing to
                        // the icon column's 28 px.
                        .menu_width(px(200.))
                        .w_full(),
                )
                .into_any_element(),
            FilterField::Title => cell
                .child(
                    Input::new(&inputs.title)
                        .xsmall()
                        .appearance(false)
                        .cleanable(true)
                        .w_full(),
                )
                .into_any_element(),
            FilterField::Creators => cell
                .child(
                    Combobox::new(&inputs.creator)
                        .xsmall()
                        .appearance(false)
                        .cleanable(true)
                        .placeholder("Creator")
                        .search_placeholder("Search creators")
                        .w_full(),
                )
                .into_any_element(),
            // The year pair splits the cell with a divider of its own, so
            // the two bounds read as two cells.
            FilterField::Published => cell
                .child(
                    Input::new(&inputs.published_from)
                        .xsmall()
                        .appearance(false)
                        .w_full(),
                )
                .child(div().w(px(1.)).h_full().flex_shrink_0().bg(cx.theme().border))
                .child(Input::new(&inputs.published_to).xsmall().appearance(false).w_full())
                .into_any_element(),
        }
    }
}

impl TableDelegate for LibraryDelegate {
    fn columns_count(&self, _cx: &App) -> usize {        COLUMNS
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        // The table reads its columns only on prepare and refresh, so the
        // width here (saved setting or built-in default) is the width the
        // table opens at.
        match col_ix {
            COL_KIND => Column::new(COLUMN_KEYS[COL_KIND], "")
                .width(px(28.))
                .resizable(false)
                .text_center(),
            COL_TITLE => Column::new(COLUMN_KEYS[COL_TITLE], "Title")
                .width(stored_width(
                    &self.settings.column_widths,
                    COL_TITLE,
                    DEFAULT_TITLE_WIDTH,
                ))
                .min_width(px(160.)),
            COL_CREATORS => Column::new(COLUMN_KEYS[COL_CREATORS], "Authors")
                .width(stored_width(
                    &self.settings.column_widths,
                    COL_CREATORS,
                    260.,
                ))
                .min_width(px(80.)),
            COL_PUBLISHED => Column::new(COLUMN_KEYS[COL_PUBLISHED], "Published")
                .width(stored_width(
                    &self.settings.column_widths,
                    COL_PUBLISHED,
                    140.,
                ))
                .min_width(px(50.)),
            _ => unreachable!("columns_count is {COLUMNS}"),
        }
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let name = self.column(col_ix, cx).name.clone();
        // The default header cell is a full-height block div, so its label
        // sits at the top of the header row. Center it vertically, and center
        // the kind column horizontally to match its cells.
        let label = h_flex()
            .w_full()
            .flex_1()
            .items_center()
            .text_size(HEADER_TEXT_SIZE);
        let label = if col_ix == COL_KIND {
            label.justify_center()
        } else {
            label
        };
        let label = match sort_field_for_col(col_ix) {
            None => label.child(name).into_any_element(),
            Some(field) => {
                // Which direction of this column's cycle currently drives the
                // order; `None` leaves the header without a marker.
                let direction = match self.sort {
                    Some((active, direction)) if active == field => Some(direction),
                    _ => None,
                };
                // Double-click steps the cycle; a single click keeps flowing
                // to the table's column selection. The marker claims the
                // header's right edge, where the kit puts its own sort icon.
                label
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
                    .when(direction.is_some(), |el| el.justify_between())
                    .child(name)
                    .when_some(
                        direction.map(|direction| sort_marker(direction, cx)),
                        |el, marker| el.child(marker),
                    )
                    .into_any_element()
            }
        };
        if !self.filters_visible {
            return v_flex().size_full().child(label);
        }
        // The filter row lives inside each header cell, so every control
        // spans exactly its column through resizes and horizontal scroll.
        // Negative margins cancel the kit's cell padding so the row runs
        // edge to edge as a row of its own; the label takes the padding
        // back.
        let pad = Size::XSmall.table_cell_padding();
        v_flex()
            .flex_1()
            .h(Size::XSmall.table_row_height() * 2.)
            .ml(-pad.left)
            .mr(-pad.right)
            .child(div().flex_1().w_full().flex().px(pad.left).child(label))
            .child(self.filter_control(col_ix, cx))
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
        let (text, muted) = cell_text_colors(self.selected_row == Some(row_ix), cx.theme());
        let cell = cell.text_color(text);
        match col_ix {
            COL_KIND => {
                let label: SharedString = kind_label(row.kind).into();
                // The selected row sits on the accent fill, where a family
                // hue would muddy the blue, so the icon goes monochrome
                // along with the rest of the row.
                let icon_color = if self.selected_row == Some(row_ix) {
                    muted
                } else {
                    kind_color(row.kind, cx.theme().is_dark())
                };
                cell.justify_center().child(
                    div()
                        .id(("kind", row_ix))
                        .text_color(icon_color)
                        .child(kind_symbol(row.kind).with_size(px(18.)))
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
                PageCursor::Query(next) => next.is_some(),
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
                PageCursor::NewestFirst(Some(_)) | PageCursor::Query(Some(_))
            )
        {
            return;
        }
        self.loading = true;
        self.error = None;
        let worker = self.worker.clone();
        let sort = self.sort;
        let filter = self.filter.clone();
        let issued_gen = self.listing_gen;
        match self.next {
            // Cursor kind pins the request shape: the id-cursor walk only
            // exists while the listing is unsorted and unfiltered, because
            // reload_from_top re-routes on any sort or filter.
            PageCursor::NewestFirst(Some(before)) => {
                cx.spawn_in(window, async move |table, window| {
                    let page = worker.list_page(Some(before), PAGE_SIZE).await;
                    _ = table.update_in(window, |table, _window, cx| {
                        let delegate = table.delegate_mut();
                        // A sort or keystroke changed the listing after
                        // this page was requested, so the page belongs to
                        // a superseded listing; drop it rather than append
                        // rows the current filter excludes.
                        if delegate.listing_gen != issued_gen {
                            return;
                        }
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
            PageCursor::Query(Some(offset)) => {
                cx.spawn_in(window, async move |table, window| {
                    let page = worker
                        .list_query_page(ResourceQuery { sort, filter }, offset, PAGE_SIZE)
                        .await;
                    _ = table.update_in(window, |table, _window, cx| {
                        let delegate = table.delegate_mut();
                        if delegate.listing_gen != issued_gen {
                            return;
                        }
                        delegate.loading = false;
                        match page {
                            Ok(page) => {
                                delegate.rows.extend(page.rows);
                                delegate.next = PageCursor::Query(page.next_offset);
                            }
                            Err(err) => delegate.error = Some(err),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            // Unreachable behind the guard above; a future cursor kind must
            // not wedge the loading lock, so release it and stop.
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

/// The filter a column header carries, mirroring the sort mapping above;
/// every column filters its own field.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum FilterField {
    Kind,
    Title,
    Creators,
    Published,
}

fn filter_field_for_col(col_ix: usize) -> Option<FilterField> {
    match col_ix {
        COL_KIND => Some(FilterField::Kind),
        COL_TITLE => Some(FilterField::Title),
        COL_CREATORS => Some(FilterField::Creators),
        COL_PUBLISHED => Some(FilterField::Published),
        _ => None,
    }
}

/// Header height for the filter row's visibility: `None` keeps the kit's
/// single row; shown filters stack a second row of controls under the names.
fn header_row_height(filters_visible: bool) -> Option<Pixels> {
    filters_visible.then(|| Size::XSmall.table_row_height() * 2.)
}

/// The filter inputs the header row renders. `LibraryApp` owns and
/// subscribes to these entities, so every keystroke re-keys the listing
/// through its wiring.
#[derive(Clone)]
struct FilterInputs {
    kind: Entity<ComboboxState<SearchableVec<KindItem>>>,
    title: Entity<InputState>,
    creator: Entity<ComboboxState<SearchableVec<CreatorItem>>>,
    published_from: Entity<InputState>,
    published_to: Entity<InputState>,
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

/// Title-filter text to a predicate: trimmed text becomes the substring to
/// match; empty text clears the predicate.
fn title_predicate(text: &str) -> Option<NonEmptyStr> {
    NonEmptyStr::try_new(text).ok()
}

/// One row per person for the creator filter box, sorted by display name
/// so the dropdown reads alphabetically whatever order the store lists in.
fn creator_options(people: &[Person]) -> Vec<(SharedString, PersonId)> {
    let mut options: Vec<(SharedString, PersonId)> = people
        .iter()
        .map(|person| (person.name.display().into(), person.id))
        .collect();
    options.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    options
}

/// Year text to the January 1 instant that brackets it; empty or
/// unparseable text leaves the bound unset.
fn year_instant(text: &str) -> Option<UtcMillis> {
    let year: i64 = text.trim().parse().ok()?;
    UtcMillis::jan_1_utc(year)
}

/// From/to year text to a window over whole years: the "from" year's
/// January 1 is inclusive, the "to" year's next January 1 exclusive, so
/// a pair brackets January 1 through December 31 exactly.
fn year_range(from: &str, to: &str) -> Option<PublishedRange> {
    let from = year_instant(from);
    let to = to
        .trim()
        .parse::<i64>()
        .ok()
        .and_then(|year| UtcMillis::jan_1_utc(year + 1));
    if from.is_none() && to.is_none() {
        return None;
    }
    Some(PublishedRange { from, to })
}

/// One dropdown row for the creator filter: the name reads, the id keys.
#[derive(Clone)]
struct CreatorItem {
    label: SharedString,
    id: PersonId,
}

impl SearchableListItem for CreatorItem {
    type Value = PersonId;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

/// One dropdown row for the parent box: the title reads, the id keys.
#[derive(Clone)]
struct ParentItem {
    label: SharedString,
    id: ResourceId,
}

impl SearchableListItem for ParentItem {
    type Value = ResourceId;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

/// Parent options from a listing page, narrowed to what the edited record's
/// rule allows: off-kind rows never appear however the page was fetched, the
/// edited record itself is skipped (a resource cannot become its own
/// parent), and the rest sort by title so the dropdown reads
/// alphabetically whatever order the store lists in.
fn parent_options(
    rows: &[ResourceSummary],
    edited: ResourceId,
    rule: ParentRule,
) -> Vec<ParentItem> {
    // A rule that forbids parents offers none at all.
    let (ParentRule::Optional { expected } | ParentRule::Required { expected }) = rule else {
        return Vec::new();
    };
    let mut items: Vec<ParentItem> = rows
        .iter()
        .filter(|row| row.id != edited && row.kind == expected)
        .map(|row| ParentItem {
            label: row.title.as_str().into(),
            id: row.id,
        })
        .collect();
    items.sort_by_key(|item| item.label.to_lowercase());
    items
}

/// Width column `col_ix` opens at: its saved width when settings have one,
/// the built-in default otherwise.
fn stored_width(widths: &BTreeMap<String, f64>, col_ix: usize, default: f32) -> Pixels {
    widths
        .get(COLUMN_KEYS[col_ix])
        .map(|width| px(*width as f32))
        .unwrap_or_else(|| px(default))
}

/// All column widths keyed by column key, as a finished resize reports
/// them: position `col_ix` becomes `COLUMN_KEYS[col_ix]`.
fn widths_by_key(widths: &[Pixels]) -> BTreeMap<String, f64> {
    COLUMN_KEYS
        .iter()
        .zip(widths)
        .map(|(key, width)| ((*key).to_string(), f64::from(f32::from(*width))))
        .collect()
}

/// (primary, secondary) cell text for a row. The selected row sits on a
/// solid accent fill, so both turn to the accent's foreground, with the
/// secondary columns slightly dimmed to keep their hierarchy.
fn cell_text_colors(selected: bool, colors: &ThemeColor) -> (Hsla, Hsla) {
    if selected {
        let on_accent = colors.primary_foreground;
        (on_accent, on_accent.opacity(0.8))
    } else {
        (colors.foreground, colors.muted_foreground)
    }
}

/// Small direction marker beside the header label of the column that
/// currently drives the order.
fn sort_marker(direction: SortDirection, cx: &App) -> Icon {
    (match direction {
        SortDirection::Ascending => symbol("arrow.up", IconName::SortAscending),
        SortDirection::Descending => symbol("arrow.down", IconName::SortDescending),
    })
    .with_size(Size::Small)
    .text_color(cx.theme().muted_foreground)
}

gpui_kit::actions!([ToggleDetailPanel]);

/// Rows on either side of the selection whose details prefetch warms.
/// Arrow-key walking moves one row at a time, so a small radius covers
/// the next few steps while keeping requests bounded.
const PREFETCH_RADIUS: usize = 4;

/// Details the prefetch cache holds. A few screens deep: scrolling back
/// through them stays instant, while a long session on the demo library's
/// thousands of rows cannot grow the cache without bound.
const DETAIL_CACHE_CAPACITY: usize = 96;

/// Fetched details keyed by id, first-in first-out. A hit renders the
/// panel without a fetch, which is what makes nearby steps feel instant;
/// details change only through an edit, and saving one invalidates that
/// id's entry (see [`DetailCache::invalidate`]), so entries are otherwise
/// never stale, and the oldest falls out past [`DETAIL_CACHE_CAPACITY`].
struct DetailCache {
    entries: HashMap<ResourceId, ResourceDetail>,
    /// Insertion order, oldest first; drives eviction.
    order: VecDeque<ResourceId>,
}

impl DetailCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&self, id: &ResourceId) -> Option<&ResourceDetail> {
        self.entries.get(id)
    }

    /// Store `id`'s detail. Re-storing an id keeps its original position
    /// in the eviction order, so a later duplicate cannot evict early.
    fn insert(&mut self, id: ResourceId, detail: ResourceDetail) {
        if self.entries.insert(id, detail).is_none() {
            self.order.push_back(id);
        }
        while self.order.len() > DETAIL_CACHE_CAPACITY {
            let oldest = self.order.pop_front().expect("order mirrors entries");
            self.entries.remove(&oldest);
        }
    }

    /// Drop `id`'s entry: an edit changed the stored record, so the next
    /// read must fetch again instead of hitting the pre-edit detail.
    fn invalidate(&mut self, id: &ResourceId) {
        if self.entries.remove(id).is_some() {
            self.order.retain(|entry| entry != id);
        }
    }
}

/// Row indexes whose details prefetch warms for a selection at `row_ix`:
/// the ±[`PREFETCH_RADIUS`] window, clamped to the listing's rows. Pure so
/// the clamping is unit-testable without a window.
fn prefetch_window(row_ix: usize, rows_count: usize) -> Range<usize> {
    let Some(last) = rows_count.checked_sub(1) else {
        return 0..0;
    };
    let start = row_ix.saturating_sub(PREFETCH_RADIUS).min(last);
    let end = row_ix.saturating_add(PREFETCH_RADIUS).min(last) + 1;
    start..end
}

pub struct LibraryApp {
    table: Entity<TableState<LibraryDelegate>>,
    /// App data directory holding config.toml, where settings persist.
    settings_dir: PathBuf,
    /// Year boxes bracketing the published window; either side alone
    /// bounds the listing, so both re-key on every edit.
    published_from_input: Entity<InputState>,
    published_to_input: Entity<InputState>,
    detail_open: bool,
    /// Row selection, tracked here so the panel shows one resource exactly
    /// as long as its row stays selected.
    selected: Option<ResourceId>,
    /// Detail for the selected resource, keyed by id: a late reply from a
    /// superseded selection is dropped instead of flashing into the panel.
    detail: Option<(ResourceId, DetailFetch)>,
    /// Details fetched ahead of the selection; a hit renders straight into
    /// the panel so nearby steps never flash loading.
    detail_cache: DetailCache,
    /// Ids with a detail fetch in flight, so selection and prefetch share
    /// one request per id instead of stacking duplicates.
    detail_inflight: HashSet<ResourceId>,
    /// The edit form while one is open: loading while the raw record is
    /// fetched, ready once the inputs exist. One form at a time, owned by
    /// the row it was opened on.
    edit: Option<EditFetch>,
    /// Per-id generation for detail fetches. Saving an edit bumps its id's
    /// generation; a reply from a fetch issued before the bump is dropped
    /// instead of re-caching the pre-edit detail.
    detail_gens: HashMap<ResourceId, u64>,
}

/// The edit flow's two states: fetching the raw record the form edits,
/// then holding the prefilled widgets.
enum EditFetch {
    Loading(ResourceId),
    Ready(Box<EditForm>),
}

/// The edit form's widgets plus the raw record they edit. Inputs are
/// prefilled from the record at construction; saving reads them back into
/// an [`EditDraft`] and builds the full-replace payload. The parent box
/// starts empty — its options need a listing, which is a second fetch —
/// and fills in the same flow that opens the form (see
/// [`LibraryApp::request_edit`]).
struct EditForm {
    record: Box<Resource>,
    title: Entity<InputState>,
    kind: Entity<ComboboxState<SearchableVec<KindItem>>>,
    parent: Entity<ComboboxState<SearchableVec<ParentItem>>>,
    publisher: Entity<InputState>,
    published_year: Entity<InputState>,
    accessed_year: Entity<InputState>,
    series: Entity<InputState>,
    series_number: Entity<InputState>,
    /// The form's own line of complaint: a rejected payload or a failed
    /// save, shown until the next attempt.
    error: Option<String>,
    /// While the save is in flight: the Save button busy, no double sends.
    saving: bool,
    /// The expected parent kind the box's current options target, `None`
    /// when the kind forbids parents or the options have not landed: a
    /// kind change re-keys the box only when this value moves.
    parent_options_kind: Option<ResourceKind>,
    /// Bumped on every options load; a reply carries the generation it was
    /// issued for, so a late fetch cannot clobber a newer kind's options.
    parent_gen: u64,
}

impl EditForm {
    /// Prefilled widgets for `record`: every scalar opens with its stored
    /// value, the kind box with the record's kind selected. The kind box's
    /// changes re-key the parent options through `worker`.
    fn new(
        record: Resource,
        worker: &DbWorker,
        window: &mut Window,
        cx: &mut Context<LibraryApp>,
    ) -> Self {
        let draft = prefill_draft(&record);
        let items = kind_items();
        // The box keys rows by position: the kind's row in the alphabetized
        // list is its selection. The list carries every kind, so the lookup
        // cannot miss.
        let kind_row = items
            .iter()
            .position(|item| item.value() == &record.kind)
            .expect("kind_items carries every kind");
        let kind = cx.new(|cx| {
            ComboboxState::new(
                SearchableVec::new(items),
                vec![IndexPath::new(kind_row)],
                window,
                cx,
            )
            .searchable(true)
        });
        let parent = cx.new(|cx| {
            ComboboxState::new(SearchableVec::new(Vec::new()), Vec::new(), window, cx)
                .searchable(true)
        });
        // A parent valid for the old kind is off-kind for the new one, and
        // a kind that forbids parents empties the box outright, so every
        // kind change re-keys the parent options.
        cx.subscribe_in(&kind, window, {
            let worker = worker.clone();
            move |app, _kind, event: &ComboboxEvent<SearchableVec<KindItem>>, window, cx| {
                let ComboboxEvent::Change(values) = event else {
                    return;
                };
                let Some(EditFetch::Ready(form)) = &app.edit else {
                    return;
                };
                // A cleared kind box falls back to the stored kind, the
                // same rule the draft applies on save.
                let new_kind = values.first().copied().unwrap_or(form.record.kind);
                let rule = new_kind.parent_rule();
                let expected = match rule {
                    ParentRule::Optional { expected } | ParentRule::Required { expected } => {
                        Some(expected)
                    }
                    ParentRule::Forbidden => None,
                };
                // The expected kind has not moved: a bookSection and a
                // dictionaryEntry share a book, so the options already
                // target the right kind.
                if expected == form.parent_options_kind {
                    return;
                }
                let id = form.record.id;
                // Returning to the record's own kind restores its stored
                // parent's selection; any other kind starts the box clean,
                // since a parent of the old kind is off-kind for the new.
                let stored_parent = (new_kind == form.record.kind)
                    .then_some(form.record.parent_id)
                    .flatten();
                app.load_parent_options(worker.clone(), id, rule, stored_parent, window, cx);
            }
        })
        .detach();
        let parent_options_kind = match draft.kind.parent_rule() {
            ParentRule::Optional { expected } | ParentRule::Required { expected } => {
                Some(expected)
            }
            ParentRule::Forbidden => None,
        };
        Self {
            title: prefill_input(draft.title, "Title", window, cx),
            kind,
            parent,
            publisher: prefill_input(draft.publisher, "Publisher", window, cx),
            published_year: prefill_input(draft.published_year, "Year", window, cx),
            accessed_year: prefill_input(draft.accessed_year, "Year", window, cx),
            series: prefill_input(draft.series, "Series id", window, cx),
            series_number: prefill_input(draft.series_number, "e.g. LCL 26", window, cx),
            record: Box::new(record),
            error: None,
            saving: false,
            parent_options_kind,
            parent_gen: 0,
        }
    }

    /// The inputs' current values as one plain draft. A cleared kind box
    /// falls back to the stored kind: every resource carries one, so the
    /// form never clears it. A cleared parent box is exactly that: no
    /// parent.
    fn draft(&self, cx: &App) -> EditDraft {
        let value =
            |input: &Entity<InputState>| input.read(cx).value().to_string();
        EditDraft {
            title: value(&self.title),
            kind: self
                .kind
                .read(cx)
                .selected_values()
                .first()
                .copied()
                .unwrap_or(self.record.kind),
            parent: self.parent.read(cx).selected_values().first().cloned(),
            publisher: value(&self.publisher),
            published_year: value(&self.published_year),
            accessed_year: value(&self.accessed_year),
            series: value(&self.series),
            series_number: value(&self.series_number),
        }
    }
}

/// One prefilled input: placeholder when the value is empty, the value as
/// opening text otherwise.
fn prefill_input(
    value: String,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut Context<LibraryApp>,
) -> Entity<InputState> {
    let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
    state.update(cx, |state, cx| state.set_value(value, window, cx));
    state
}

impl LibraryApp {
    /// Persist a finished column resize to config.toml. The table emits
    /// [`TableEvent::ColumnWidthsChanged`] once per completed drag or fit,
    /// so this is one small write per user action; the delegate's copy is
    /// updated too, so a later refresh reads the same widths back.
    fn save_column_widths(&mut self, widths: &[Pixels], cx: &mut Context<Self>) {
        let settings = self.table.update(cx, |table, _cx| {
            let settings = &mut table.delegate_mut().settings;
            settings.column_widths = widths_by_key(widths);
            settings.clone()
        });
        if let Err(err) = settings.save(&self.settings_dir) {
            eprintln!("warning: could not save column widths: {err:#}");
        }
    }

    pub fn new(
        table: Entity<TableState<LibraryDelegate>>,
        settings_dir: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe_in(&table, window, |this, _table, event: &TableEvent, window, cx| {
            if let TableEvent::ColumnWidthsChanged(widths) = event {
                this.save_column_widths(widths, cx);
                return;
            }
            let selected_row = match event {
                TableEvent::SelectRow(row_ix) => Some(*row_ix),
                TableEvent::ClearSelection => None,
                _ => return,
            };
            this.table.update(cx, |table, cx| {
                table.delegate_mut().selected_row = selected_row;
                cx.notify();
            });
            match selected_row {
                Some(row_ix) => this.select_row(row_ix, window, cx),
                None => this.clear_selection(cx),
            }
        })
        .detach();

        // The kind box is the header's one static dropdown: every kind is
        // known without a fetch, so it opens fully populated. Picking or
        // clearing a kind re-keys the listing through the same reload the
        // title box uses.
        let kind_box = cx.new(|cx| {
            ComboboxState::new(SearchableVec::new(kind_items()), Vec::new(), window, cx)
                .searchable(true)
        });
        cx.subscribe_in(
            &kind_box,
            window,
            |this, _box, event: &ComboboxEvent<SearchableVec<KindItem>>, window, cx| {
                // The box is single-select: one kind narrows, none clears.
                let ComboboxEvent::Change(values) = event else {
                    return;
                };
                let kind = values.first().copied();
                let selected_id = this
                    .table
                    .read(cx)
                    .selected_row()
                    .and_then(|ix| this.table.read(cx).delegate().row_id(ix));
                this.table.update(cx, |table, cx| {
                    table
                        .delegate_mut()
                        .request_kind_filter(kind, selected_id, window, cx);
                });
                cx.notify();
            },
        )
        .detach();

        // The box re-keys the listing on every change, clear button
        // included; the listing generation keeps a late reply from
        // clobbering the rows a newer keystroke asked for.
        let title_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter by title"));
        cx.subscribe_in(
            &title_input,
            window,
            |this, input, event: &InputEvent, window, cx| {
                // Focus and blur leave the listing alone; only edits count.
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let needle = title_predicate(&input.read(cx).value());
                let selected_id = this
                    .table
                    .read(cx)
                    .selected_row()
                    .and_then(|ix| this.table.read(cx).delegate().row_id(ix));
                this.table.update(cx, |table, cx| {
                    table
                        .delegate_mut()
                        .request_title_filter(needle, selected_id, window, cx);
                });
                cx.notify();
            },
        )
        .detach();

        // The creator box starts empty and fills when the person list
        // lands from the worker; picking or clearing a person re-keys the
        // listing through the same reload the title box uses.
        let creator_box = cx.new(|cx| {
            ComboboxState::new(SearchableVec::new(Vec::new()), Vec::new(), window, cx)
                .searchable(true)
        });
        cx.subscribe_in(
            &creator_box,
            window,
            |this, _box, event: &ComboboxEvent<SearchableVec<CreatorItem>>, window, cx| {
                // The box is single-select: one id narrows, none clears.
                let ComboboxEvent::Change(values) = event else {
                    return;
                };
                let creator = values.first().copied();
                let selected_id = this
                    .table
                    .read(cx)
                    .selected_row()
                    .and_then(|ix| this.table.read(cx).delegate().row_id(ix));
                this.table.update(cx, |table, cx| {
                    table
                        .delegate_mut()
                        .request_creator_filter(creator, selected_id, window, cx);
                });
                cx.notify();
            },
        )
        .detach();

        let worker = table.read(cx).delegate().worker().clone();
        let creator_options_box = creator_box.clone();
        cx.spawn_in(window, async move |app, window| {
            let reply = worker.list_persons().await;
            _ = app.update_in(window, |app, window, cx| match reply {
                Ok(people) => {
                    let items: Vec<_> = creator_options(&people)
                        .into_iter()
                        .map(|(label, id)| CreatorItem { label, id })
                        .collect();
                    creator_options_box.update(cx, |state, cx| {
                        state.set_items(SearchableVec::new(items), window, cx);
                    });
                }
                Err(err) => app.table.update(cx, |table, _cx| {
                    // The dropdown stays empty; the listing's error line
                    // explains why.
                    table.delegate_mut().error = Some(err);
                }),
            });
        })
        .detach();

        // Year boxes bracket whole years: the from year's January 1 is
        // inclusive, the to year's next January 1 exclusive, so either
        // side alone still bounds the listing.
        let published_from_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("From"));
        let published_to_input = cx.new(|cx| InputState::new(window, cx).placeholder("To"));
        for input in [&published_from_input, &published_to_input] {
            cx.subscribe_in(input, window, |this, _input, event: &InputEvent, window, cx| {
                // Only edits re-key the listing; focus and blur leave it be.
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                this.apply_year_range(window, cx);
            })
            .detach();
        }

        // The header filter row renders the same entities this app subscribes
        // to, so filtering works from the column headers with the wiring
        // above unchanged.
        table.update(cx, |table, _| {
            table.delegate_mut().filter_inputs = Some(FilterInputs {
                kind: kind_box,
                title: title_input,
                creator: creator_box,
                published_from: published_from_input.clone(),
                published_to: published_to_input.clone(),
            });
        });

        Self {
            table,
            settings_dir,
            published_from_input,
            published_to_input,
            detail_open: false,
            selected: None,
            detail: None,
            detail_cache: DetailCache::new(),
            detail_inflight: HashSet::new(),
            edit: None,
            detail_gens: HashMap::new(),
        }
    }

    /// Row selection: the panel follows the id immediately, then prefetch
    /// warms the rows around it so the next step up or down is instant.
    fn select_row(&mut self, row_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.table.read(cx).delegate().row_id(row_ix) else {
            return;
        };
        self.selected = Some(id);
        // The form edits the record the panel was showing; a different row
        // selected means that record is no longer in view, so the form
        // closes instead of editing out of sight.
        let editing_this = match &self.edit {
            Some(EditFetch::Loading(loading)) => *loading == id,
            Some(EditFetch::Ready(form)) => form.record.id == id,
            None => true,
        };
        if !editing_this {
            self.edit = None;
        }
        self.show_detail(id, window, cx);
        self.prefetch_around(row_ix, window, cx);
        cx.notify();
    }

    /// Point the panel at `id`'s detail: a prefetch hit renders the fields
    /// immediately; a miss shows loading while the one fetch for this id
    /// fills it. A slot already serving this id is left alone, so a
    /// reselect never restarts a finished or in-flight fetch.
    fn show_detail(&mut self, id: ResourceId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(detail) = self.detail_cache.get(&id) {
            self.detail = Some((id, DetailFetch::Loaded(Box::new(detail.clone()))));
            return;
        }
        if self.detail.as_ref().is_some_and(|(loaded, _)| *loaded == id) {
            return;
        }
        self.detail = Some((id, DetailFetch::Loading));
        self.fetch_detail(id, window, cx);
    }

    /// Warm the cache around the selection. Cached and in-flight rows are
    /// skipped, so stepping back and forth near one spot issues no new
    /// requests.
    fn prefetch_around(&mut self, row_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<Option<ResourceId>> = {
            let delegate = self.table.read(cx).delegate();
            prefetch_window(row_ix, delegate.rows_count(cx))
                .map(|ix| delegate.row_id(ix))
                .collect()
        };
        for id in ids.into_iter().flatten() {
            if self.detail_cache.get(&id).is_none() {
                self.fetch_detail(id, window, cx);
            }
        }
    }

    /// Fetch `id`'s detail off the UI thread, unless one is already in
    /// flight. The reply lands in the cache whatever happened since, and
    /// the panel only changes when this id is still the selection, so a
    /// stale reply never flashes in — except after an edit, which bumps the
    /// id's generation so the pre-edit reply is discarded outright.
    fn fetch_detail(&mut self, id: ResourceId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.detail_inflight.insert(id) {
            return;
        }
        let fetch_epoch = *self.detail_gens.get(&id).unwrap_or(&0);
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let reply = worker.resource_detail(id).await;
            _ = app.update_in(window, |app, _window, cx| {
                app.detail_inflight.remove(&id);
                // The record this reply describes was edited after the
                // fetch was issued; it must not enter the cache or the
                // panel. The save's own refetch (issued at the new
                // epoch) fills both.
                if *app.detail_gens.get(&id).unwrap_or(&0) != fetch_epoch {
                    return;
                }
                match reply {
                    Ok(detail) => {
                        // Failures stay out of the cache: reselecting a
                        // failed row gets a fresh retry, not a stored error.
                        app.detail_cache.insert(id, detail);
                        if app.selected == Some(id) {
                            app.detail = Some((
                                id,
                                DetailFetch::Loaded(Box::new(
                                    app.detail_cache.get(&id).expect("just inserted").clone(),
                                )),
                            ));
                            cx.notify();
                        }
                    }
                    Err(error) => {
                        if app.selected == Some(id) {
                            app.detail = Some((id, DetailFetch::Failed(error)));
                            cx.notify();
                        }
                    }
                }
            });
        })
        .detach();
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        cx.notify();
    }

    /// Recompute the published window from both year boxes and hand it
    /// to the listing; one change on either side re-keys the whole window.
    fn apply_year_range(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let from = self.published_from_input.read(cx).value();
        let to = self.published_to_input.read(cx).value();
        let range = year_range(&from, &to);
        let selected_id = self
            .table
            .read(cx)
            .selected_row()
            .and_then(|ix| self.table.read(cx).delegate().row_id(ix));
        self.table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .request_published_filter(range, selected_id, window, cx);
        });
        cx.notify();
    }

    /// Show or hide the whole filter row; the filters themselves stay set.
    fn toggle_filters(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            let delegate = table.delegate_mut();
            delegate.filters_visible = !delegate.filters_visible;
            cx.notify();
        });
        cx.notify();
    }

    fn toggle_detail_panel(&mut self, cx: &mut Context<Self>) {        // The detail is fetched on selection whether or not the panel shows,
        // so opening it never waits on a fetch.
        self.detail_open = !self.detail_open;
        cx.notify();
    }

    /// Open the edit form for the selected resource: fetch the raw record
    /// (the panel's projection drops the ids the form needs), then open
    /// the prefilled widgets — the parent box empty until its options land,
    /// a second fetch in the same flow. One form at a time; the sidebar
    /// swaps to it while it exists.
    fn request_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            return;
        };
        if self.edit.is_some() {
            return;
        }
        self.edit = Some(EditFetch::Loading(id));
        self.detail_open = true;
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let reply = worker.resource(id).await;
            _ = app.update_in(window, |app, window, cx| {
                match reply {
                    Ok(record) => {
                        // The selection moved or re-clicked Edit since the
                        // request: the reply belongs to a form nobody is
                        // waiting for, so it is dropped.
                        if app.selected == Some(id)
                            && matches!(app.edit, Some(EditFetch::Loading(_)))
                        {
                            let parent_id = record.parent_id;
                            let rule = record.kind.parent_rule();
                            let form = EditForm::new(record, &worker, window, cx);
                            app.edit = Some(EditFetch::Ready(Box::new(form)));
                            // The parent box opened empty: its options come
                            // from a listing narrowed to the kinds this
                            // record's rule allows, fetched next.
                            app.load_parent_options(
                                worker.clone(),
                                id,
                                rule,
                                parent_id,
                                window,
                                cx,
                            );
                        }
                    }
                    Err(err) => {
                        app.edit = None;
                        if app.selected == Some(id) {
                            app.detail = Some((id, DetailFetch::Failed(err)));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Fill the parent box with options for `id`'s current kind rule: the
    /// listing narrows to the rule's expected kind at the store, so a
    /// sparse kind still fills the dropdown instead of losing a race with
    /// the library's newest rows. `stored_parent` names a parent beyond the
    /// fetched page so it stays selectable and a save never silently drops
    /// the link; a rule that forbids parents empties the box instead of
    /// fetching. The form's generation guard drops a late reply once a
    /// newer kind change has re-keyed the options.
    fn load_parent_options(
        &mut self,
        worker: DbWorker,
        id: ResourceId,
        rule: ParentRule,
        stored_parent: Option<ResourceId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(EditFetch::Ready(form)) = &mut self.edit else {
            return;
        };
        if form.record.id != id {
            return;
        }
        let expected = match rule {
            ParentRule::Optional { expected } | ParentRule::Required { expected } => {
                Some(expected)
            }
            ParentRule::Forbidden => None,
        };
        form.parent_options_kind = expected;
        form.parent_gen += 1;
        let options_gen = form.parent_gen;
        let (ParentRule::Optional { expected } | ParentRule::Required { expected }) = rule else {
            // Nothing can parent this kind, and a selection left over
            // from the previous kind would ride into a doomed save: both
            // go now rather than after the next fetch.
            form.parent.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(Vec::new()), window, cx);
                state.set_selected_indices(Vec::new(), window, cx);
            });
            cx.notify();
            return;
        };
        let query = ResourceQuery {
            sort: None,
            filter: ResourceFilter::by_kind(expected),
        };
        cx.spawn_in(window, async move |app, window| {
            let mut items: Vec<ParentItem> = Vec::new();
            let mut missing_parent = stored_parent;
            if let Ok(page) = worker.list_query_page(query, 0, PARENT_OPTIONS_LIMIT).await {
                items = parent_options(&page.rows, id, rule);
                missing_parent = stored_parent
                    .filter(|parent| !items.iter().any(|item| item.id == *parent));
            }
            if let Some(parent) = missing_parent {
                // The stored parent did not make the page. Its own record
                // still names its title; if even that read fails, the raw
                // id is the honest label — better than silently dropping
                // the stored link on save.
                let label = match worker.resource(parent).await {
                    Ok(record) => SharedString::from(record.title.as_str()),
                    Err(_) => SharedString::from(parent.to_string()),
                };
                items.insert(0, ParentItem { label, id: parent });
            }
            // A stored parent opens selected; any other caller changed the
            // kind, and the stale selection clears with the options.
            let selection = stored_parent.and_then(|parent| {
                items
                    .iter()
                    .position(|item| item.id == parent)
                    .map(IndexPath::new)
            });
            _ = app.update_in(window, |app, window, cx| {
                // The form closed, a different record's form opened, or a
                // newer kind change re-keyed the options since the fetch:
                // this reply belongs to nobody.
                let Some(EditFetch::Ready(form)) = &app.edit else {
                    return;
                };
                if form.record.id != id || form.parent_gen != options_gen {
                    return;
                }
                form.parent.update(cx, |state, cx| {
                    state.set_items(SearchableVec::new(items), window, cx);
                    match selection {
                        Some(row) => state.set_selected_indices(vec![row], window, cx),
                        None => state.set_selected_indices(Vec::new(), window, cx),
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Close the form without writing; the panel falls back to the detail.
    fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        self.edit = None;
        cx.notify();
    }

    /// Save the form: read the inputs into a draft, build the full-replace
    /// payload, send it through the worker. A rejected payload or a failed
    /// save shows in the form; success closes it and refreshes the panel
    /// and the listing row.
    fn save_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(EditFetch::Ready(form)) = &self.edit else {
            return;
        };
        if form.saving {
            return;
        }
        let record = form.record.clone();
        let draft = form.draft(cx);
        let new = match build_update(&record, &draft) {
            Ok(new) => new,
            Err(message) => {
                if let Some(EditFetch::Ready(form)) = &mut self.edit {
                    form.error = Some(message);
                }
                cx.notify();
                return;
            }
        };
        let id = record.id;
        if let Some(EditFetch::Ready(form)) = &mut self.edit {
            form.saving = true;
            form.error = None;
        }
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let reply = worker.update_resource(id, new).await;
            _ = app.update_in(window, |app, window, cx| {
                match reply {
                    Ok(_) => {
                        app.edit = None;
                        // The stored record changed under every detail
                        // fetched for it: bump the id's generation so an
                        // in-flight fetch's reply is dropped, forget the
                        // cached detail, and refetch for the panel.
                        *app.detail_gens.entry(id).or_default() += 1;
                        app.detail_inflight.remove(&id);
                        app.detail_cache.invalidate(&id);
                        app.detail = Some((id, DetailFetch::Loading));
                        app.fetch_detail(id, window, cx);
                        // The row may have moved under the current sort;
                        // the reload re-walks the listing and keeps the
                        // highlight on the edited id.
                        app.table.update(cx, |table, cx| {
                            table
                                .delegate_mut()
                                .reload_from_top(Some(id), window, cx);
                        });
                    }
                    Err(err) => match &mut app.edit {
                        Some(EditFetch::Ready(form)) => {
                            form.error = Some(err);
                            form.saving = false;
                        }
                        _ => app.edit = None,
                    },
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn detail_view(&self) -> DetailView<'_> {
        match (self.selected, self.detail.as_ref()) {
            (Some(id), Some((loaded, fetch))) if *loaded == id => match fetch {
                DetailFetch::Loading => DetailView::Loading,
                DetailFetch::Loaded(detail) => DetailView::Fields(detail),
                DetailFetch::Failed(error) => DetailView::Failed(error),
            },
            // select_row fills the slot in the same turn — from the cache
            // or with a started fetch — so this arm only bridges the gap
            // between its statements; the panel just shows loading.
            (Some(_), _) => DetailView::Loading,
            (None, _) => DetailView::Empty,
        }
    }

    /// The right-side sidebar: the edit form while one is open, the detail
    /// fields otherwise. The form keeps the sidebar's anchor, width, and
    /// surface but drops the kind tint, so its inputs sit on a plain
    /// background. The detail carries the edit pencil in its top-right
    /// corner; no row selected means nothing to edit, and an open form
    /// means the sidebar already is the editor.
    fn render_side_panel(&self, cx: &mut Context<Self>) -> Div {
        match &self.edit {
            Some(EditFetch::Ready(form)) => self.render_edit_panel(form, cx),
            Some(EditFetch::Loading(_)) => {
                render_panel(DetailView::Loading, None, cx)
            }
            None => {
                let edit_button = self.selected.is_some().then(|| {
                    Button::new("edit-resource")
                        .ghost()
                        .small()
                        .icon(symbol("pencil", AssetIconName::PenLine))
                        .tooltip("Edit resource")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.request_edit(window, cx)
                        }))
                        .into_any_element()
                });
                render_panel(self.detail_view(), edit_button, cx)
            }
        }
    }

    /// The form: Save/Cancel above, one labelled row per scalar, the error
    /// line under the buttons. Scrollable like the detail sidebar, so a
    /// short window still reaches every row. Parent is a searchable
    /// dropdown of library titles, not an id to type: users pick the
    /// containing work by name.
    fn render_edit_panel(&self, form: &EditForm, cx: &mut Context<Self>) -> Div {
        let saving = form.saving;
        let error = form.error.clone();
        let danger = cx.theme().danger;

        v_flex()
            .h_full()
            .flex_shrink_0()
            .w(PANEL_WIDTH)
            .bg(cx.theme().background)
            .border_l_1()
            .border_color(cx.theme().border)
            .text_size(FIELD_TEXT_SIZE)
            .child(
                v_flex()
                    .id("edit-panel")
                    .size_full()
                    .p_3()
                    .gap_2()
                    .overflow_y_scroll()
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("cancel-edit")
                                    .ghost()
                                    .small()
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.cancel_edit(cx)
                                    })),
                            )
                            .child(
                                Button::new("save-edit")
                                    .primary()
                                    .small()
                                    .label("Save")
                                    .loading(saving)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_edit(window, cx)
                                    })),
                            ),
                    )
                    .when_some(error, |el, error| {
                        el.child(div().text_xs().text_color(danger).child(error))
                    })
                    .child(edit_field("Title", Input::new(&form.title).small().w_full()))
                    .child(edit_field(
                        "Kind",
                        Combobox::new(&form.kind)
                            .small()
                            .placeholder("Kind")
                            .search_placeholder("Search kinds")
                            .w_full(),
                    ))
                    .child(edit_field(
                        "Parent",
                        Combobox::new(&form.parent)
                            .small()
                            .placeholder("Parent")
                            .search_placeholder("Search resources")
                            .cleanable(true)
                            .w_full(),
                    ))
                    .child(edit_field(
                        "Publisher",
                        Input::new(&form.publisher).small().w_full(),
                    ))
                    .child(edit_field(
                        "Published",
                        Input::new(&form.published_year).small().w_full(),
                    ))
                    .child(edit_field(
                        "Accessed",
                        Input::new(&form.accessed_year).small().w_full(),
                    ))
                    .child(edit_field("Series", Input::new(&form.series).small().w_full()))
                    .child(edit_field(
                        "Series no.",
                        Input::new(&form.series_number).small().w_full(),
                    )),
            )
    }
}

/// One form row: the panel's label column left, the control filling the
/// remaining width.
fn edit_field(label: &'static str, control: impl IntoElement) -> Div {
    h_flex()
        .items_center()
        .gap_2()
        .child(div().w(FIELD_LABEL_WIDTH).flex_shrink_0().child(label))
        .child(control)
}

impl Render for LibraryApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let delegate = self.table.read(cx).delegate();
        let error = delegate.error().map(str::to_owned);
        let filters_visible = delegate.filters_visible;
        let filter_active = delegate.filter.is_active();
        let danger = cx.theme().danger;

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            // `]` toggles the detail panel; the context must cover the
            // focused table, which renders below this root.
            .key_context("LibraryApp")
            .on_action(cx.listener(|this, _: &ToggleDetailPanel, _window, cx| {
                this.toggle_detail_panel(cx);
            }))
            .child(
                // The toolbar doubles as the window's title bar: the traffic
                // lights sit in the bar's built-in 80 px left padding, its
                // empty stretches drag the window, and it draws its own
                // bottom hairline.
                TitleBar::new()
                    .pr_2()
                    .child(
                        Button::new("toggle-filters")
                            .ghost()
                            .small()
                            .icon(symbol("line.3.horizontal.decrease", IconName::Settings2))
                            .label("Filters")
                            .selected(filters_visible)
                            .when(filter_active, |button| button.text_color(cx.theme().primary))
                            .on_click(cx.listener(|this, _, _window, cx| this.toggle_filters(cx))),
                    )
                    .child(
                        Button::new("toggle-theme")
                            .ghost()
                            .small()
                            .icon(if cx.theme().is_dark() {
                                symbol("sun.max", IconName::Sun)
                            } else {
                                symbol("moon", IconName::Moon)
                            })
                            .label(if cx.theme().is_dark() { "Light" } else { "Dark" })
                            .tooltip(if cx.theme().is_dark() {
                                "Switch to light theme"
                            } else {
                                "Switch to dark theme"
                            })
                            .on_click(cx.listener(|_, _, _window, cx| {
                                let mode = if cx.theme().is_dark() {
                                    ThemeMode::Light
                                } else {
                                    ThemeMode::Dark
                                };
                                crate::theme::apply(mode, cx);
                            })),
                    ),
            )
            .child(
                // The grid and the sidebar share one row: the grid gives
                // way by width when the window cannot hold both at full
                // size. The sidebar never overlays the grid, so its empty
                // stretches cannot swallow clicks aimed at (or through to)
                // rows.
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        // `h_flex` centers its children on the cross axis
                        // instead of stretching them, so the wrapper must
                        // claim the row's height itself: without `h_full`
                        // it falls back to content height, every
                        // percentage height below it (the table, its
                        // scroll body, the rows list) collapses, and the
                        // header-only strip floats mid-window with no
                        // rows and no empty state.
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(
                                DataTable::new(&self.table)
                                    .with_size(Size::XSmall)
                                    .bordered(false)
                                    .stripe(true)
                                    // The table holds focus from the first frame (so
                                    // the detail-panel key works), which makes the
                                    // kit's keyboard focus ring draw on every arrow
                                    // key. The table fills this container edge to
                                    // edge, so only the ring's top edge is inside
                                    // the window, and it reads as a stray bar under
                                    // the toolbar. The row highlight already shows
                                    // where focus is; the ring adds nothing.
                                    .focus_ring(false)
                                    .when_some(header_row_height(filters_visible), |table, height| {
                                        table.header_row_height(height)
                                    }),
                            ),
                    )
                    // The sidebar is the edit form while one is open, the
                    // detail fields otherwise; hidden entirely while the
                    // panel is toggled off.
                    .when(self.detail_open, |el| {
                        el.child(self.render_side_panel(cx))
                    }),
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

#[cfg(test)]
mod column_width_tests {
    // No `use super::*`: the glob would re-import gpui's `test` proc
    // macro (via the module's `gpui_kit::*`), hijacking `#[test]`.
    use super::{
        COL_CREATORS, COL_PUBLISHED, COL_TITLE, COLUMN_KEYS, COLUMNS, DEFAULT_TITLE_WIDTH,
        stored_width, widths_by_key,
    };
    use gpui_kit::px;
    use std::collections::BTreeMap;

    #[test]
    fn widths_by_key_keys_each_column_by_its_config_key() {
        let map = widths_by_key(&[px(28.), px(310.), px(260.), px(140.)]);
        assert_eq!(map.len(), COLUMNS);
        assert_eq!(map[COLUMN_KEYS[COL_TITLE]], 310.);
        assert_eq!(map[COLUMN_KEYS[COL_PUBLISHED]], 140.);
        assert!(COLUMN_KEYS.iter().all(|key| map.contains_key(*key)));
    }

    #[test]
    fn stored_width_prefers_saved_widths_over_defaults() {
        let saved = BTreeMap::from([("title".to_owned(), 480.)]);
        assert_eq!(
            stored_width(&saved, COL_TITLE, DEFAULT_TITLE_WIDTH),
            px(480.)
        );
        assert_eq!(stored_width(&BTreeMap::new(), COL_CREATORS, 260.), px(260.));
        // The title's built-in width is half its former 620 px.
        assert_eq!(DEFAULT_TITLE_WIDTH, 310.);
    }
}

#[cfg(test)]
mod sort_cycle_tests {
    // No `use super::*`: the glob would re-import gpui's `test` proc
    // macro (via the module's `gpui_kit::*`), hijacking `#[test]`.
    use super::{
        cell_text_colors, creator_options, filter_field_for_col, header_row_height, next_sort,
        sort_field_for_col, title_predicate, year_instant, year_range, FilterField, COL_CREATORS, COL_KIND,
        COL_PUBLISHED, COL_TITLE,
    };
    use gpui_kit::component::ThemeColor;
    use gpui_kit::{px, rgb, white};
    use oeuvre_core::{Name, Person, PersonId, ResourceSort, SortDirection, UtcMillis};

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

    #[test]
    fn title_predicate_trims_and_clears_on_empty() {
        assert_eq!(title_predicate(""), None);
        assert_eq!(title_predicate("   "), None);
        let needle = title_predicate("  MOD ").expect("non-empty needle");
        assert_eq!(needle.as_str(), "MOD");
    }

    #[test]
    fn year_instant_parses_and_rejects() {
        assert_eq!(
            year_instant("  1962 "),
            Some(UtcMillis::jan_1_utc(1962).expect("civil year"))
        );
        assert_eq!(year_instant(""), None);
        assert_eq!(year_instant("19x"), None);
    }

    #[test]
    fn year_range_brackets_whole_years() {
        let both = year_range("1962", "1963").expect("both years parse");
        assert_eq!(
            both.from,
            Some(UtcMillis::jan_1_utc(1962).expect("civil year"))
        );
        // The to-year's next January 1 is exclusive, so 1963 lands whole.
        assert_eq!(
            both.to,
            Some(UtcMillis::jan_1_utc(1964).expect("civil year"))
        );

        let from_only = year_range("1984", "").expect("from year parses");
        assert_eq!(
            from_only.from,
            Some(UtcMillis::jan_1_utc(1984).expect("civil year"))
        );
        assert_eq!(from_only.to, None);

        let to_only = year_range("", "1984").expect("to year parses");
        assert_eq!(to_only.from, None);
        assert_eq!(
            to_only.to,
            Some(UtcMillis::jan_1_utc(1985).expect("civil year"))
        );

        assert_eq!(year_range("", "not a year"), None);
        assert_eq!(year_range("", ""), None);
    }

    #[test]
    fn creator_options_sort_by_display_name() {
        let hopper = Person::try_new(
            PersonId::generate(),
            Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap(),
            vec![],
        )
        .unwrap();
        let lovelace = Person::try_new(
            PersonId::generate(),
            Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
            vec![],
        )
        .unwrap();
        // The store's listing order must not leak into the dropdown.
        let options = creator_options(&[hopper, lovelace]);
        let labels: Vec<&str> = options.iter().map(|(label, _)| label.as_ref()).collect();
        assert_eq!(labels, ["Ada Lovelace", "Grace Hopper"]);
        assert!(options.iter().all(|(_, id)| id.as_ulid().timestamp_ms() > 0));
    }

    #[test]
    fn header_columns_map_to_filter_fields() {
        assert_eq!(filter_field_for_col(COL_KIND), Some(FilterField::Kind));
        assert_eq!(
            filter_field_for_col(COL_TITLE),
            Some(FilterField::Title)
        );
        assert_eq!(
            filter_field_for_col(COL_CREATORS),
            Some(FilterField::Creators)
        );
        assert_eq!(
            filter_field_for_col(COL_PUBLISHED),
            Some(FilterField::Published)
        );
    }

    #[test]
    fn selected_row_text_turns_white_on_the_accent_fill() {
        let colors = ThemeColor {
            foreground: rgb(0x111111).into(),
            muted_foreground: rgb(0x777777).into(),
            primary_foreground: white(),
            ..ThemeColor::default()
        };
        assert_eq!(
            cell_text_colors(false, &colors),
            (colors.foreground, colors.muted_foreground)
        );
        let (primary, secondary) = cell_text_colors(true, &colors);
        assert_eq!(primary, white());
        // Secondary columns stay white, only dimmed, so they still read
        // on the blue fill.
        assert_eq!((secondary.h, secondary.s, secondary.l), (0., 0., 1.));
        assert!(secondary.a < 1. && secondary.a >= 0.75);
    }

    #[test]
    fn header_grows_a_second_row_only_while_filters_show() {
        // Hidden filters keep the kit's single-row header.
        assert_eq!(header_row_height(false), None);
        // Shown filters stack one control row under the names: two
        // 26px extra-small rows.
        assert_eq!(header_row_height(true), Some(px(52.)));
    }
}

#[cfg(test)]
mod detail_prefetch_tests {
    // No `use super::*`: the glob would re-import gpui's `test` proc
    // macro (via the module's `gpui_kit::*`), hijacking `#[test]`.
    use super::{DETAIL_CACHE_CAPACITY, DetailCache, PREFETCH_RADIUS, prefetch_window};
    use oeuvre_core::{NonEmptyStr, ResourceDetail, ResourceId, ResourceKind, UtcMillis};

    /// A detail whose title is the only varying field, enough to prove
    /// which entry a cache read returns.
    fn detail(id: ResourceId, title: &str) -> ResourceDetail {
        ResourceDetail {
            id,
            kind: ResourceKind::Book,
            title: NonEmptyStr::try_new(title).unwrap(),
            creators: Vec::new(),
            publisher: None,
            published: None,
            accessed: None,
            container_title: None,
            series_title: None,
            series_number: None,
            date_added: UtcMillis::from_unix_ms(0),
            date_edited: UtcMillis::from_unix_ms(0),
        }
    }

    #[test]
    fn cache_round_trips_by_id() {
        let a = ResourceId::generate();
        let b = ResourceId::generate();
        assert_ne!(a, b);
        let mut cache = DetailCache::new();
        cache.insert(a, detail(a, "Citation Networks 000"));
        cache.insert(b, detail(b, "Provenance Studies 001"));
        assert_eq!(cache.get(&a).unwrap().title.as_str(), "Citation Networks 000");
        assert_eq!(cache.get(&b).unwrap().title.as_str(), "Provenance Studies 001");
        assert!(cache.get(&ResourceId::generate()).is_none());
    }

    #[test]
    fn cache_evicts_the_oldest_detail_past_capacity() {
        let mut ids = Vec::new();
        let mut cache = DetailCache::new();
        // Capacity plus one insert: the first id is the one that falls out.
        for _ in 0..=DETAIL_CACHE_CAPACITY {
            let id = ResourceId::generate();
            cache.insert(id, detail(id, "row"));
            ids.push(id);
        }
        assert!(cache.get(&ids[0]).is_none());
        assert!(cache.get(&ids[1]).is_some());
        assert!(cache.get(&ids[DETAIL_CACHE_CAPACITY]).is_some());
    }

    #[test]
    fn reinserting_an_id_keeps_its_original_eviction_position() {
        let a = ResourceId::generate();
        let mut cache = DetailCache::new();
        // A repeated insert updates the detail but must not add a second
        // place in the eviction order.
        cache.insert(a, detail(a, "first"));
        cache.insert(a, detail(a, "second"));
        assert_eq!(cache.get(&a).unwrap().title.as_str(), "second");

        // A plus capacity minus one others fill the cache exactly, so a
        // duplicated order entry would already have evicted something.
        let mut others = Vec::new();
        for _ in 0..DETAIL_CACHE_CAPACITY - 1 {
            let id = ResourceId::generate();
            cache.insert(id, detail(id, "other"));
            others.push(id);
        }
        assert!(cache.get(&a).is_some());
        assert!(others.iter().all(|id| cache.get(id).is_some()));

        // One more insert evicts a, the oldest, and nothing else.
        let newest = ResourceId::generate();
        cache.insert(newest, detail(newest, "newest"));
        assert!(cache.get(&a).is_none());
        assert!(others.iter().all(|id| cache.get(id).is_some()));
    }

    #[test]
    fn prefetch_window_centers_on_the_selection() {
        // Radius 4 around row 50 of 100: rows 46 through 54.
        assert_eq!(prefetch_window(50, 100), 46..55);
    }

    #[test]
    fn prefetch_window_clamps_at_both_ends() {
        // The first and last rows keep full windows on their inside side.
        assert_eq!(prefetch_window(0, 100), 0..PREFETCH_RADIUS + 1);
        assert_eq!(prefetch_window(99, 100), 95..100);
        // A one-row listing prefetches exactly that row.
        assert_eq!(prefetch_window(0, 1), 0..1);
    }

    #[test]
    fn prefetch_window_is_empty_without_rows() {
        assert_eq!(prefetch_window(0, 0), 0..0);
    }
}

#[cfg(test)]
mod parent_options_tests {
    // No `use super::*`: the glob would re-import gpui's `test` proc
    // macro (via the module's `gpui_kit::*`), hijacking `#[test]`.
    use super::{ParentItem, SearchableListItem, parent_options};
    use oeuvre_core::{NonEmptyStr, ResourceId, ResourceKind, ResourceSummary};

    fn summary(title: &str) -> ResourceSummary {
        ResourceSummary {
            id: ResourceId::generate(),
            kind: ResourceKind::Book,
            title: NonEmptyStr::try_new(title).expect("title non-empty"),
            creators_label: String::new(),
            published_label: None,
            publisher: None,
            container_title: None,
        }
    }

    #[test]
    fn parent_options_sort_by_title_and_skip_the_record_itself() {
        let zzz = summary("Zzz Weekly");
        let aaa = summary("Aaa Book");
        let mut edited = summary("The edited record");
        edited.id = ResourceId::generate();
        let edited_id = edited.id;

        let options = parent_options(
            &[zzz, edited, aaa],
            edited_id,
            ResourceKind::BookSection.parent_rule(),
        );
        let labels: Vec<String> = options.iter().map(ParentItem::title).map(String::from).collect();
        // Alphabetical whatever order the store lists in, and a resource
        // cannot become its own parent, so the edited record never
        // appears among its own options.
        assert_eq!(labels, ["Aaa Book", "Zzz Weekly"]);
        assert!(options.iter().all(|item| *item.value() != edited_id));
    }

    #[test]
    fn parent_options_keep_only_the_expected_kind() {
        let mut journal = summary("Zzz Journal");
        journal.kind = ResourceKind::Journal;
        let mut article = summary("An article is not a parent");
        article.kind = ResourceKind::JournalArticle;
        let edited = summary("The edited record");
        let edited_id = edited.id;

        // A journal article's parent is a journal: articles and anything
        // else never appear among the options, whatever page served them.
        let options = parent_options(
            &[journal, article, edited],
            edited_id,
            ResourceKind::JournalArticle.parent_rule(),
        );
        let labels: Vec<String> = options.iter().map(ParentItem::title).map(String::from).collect();
        assert_eq!(labels, ["Zzz Journal"]);
    }
}
