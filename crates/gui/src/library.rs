//! Library grid: resource summary rows, one page at a time.

use std::{
    any::Any,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    ops::Range,
    path::PathBuf,
    time::{Duration, Instant},
};

use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::{Transition as MotionTransition, transition};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::combobox::{Combobox, ComboboxEvent, ComboboxState};
use gpui_kit::component::input::{Enter, Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::resizable::resize_handle;
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::table::{Column, DataTable, TableDelegate, TableEvent, TableState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, FocusTrapElement as _, FocusableExt as _, Icon, IconName, IndexPath,
    Selectable as _, Sizable as _, Size, ThemeColor, ThemeMode, TitleBar, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{Focusable as _, *};

use oeuvre_core::{
    Collection, CollectionId, CollectionTreeNode, NonEmptyStr, ParentRule, Person, PersonId,
    PublishedRange, Resource, ResourceCounts, ResourceDetail, ResourceFilter, ResourceId,
    ResourceKind, ResourcePage, ResourceQuery, ResourceSort, ResourceSummary, SortDirection, Tag,
    TagId, UtcMillis, format_recorded_date,
};

use crate::collections::{CollectionChoice, CollectionDelegate, flatten_collections};
use crate::db::DbWorker;
use crate::detail::{
    DetailFetch, DetailView, FIELD_LABEL_WIDTH, FIELD_TEXT_SIZE, PANEL_WIDTH, render_panel,
};
use crate::edit::{
    EditDraft, KindItem, build_create, build_update, creators_from_person_ids, kind_items,
    prefill_draft,
};
use crate::icons::symbol;
use crate::inline_edit::{
    InlineField, InlineRowEdit, InlineSession, PersonChoice, PersonDelegate,
    creators_badge_trigger, person_items, person_name_from_query, selected_person_ids,
};
use crate::kind::{kind_color, kind_label, kind_symbol};
use crate::settings::Settings;
use crate::tags::{TagChoice, TagDelegate};

/// Rows fetched per scroll trigger. Small enough to feel incremental, large
/// enough that the demo library spans several pages.
pub const PAGE_SIZE: u32 = 50;

/// Visible sidebar rows from a collection forest, respecting expand state.
/// The `Option<CollectionId>` is the tree parent that placed this row (`None` for roots).
fn visible_collection_rows(
    forest: &[CollectionTreeNode],
    expanded: &HashSet<CollectionId>,
) -> Vec<(Collection, usize, bool, Option<CollectionId>)> {
    let mut out = Vec::new();
    fn walk(
        nodes: &[CollectionTreeNode],
        expanded: &HashSet<CollectionId>,
        depth: usize,
        parent: Option<CollectionId>,
        out: &mut Vec<(Collection, usize, bool, Option<CollectionId>)>,
    ) {
        for node in nodes {
            let has_children = !node.children.is_empty();
            out.push((node.collection.clone(), depth, has_children, parent));
            if has_children && expanded.contains(&node.collection.id) {
                walk(
                    &node.children,
                    expanded,
                    depth + 1,
                    Some(node.collection.id),
                    out,
                );
            }
        }
    }
    walk(forest, expanded, 0, None, &mut out);
    out
}

fn expand_all_parents(forest: &[CollectionTreeNode], expanded: &mut HashSet<CollectionId>) {
    for node in forest {
        if !node.children.is_empty() {
            expanded.insert(node.collection.id);
            expand_all_parents(&node.children, expanded);
        }
    }
}

fn collection_subtree_ids(node: &CollectionTreeNode) -> HashSet<CollectionId> {
    let mut ids = HashSet::new();
    ids.insert(node.collection.id);
    for child in &node.children {
        ids.extend(collection_subtree_ids(child));
    }
    ids
}

/// Nest targets: every collection except `child` and its descendants.
fn nest_parent_candidates(forest: &[CollectionTreeNode], child: CollectionId) -> Vec<Collection> {
    let mut blocked = HashSet::new();
    blocked.insert(child);
    fn walk_blocked(
        nodes: &[CollectionTreeNode],
        child: CollectionId,
        blocked: &mut HashSet<CollectionId>,
    ) {
        for node in nodes {
            if node.collection.id == child {
                blocked.extend(collection_subtree_ids(node));
            } else {
                walk_blocked(&node.children, child, blocked);
            }
        }
    }
    walk_blocked(forest, child, &mut blocked);
    flatten_collections(forest)
        .into_iter()
        .filter(|c| !blocked.contains(&c.id))
        .collect()
}

fn can_nest_under(
    forest: &[CollectionTreeNode],
    child: CollectionId,
    parent: CollectionId,
) -> bool {
    nest_parent_candidates(forest, child)
        .iter()
        .any(|c| c.id == parent)
}

/// Sidebar-density menu row (12px type). PopupMenu has no public Small size API.
fn compact_menu_item(
    label: impl Into<SharedString>,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> PopupMenuItem {
    let label = label.into();
    PopupMenuItem::element(move |_, _| div().text_size(px(12.)).child(label.clone()))
        .on_click(handler)
}

/// Payload for dragging a sidebar collection onto another (or the header to unnest).
#[derive(Clone)]
struct CollectionDrag {
    id: CollectionId,
    name: SharedString,
    from_parent: Option<CollectionId>,
}

struct CollectionDragGhost {
    name: SharedString,
}

impl Render for CollectionDragGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_0p5()
            .rounded(px(6.))
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .text_size(px(12.))
            .border_1()
            .border_color(cx.theme().border)
            .child(self.name.clone())
    }
}

#[derive(Clone)]
struct SidebarResize {
    left: bool,
    width: Pixels,
    pointer_x: Pixels,
}

impl Render for SidebarResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

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
const COL_PUBLISHER: usize = 4;
const COL_ADDED: usize = 5;
const COL_EDITED: usize = 6;
const COLUMNS: usize = 7;

/// Stable key for each column, indexed like the COL_ constants above.
/// Settings persist column widths under these names, so widths survive any
/// future column reordering.
const COLUMN_KEYS: [&str; COLUMNS] = [
    "kind",
    "title",
    "creators",
    "published",
    "publisher",
    "added",
    "edited",
];

/// The title column's built-in width, half of its former 620 px, so the
/// grid opens readable on narrower windows.
const DEFAULT_TITLE_WIDTH: f32 = 310.;

const CELL_TEXT_SIZE: Pixels = px(12.);
/// Inline editors read slightly larger than static cells so the caret and
/// typed text stay obvious on the accent row.
const INLINE_CELL_TEXT_SIZE: Pixels = px(13.);
/// Kind dropdown is wider than the 28px icon column so labels aren't clipped.
const INLINE_KIND_MENU_WIDTH: Pixels = px(220.);
/// Header labels are one step below the cell text; the kit's default
/// `text_xs` reads too small at this base font size.
const HEADER_TEXT_SIZE: Pixels = px(10.);

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
    /// Mirrors the table's selected column the same way; row and column
    /// selection are exclusive, and cell text follows whichever portion
    /// sits on the accent tint.
    selected_col: Option<usize>,
    /// Persisted GUI settings: column widths loaded from config.toml ride
    /// through `column`, and finished resizes are written back by
    /// `LibraryApp`.
    settings: Settings,
    /// Bumped on every accepted sort or filter change. Each fetch carries
    /// the generation it was issued for, so a reply landing after a newer
    /// change is dropped instead of clobbering the newer listing.
    listing_gen: u64,
    /// Inline row edit/create session; `render_td` swaps cells for editors
    /// while this is `Ready` or `Create`.
    inline: Option<InlineSession>,
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
            selected_col: None,
            settings,
            listing_gen: 0,
            inline: None,
        }
    }

    /// Apply a header's next cycle state (see [`next_sort`]). A changed
    /// order reloads the grid from the top of the new listing.
    ///
    /// `restore_col`, when set, keeps that column selected after the page
    /// lands so Space can step the cycle again; otherwise the row highlight
    /// follows `selected_id` the way a double-click does.
    fn request_sort(
        &mut self,
        field: ResourceSort,
        selected_id: Option<ResourceId>,
        restore_col: Option<usize>,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        // One reload in flight is enough: a double-click during the fetch
        // lands on the completed order instead of stacking requests.
        if self.loading {
            return;
        }
        self.sort = next_sort(self.sort, field);
        self.reload_from_top(selected_id, restore_col, window, cx);
    }

    /// Re-key the listing under a new search predicate. A keystroke must
    /// never be dropped, so unlike a sort click this reloads even while a
    /// fetch is in flight; the listing generation discards whichever reply
    /// the change made stale.
    fn request_search(
        &mut self,
        needle: Option<NonEmptyStr>,
        selected_id: Option<ResourceId>,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        if self.filter.search == needle {
            return;
        }
        self.filter.search = needle;
        self.reload_from_top(selected_id, None, window, cx);
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
        self.reload_from_top(selected_id, None, window, cx);
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
        self.reload_from_top(selected_id, None, window, cx);
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
        self.reload_from_top(selected_id, None, window, cx);
    }

    /// Fetch the first page under the current sort and filter and replace
    /// the rows. The fetch carries the generation it serves. When
    /// `restore_col` is set, that column stays selected so Space can keep
    /// cycling; otherwise the row highlight follows `selected_id`.
    fn reload_from_top(
        &mut self,
        selected_id: Option<ResourceId>,
        restore_col: Option<usize>,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        self.listing_gen += 1;
        self.loading = true;
        self.error = None;
        // A fresh listing invalidates in-row editors (row indices move).
        self.inline = None;
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
                // Column restore wins so Space can step the sort cycle
                // again; otherwise the highlight follows the selected
                // resource to its new row, and a miss clears selection
                // (the detail panel returns to empty).
                if let Some(col_ix) = restore_col {
                    table.set_selected_col(col_ix, cx);
                } else {
                    let selected_ix = selected_id
                        .and_then(|id| table.delegate().rows.iter().position(|row| row.id == id));
                    match selected_ix {
                        Some(ix) => table.set_selected_row(ix, cx),
                        None => table.clear_selection(cx),
                    }
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
    /// A create draft at row 0 has no id yet.
    pub fn row_id(&self, row_ix: usize) -> Option<ResourceId> {
        let Some(data_ix) = self.data_row_ix(row_ix) else {
            return None;
        };
        self.rows.get(data_ix).map(|row| row.id)
    }

    /// Map a table row index to a stored summary index. `None` means the
    /// create-draft overlay row (not in `rows`).
    fn data_row_ix(&self, row_ix: usize) -> Option<usize> {
        if self.inline.as_ref().is_some_and(|s| s.is_create()) {
            if row_ix == 0 { None } else { Some(row_ix - 1) }
        } else {
            Some(row_ix)
        }
    }

    fn inline_editing_row(&self, row_ix: usize) -> Option<&InlineRowEdit> {
        let session = self.inline.as_ref()?;
        let form = session.ready_form()?;
        (form.row_ix == row_ix).then_some(form)
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
        let (Some(inputs), Some(field)) =
            (self.filter_inputs.as_ref(), filter_field_for_col(col_ix))
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
                                .when(active, |icon| icon.text_color(cx.theme().primary))
                                .when(!active, |icon| icon.text_color(cx.theme().muted_foreground))
                        })
                        // The popup reads kinds, not the funnel it hangs
                        // off: a fixed width keeps it from collapsing to
                        // the icon column's 28 px.
                        .menu_width(px(200.))
                        .w_full(),
                )
                .into_any_element(),
            // The title predicate lives in the always-visible toolbar search.
            FilterField::Title => cell.into_any_element(),
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
                .child(
                    div()
                        .w(px(1.))
                        .h_full()
                        .flex_shrink_0()
                        .bg(cx.theme().border),
                )
                .child(
                    Input::new(&inputs.published_to)
                        .xsmall()
                        .appearance(false)
                        .w_full(),
                )
                .into_any_element(),
        }
    }
}

impl TableDelegate for LibraryDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        COLUMNS
    }

    fn rows_count(&self, _cx: &App) -> usize {
        let draft = self.inline.as_ref().is_some_and(|s| s.is_create()) as usize;
        self.rows.len() + draft
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
            COL_ADDED => Column::new(COLUMN_KEYS[COL_ADDED], "Added")
                .width(stored_width(&self.settings.column_widths, COL_ADDED, 110.))
                .min_width(px(50.)),
            COL_EDITED => Column::new(COLUMN_KEYS[COL_EDITED], "Edited")
                .width(stored_width(&self.settings.column_widths, COL_EDITED, 110.))
                .min_width(px(50.)),
            COL_PUBLISHER => Column::new(COLUMN_KEYS[COL_PUBLISHER], "Publisher")
                .width(stored_width(
                    &self.settings.column_widths,
                    COL_PUBLISHER,
                    160.,
                ))
                .min_width(px(60.)),
            _ => unreachable!("columns_count is {COLUMNS}"),
        }
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        // Reserve the same 1px border on every row so selecting one changes
        // only paint, not its content box. Transparent rules remain invisible.
        div()
            .id(("row", row_ix))
            .border_1()
            .border_color(transparent_white())
            .when(self.selected_row == Some(row_ix), |row| {
                row.border_color(cx.theme().table_active_border.opacity(0.5))
                    .rounded(px(6.))
            })
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
        // Header labels stay on the theme foreground even when the column
        // body is accent-filled; only cells switch to on-accent text.
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
                        let selected_id = table
                            .selected_row()
                            .and_then(|ix| table.delegate().row_id(ix));
                        table
                            .delegate_mut()
                            .request_sort(field, selected_id, None, window, cx);
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
        // Inline editors replace labels for the active row's editable columns.
        if let Some(form) = self.inline_editing_row(row_ix) {
            let on_accent = cell_on_accent(self.selected_row, self.selected_col, row_ix, col_ix);
            let (text, muted) = cell_text_colors(on_accent, cx.theme());
            let active = InlineField::from_col(col_ix) == Some(form.active_field);
            let cell = inline_active_cell(
                cell.text_color(text).text_size(INLINE_CELL_TEXT_SIZE),
                active,
                cx.theme(),
            );
            return match col_ix {
                COL_KIND => {
                    // Icon is painted by us (Combobox triggers stay invisible on
                    // the 28px column); the Combobox sits on top for clicks.
                    let kind = form.selected_kind(cx);
                    let icon_color = kind_color(kind, cx.theme().is_dark());
                    let label: SharedString = kind_label(kind).into();
                    cell.justify_center().child(
                        div()
                            .relative()
                            .size_full()
                            .child(
                                div()
                                    .id(("inline-kind-icon", row_ix))
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(icon_color)
                                    .child(kind_symbol(kind).with_size(px(16.)))
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(label.clone()).build(window, cx)
                                    }),
                            )
                            .child(
                                Combobox::new(&form.kind)
                                    .appearance(false)
                                    .placeholder("Kind")
                                    .search_placeholder("Search kinds")
                                    .menu_width(INLINE_KIND_MENU_WIDTH)
                                    .menu_max_h(px(320.))
                                    .render_trigger(|_, _, _| div().size_full().into_any_element())
                                    .w_full()
                                    .h_full(),
                            ),
                    )
                }
                COL_TITLE => cell.child(
                    Input::new(&form.title)
                        .appearance(false)
                        .text_size(INLINE_CELL_TEXT_SIZE)
                        .text_color(text)
                        .w_full(),
                ),
                COL_CREATORS => cell.child(
                    Combobox::new(&form.creators)
                        .appearance(false)
                        .cleanable(true)
                        .placeholder("Creators")
                        .search_placeholder("Search people")
                        .menu_width(px(280.))
                        .render_trigger(|trigger, _window, cx| {
                            creators_badge_trigger(trigger.selection(), trigger.placeholder(), cx)
                        })
                        .w_full(),
                ),
                COL_PUBLISHED => cell.child(
                    Input::new(&form.published_year)
                        .appearance(false)
                        .text_size(INLINE_CELL_TEXT_SIZE)
                        .text_color(muted)
                        .w_full(),
                ),
                COL_PUBLISHER => cell.child(
                    Input::new(&form.publisher)
                        .appearance(false)
                        .text_size(INLINE_CELL_TEXT_SIZE)
                        .text_color(muted)
                        .w_full(),
                ),
                COL_ADDED => cell.text_color(muted).child(
                    form.record
                        .as_ref()
                        .map(|r| format_recorded_date(r.date_added))
                        .unwrap_or_default(),
                ),
                COL_EDITED => {
                    if form.author_creating {
                        cell.text_color(muted).child("Creating author…")
                    } else if let Some(error) = form.error.clone() {
                        let danger = cx.theme().danger;
                        cell.text_color(danger)
                            .child(div().min_w_0().truncate().child(error))
                    } else {
                        cell.text_color(muted).child(
                            form.record
                                .as_ref()
                                .map(|r| format_recorded_date(r.date_edited))
                                .unwrap_or_default(),
                        )
                    }
                }
                _ => unreachable!("columns_count is {COLUMNS}"),
            };
        }

        let Some(data_ix) = self.data_row_ix(row_ix) else {
            // Create draft occupies row 0 only while editors are Ready/Create;
            // Loading never uses the create slot without a form.
            return cell;
        };
        let Some(row) = self.rows.get(data_ix) else {
            return cell;
        };
        // The accent tint keeps foreground text readable on selected rows and columns.
        let on_accent = cell_on_accent(self.selected_row, self.selected_col, row_ix, col_ix);
        let (text, muted) = cell_text_colors(on_accent, cx.theme());
        let cell = cell.text_color(text);
        match col_ix {
            COL_KIND => {
                let label: SharedString = kind_label(row.kind).into();
                let icon_color = kind_color(row.kind, cx.theme().is_dark());
                cell.justify_center().child(
                    div()
                        .id(("kind", row_ix))
                        .text_color(icon_color)
                        .child(kind_symbol(row.kind).with_size(px(16.)))
                        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx)),
                )
            }
            COL_TITLE => cell.child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(row.title.as_str().to_owned()),
            ),
            COL_CREATORS => cell
                .text_color(muted)
                .child(div().min_w_0().truncate().child(row.creators_label.clone())),
            COL_PUBLISHED => cell
                .text_color(muted)
                .children(row.published_label.as_ref().map(|p| p.as_str().to_owned())),
            COL_ADDED => cell
                .text_color(muted)
                .child(format_recorded_date(row.date_added)),
            COL_EDITED => cell
                .text_color(muted)
                .child(format_recorded_date(row.date_edited)),
            COL_PUBLISHER => cell.text_color(muted).child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(row.publisher.clone().unwrap_or_default()),
            ),
            _ => unreachable!("columns_count is {COLUMNS}"),
        }
    }
    fn cell_text(&self, row_ix: usize, col_ix: usize, _cx: &App) -> String {
        let Some(data_ix) = self.data_row_ix(row_ix) else {
            return String::new();
        };
        let Some(row) = self.rows.get(data_ix) else {
            return String::new();
        };
        match col_ix {
            COL_KIND => kind_label(row.kind),
            COL_TITLE => row.title.as_str().to_owned(),
            COL_CREATORS => row.creators_label.clone(),
            COL_PUBLISHED => row
                .published_label
                .as_ref()
                .map(|p| p.as_str().to_owned())
                .unwrap_or_default(),
            COL_ADDED => format_recorded_date(row.date_added),
            COL_EDITED => format_recorded_date(row.date_edited),
            COL_PUBLISHER => row.publisher.clone().unwrap_or_default(),
            _ => unreachable!("columns_count is {COLUMNS}"),
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
        COL_PUBLISHER => Some(ResourceSort::Publisher),
        COL_ADDED => Some(ResourceSort::DateAdded),
        COL_EDITED => Some(ResourceSort::DateEdited),
        _ => None,
    }
}

/// The filter a column header carries, mirroring the sort mapping above;
/// the kind, title, creators, and published columns each filter their own
/// field, while the record-keeping and publisher columns carry none — the
/// store exposes no predicate for them yet.
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

/// Focus one inline editor without holding a borrow of `InlineRowEdit`.
fn focus_inline_entities(
    field: InlineField,
    kind: &Entity<ComboboxState<SearchableVec<KindItem>>>,
    title: &Entity<InputState>,
    creators: &Entity<ComboboxState<PersonDelegate>>,
    published: &Entity<InputState>,
    publisher: &Entity<InputState>,
    window: &mut Window,
    cx: &mut App,
) {
    let handle = match field {
        InlineField::Kind => kind.read(cx).focus_handle(cx),
        InlineField::Title => title.read(cx).focus_handle(cx),
        InlineField::Creators => creators.read(cx).focus_handle(cx),
        InlineField::Published => published.read(cx).focus_handle(cx),
        InlineField::Publisher => publisher.read(cx).focus_handle(cx),
    };
    window.focus(&handle, cx);
}

/// Advance `active_field` by `delta`, focus that editor, keep the row selected.
fn step_inline_field(
    table: &Entity<TableState<LibraryDelegate>>,
    delta: isize,
    window: &mut Window,
    cx: &mut Context<LibraryApp>,
) {
    let (row_ix, entities, current) = {
        let delegate = table.read(cx).delegate();
        let Some(form) = delegate.inline.as_ref().and_then(|s| s.ready_form()) else {
            return;
        };
        (
            form.row_ix,
            (
                form.kind.clone(),
                form.title.clone(),
                form.creators.clone(),
                form.published_year.clone(),
                form.publisher.clone(),
            ),
            form.active_field,
        )
    };
    let ix = InlineField::ALL
        .iter()
        .position(|f| *f == current)
        .unwrap_or(1);
    let len = InlineField::ALL.len() as isize;
    let next = InlineField::ALL[((ix as isize) + delta).rem_euclid(len) as usize];
    table.update(cx, |table, cx| {
        if let Some(form) = table
            .delegate_mut()
            .inline
            .as_mut()
            .and_then(|s| s.ready_form_mut())
        {
            form.active_field = next;
        }
        table.delegate_mut().selected_row = Some(row_ix);
        table.delegate_mut().selected_col = None;
        table.set_selected_row(row_ix, cx);
    });
    let (kind, title, creators, published, publisher) = entities;
    focus_inline_entities(
        next, &kind, &title, &creators, &published, &publisher, window, cx,
    );
}

/// Whether this cell sits on the selected row or column's accent tint.
fn cell_on_accent(
    selected_row: Option<usize>,
    selected_col: Option<usize>,
    row_ix: usize,
    col_ix: usize,
) -> bool {
    selected_row == Some(row_ix) || selected_col == Some(col_ix)
}

/// Hairline around the field Tab currently owns, distinct from the row tint.
fn inline_active_cell(cell: Div, active: bool, colors: &ThemeColor) -> Div {
    if !active {
        return cell;
    }
    cell.border_1().border_color(colors.ring).rounded(px(2.))
}

/// Selected cells retain normal foreground text on the pale accent tint;
/// secondary columns are dimmed slightly rather than switching to white.
fn cell_text_colors(on_accent: bool, colors: &ThemeColor) -> (Hsla, Hsla) {
    if on_accent {
        (colors.foreground, colors.foreground.opacity(0.75))
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

gpui_kit::actions!([
    ToggleLeftPanel,
    ToggleDetailPanel,
    CycleColumnSort,
    ToggleEditMode,
    BeginInlineEdit,
    BeginNewRow,
    FocusColumnFilter,
    FocusSearch,
    InlineNextField,
    InlinePrevField,
]);

/// Two Esc presses within this window clear filters and hide the filter row.
const CLEAR_FILTERS_DOUBLE_ESC: Duration = Duration::from_millis(400);

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

/// Left-nav selection: All items, Favorites, one kind, tag, or collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarNav {
    All,
    Favorites,
    Kind(ResourceKind),
    Tag(TagId),
    Collection(CollectionId),
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
    left_panel_open: bool,
    resizing_sidebar: Option<SidebarResize>,
    counts: Option<ResourceCounts>,
    counts_error: Option<String>,
    counts_gen: u64,
    /// Library tags for the left-nav section, ordered by text.
    tags: Vec<Tag>,
    /// Nested collections for the left-nav tree.
    collection_forest: Vec<CollectionTreeNode>,
    /// Collection ids whose children are shown in the sidebar tree.
    expanded_collections: HashSet<CollectionId>,
    /// Tree parent of the selected collection row (`None` when the selection is a root).
    collection_focus_parent: Option<CollectionId>,
    /// Collection currently showing an inline rename field.
    renaming_collection: Option<CollectionId>,
    /// Inline rename field; Enter commits.
    collection_name_input: Entity<InputState>,
    /// Detail-pane autocomplete for attaching a library tag or creating one.
    tag_picker: Entity<ComboboxState<TagDelegate>>,
    /// Detail-pane autocomplete for attaching a library collection.
    collection_picker: Entity<ComboboxState<CollectionDelegate>>,
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
    /// First Esc of a double-tap; cleared after the pair fires or times out.
    /// Observed after action dispatch so it still works while an Input owns
    /// the single-Esc binding.
    last_escape_at: Option<Instant>,
    /// Focus trap host for the grid while an inline session is open so Root
    /// Tab/TabPrev cannot walk focus into the toolbar or sidebar.
    inline_trap: FocusHandle,
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
    creators: Entity<ComboboxState<PersonDelegate>>,
    publisher: Entity<InputState>,
    published_year: Entity<InputState>,
    accessed_year: Entity<InputState>,
    series: Entity<InputState>,
    series_number: Entity<InputState>,
    /// Tags staged for save: order matches the form rows; the Combobox
    /// adds, the row × removes, neither writes until Save.
    tags: Vec<Tag>,
    /// Collections staged for save (same attach pattern as tags).
    collections: Vec<Collection>,
    /// The form's own line of complaint: a rejected payload or a failed
    /// save, shown until the next attempt.
    error: Option<String>,
    /// While the save is in flight: the Save button busy, no double sends.
    author_creating: bool,
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
        library_tags: &[Tag],
        library_collections: &[Collection],
        worker: &DbWorker,
        people: &[Person],
        window: &mut Window,
        cx: &mut Context<LibraryApp>,
    ) -> Self {
        let draft = prefill_draft(&record);
        let tags: Vec<Tag> = draft
            .tag_ids
            .iter()
            .filter_map(|id| library_tags.iter().find(|tag| tag.id == *id).cloned())
            .collect();
        let collections: Vec<Collection> = draft
            .collection_ids
            .iter()
            .filter_map(|id| {
                library_collections
                    .iter()
                    .find(|collection| collection.id == *id)
                    .cloned()
            })
            .collect();
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
        let items = person_items(people);
        let selected: Vec<IndexPath> = draft
            .creators
            .iter()
            .filter_map(|spec| {
                items
                    .iter()
                    .position(|item| item.choice == PersonChoice::Existing(spec.person_id))
                    .map(IndexPath::new)
            })
            .collect();
        let creators = cx.new(|cx| {
            ComboboxState::new(PersonDelegate::new(people), selected, window, cx)
                .multiple(true)
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
            ParentRule::Optional { expected } | ParentRule::Required { expected } => Some(expected),
            ParentRule::Forbidden => None,
        };
        Self {
            title: prefill_input(draft.title, "Title", window, cx),
            kind,
            parent,
            creators,
            publisher: prefill_input(draft.publisher, "Publisher", window, cx),
            published_year: prefill_input(draft.published_year, "Year", window, cx),
            accessed_year: prefill_input(draft.accessed_year, "Year", window, cx),
            series: prefill_input(draft.series, "Series id", window, cx),
            series_number: prefill_input(draft.series_number, "e.g. LCL 26", window, cx),
            tags,
            collections,
            author_creating: false,
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
        let value = |input: &Entity<InputState>| input.read(cx).value().to_string();
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
            creators: creators_from_person_ids(
                &selected_person_ids(&self.creators.read(cx).selected_values()),
                &self.record.to_new().creators,
            ),
            tag_ids: self.tags.iter().map(|tag| tag.id).collect(),
            collection_ids: self.collections.iter().map(|c| c.id).collect(),
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
    fn sidebar_resize_handle(
        &self,
        left: bool,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        resize_handle(
            if left {
                "resize-left-sidebar"
            } else {
                "resize-right-sidebar"
            },
            Axis::Horizontal,
        )
        .inside(if left {
            gpui_kit::component::resizable::HandleEdge::Trailing
        } else {
            gpui_kit::component::resizable::HandleEdge::Leading
        })
        .on_drag(
            SidebarResize {
                left,
                width,
                pointer_x: px(0.),
            },
            move |drag, _, window, cx| {
                let mut drag = drag.clone();
                drag.pointer_x = window.mouse_position().x;
                let _ = entity.update(cx, |this, cx| {
                    this.resizing_sidebar = Some(drag.clone());
                    cx.notify();
                });
                cx.new(|_| drag)
            },
        )
    }

    fn resize_sidebar(
        &mut self,
        event: &DragMoveEvent<SidebarResize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.resizing_sidebar.as_ref() else {
            return;
        };
        let delta = event.event.position.x - drag.pointer_x;
        let width = drag.width + if drag.left { delta } else { -delta };
        self.table.update(cx, |table, _| {
            let settings = &mut table.delegate_mut().settings;
            if drag.left {
                settings.left_sidebar_width =
                    Some(f64::from(f32::from(width.clamp(px(140.), px(420.)))));
            } else {
                settings.right_sidebar_width =
                    Some(f64::from(f32::from(width.clamp(px(240.), px(640.)))));
            }
        });
        cx.notify();
    }

    fn finish_sidebar_resize(&mut self, cx: &mut Context<Self>) {
        if self.resizing_sidebar.take().is_none() {
            return;
        }
        if let Err(err) = self
            .table
            .read(cx)
            .delegate()
            .settings
            .save(&self.settings_dir)
        {
            eprintln!("warning: could not save sidebar widths: {err:#}");
        }
        cx.notify();
    }
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
        cx.subscribe_in(
            &table,
            window,
            |this, _table, event: &TableEvent, window, cx| {
                match event {
                    TableEvent::ColumnWidthsChanged(widths) => {
                        this.save_column_widths(widths, cx);
                    }
                    TableEvent::SelectRow(row_ix) => {
                        this.table.update(cx, |table, cx| {
                            let delegate = table.delegate_mut();
                            delegate.selected_row = Some(*row_ix);
                            delegate.selected_col = None;
                            cx.notify();
                        });
                        this.select_row(*row_ix, window, cx);
                    }
                    TableEvent::SelectColumn(col_ix) => {
                        // While inline editing, Tab/arrows mean field focus — not
                        // column selection. Restore the row highlight and move
                        // the caret into the matching editor.
                        if this.has_inline(cx) {
                            this.focus_inline_col(*col_ix, window, cx);
                            return;
                        }
                        // Column highlight is visual only: keep the detail for
                        // the last selected resource so Space can sort without
                        // wiping the sidebar.
                        this.table.update(cx, |table, cx| {
                            let delegate = table.delegate_mut();
                            delegate.selected_row = None;
                            delegate.selected_col = Some(*col_ix);
                            cx.notify();
                        });
                    }
                    TableEvent::ClearSelection => {
                        this.table.update(cx, |table, cx| {
                            let delegate = table.delegate_mut();
                            delegate.selected_row = None;
                            delegate.selected_col = None;
                            cx.notify();
                        });
                        this.clear_selection(cx);
                    }
                    _ => {}
                }
            },
        )
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
                    let delegate = table.delegate_mut();
                    // Header kind filter and Favorites nav are exclusive.
                    if kind.is_some() {
                        delegate.filter.favorite = None;
                    }
                    delegate.request_kind_filter(kind, selected_id, window, cx);
                });
                cx.notify();
            },
        )
        .detach();

        // The box re-keys the listing on every change, clear button
        // included; the listing generation keeps a late reply from
        // clobbering the rows a newer keystroke asked for.
        let title_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search title or publisher"));
        cx.subscribe_in(
            &title_input,
            window,
            |this, input, event: &InputEvent, window, cx| {
                match event {
                    // Focus and blur leave the listing alone; only edits count.
                    InputEvent::Change => {
                        let needle = title_predicate(&input.read(cx).value());
                        let selected_id = this
                            .table
                            .read(cx)
                            .selected_row()
                            .and_then(|ix| this.table.read(cx).delegate().row_id(ix));
                        this.table.update(cx, |table, cx| {
                            table
                                .delegate_mut()
                                .request_search(needle, selected_id, window, cx);
                        });
                        cx.notify();
                    }
                    // Match the creator box: Enter commits the filter and
                    // returns keyboard focus to the grid.
                    InputEvent::PressEnter {
                        secondary: false, ..
                    } => this.focus_table(window, cx),
                    _ => {}
                }
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
        let published_from_input = cx.new(|cx| InputState::new(window, cx).placeholder("From"));
        let published_to_input = cx.new(|cx| InputState::new(window, cx).placeholder("To"));
        for input in [&published_from_input, &published_to_input] {
            cx.subscribe_in(
                input,
                window,
                |this, _input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => this.apply_year_range(window, cx),
                    InputEvent::PressEnter {
                        secondary: false, ..
                    } => this.focus_table(window, cx),
                    _ => {}
                },
            )
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

        // After-dispatch observer: Input/DataTable may consume a single Esc,
        // but a second within the window still clears filters from anywhere.
        // While inline edit is open, the first Esc cancels that session —
        // unless a Combobox menu was open (List in the stack): that Esc only
        // dismisses the menu.
        cx.observe_keystrokes(|this, event, window, cx| {
            if event.keystroke.key != "escape" || event.keystroke.modifiers.modified() {
                this.last_escape_at = None;
                return;
            }
            if this.has_inline(cx) {
                let combobox_menu_open = event.context_stack.iter().any(|ctx| ctx.contains("List"));
                if combobox_menu_open {
                    this.last_escape_at = None;
                    return;
                }
                this.last_escape_at = None;
                this.cancel_inline(cx);
                this.focus_table(window, cx);
                return;
            }
            let now = Instant::now();
            if this
                .last_escape_at
                .is_some_and(|prev| now.duration_since(prev) <= CLEAR_FILTERS_DOUBLE_ESC)
            {
                this.last_escape_at = None;
                this.clear_all_filters(window, cx);
            } else {
                this.last_escape_at = Some(now);
            }
        })
        .detach();
        let worker = table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let counts = worker.resource_counts().await;
            _ = app.update_in(window, |app, _window, cx| {
                app.set_counts(counts, 0, cx);
            });
        })
        .detach();

        // Edit-form Combobox: searchable library tags plus a Create row.
        // Picks stage onto the form; Save writes them with the rest.
        let tag_picker = cx.new(|cx| {
            ComboboxState::new(TagDelegate::new(&[], &[]), Vec::new(), window, cx).searchable(true)
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
                this.stage_tag_on_edit_form(choice, window, cx);
            },
        )
        .detach();
        let worker_tags = table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let tags = worker_tags.list_tags().await.unwrap_or_default();
            let forest = worker_tags
                .list_collection_forest()
                .await
                .unwrap_or_default();
            _ = app.update_in(window, |app, window, cx| {
                app.tags = tags;
                app.set_collection_forest(forest, window, cx);
                app.sync_tag_picker(window, cx);
                cx.notify();
            });
        })
        .detach();

        let collection_picker = cx.new(|cx| {
            ComboboxState::new(CollectionDelegate::new(&[]), Vec::new(), window, cx)
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
                this.stage_collection_on_edit_form(choice, window, cx);
            },
        )
        .detach();

        let collection_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Collection name"));
        cx.subscribe_in(
            &collection_name_input,
            window,
            |this, _input, event: &InputEvent, window, cx| {
                if matches!(
                    event,
                    InputEvent::PressEnter {
                        secondary: false,
                        ..
                    }
                ) {
                    this.commit_collection_rename(window, cx);
                }
            },
        )
        .detach();

        Self {
            table,
            settings_dir,
            published_from_input,
            published_to_input,
            detail_open: true,
            left_panel_open: true,
            resizing_sidebar: None,
            counts: None,
            counts_error: None,
            counts_gen: 0,
            tags: Vec::new(),
            collection_forest: Vec::new(),
            expanded_collections: HashSet::new(),
            collection_focus_parent: None,
            renaming_collection: None,
            collection_name_input,
            tag_picker,
            collection_picker,
            selected: None,
            detail: None,
            detail_cache: DetailCache::new(),
            detail_inflight: HashSet::new(),
            edit: None,
            detail_gens: HashMap::new(),
            last_escape_at: None,
            inline_trap: cx.focus_handle(),
        }
    }

    fn set_counts(
        &mut self,
        result: Result<ResourceCounts, String>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if generation != self.counts_gen {
            return;
        }
        match result {
            Ok(counts) => {
                self.counts = Some(counts);
                self.counts_error = None;
            }
            Err(err) => {
                self.counts_error = Some(err);
            }
        }
        cx.notify();
    }

    fn refresh_counts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.counts_gen += 1;
        let generation = self.counts_gen;
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let result = worker.resource_counts().await;
            let tags = worker.list_tags().await.unwrap_or_default();
            let forest = worker.list_collection_forest().await.unwrap_or_default();
            _ = app.update_in(window, |app, window, cx| {
                app.tags = tags;
                app.set_collection_forest(forest, window, cx);
                app.sync_tag_picker(window, cx);
                app.set_counts(result, generation, cx)
            });
        })
        .detach();
    }

    fn set_collection_forest(
        &mut self,
        forest: Vec<CollectionTreeNode>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut expanded = HashSet::new();
        expand_all_parents(&forest, &mut expanded);
        self.collection_forest = forest;
        self.expanded_collections = expanded;
        self.sync_collection_picker(window, cx);
    }

    fn sync_collection_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let staged = match &self.edit {
            Some(EditFetch::Ready(form)) => form.collections.clone(),
            _ => Vec::new(),
        };
        let staged_ids: HashSet<CollectionId> = staged.iter().map(|c| c.id).collect();
        let available: Vec<Collection> = flatten_collections(&self.collection_forest)
            .into_iter()
            .filter(|c| !staged_ids.contains(&c.id))
            .collect();
        self.collection_picker.update(cx, |state, cx| {
            state.set_items(CollectionDelegate::new(&available), window, cx);
        });
    }

    fn stage_collection_on_edit_form(
        &mut self,
        choice: CollectionChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(collection) = flatten_collections(&self.collection_forest)
            .into_iter()
            .find(|c| c.id == choice.0)
        else {
            return;
        };
        let Some(EditFetch::Ready(form)) = &mut self.edit else {
            return;
        };
        if form.collections.iter().any(|c| c.id == choice.0) {
            return;
        }
        form.collections.push(collection);
        self.sync_collection_picker(window, cx);
        cx.notify();
    }

    fn unstage_collection_on_edit_form(
        &mut self,
        collection_id: CollectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(EditFetch::Ready(form)) = &mut self.edit else {
            return;
        };
        form.collections.retain(|c| c.id != collection_id);
        self.sync_collection_picker(window, cx);
        cx.notify();
    }

    /// New collection at the same tree level as the current selection (sibling),
    /// or a root when no collection is selected.
    fn create_sidebar_collection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Sibling of the selected row: same tree parent (None → new root).
        let parent_id = match self.table.read(cx).delegate().filter.collection {
            Some(_) => self.collection_focus_parent,
            None => None,
        };
        if let Some(parent) = parent_id {
            self.expanded_collections.insert(parent);
        }
        let focus_parent = parent_id;
        let worker = self.table.read(cx).delegate().worker().clone();
        let name = NonEmptyStr::try_new("New collection").expect("literal");
        cx.spawn_in(window, async move |app, window| {
            let result = worker.create_collection(name, parent_id).await;
            _ = app.update_in(window, |app, window, cx| {
                if let Ok(collection) = result {
                    app.collection_focus_parent = focus_parent;
                    app.refresh_counts(window, cx);
                    app.select_sidebar_nav(SidebarNav::Collection(collection.id), window, cx);
                    app.begin_collection_rename(collection.id, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn begin_collection_rename(
        &mut self,
        id: CollectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = flatten_collections(&self.collection_forest)
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| c.name.as_str().to_owned())
            .unwrap_or_default();
        self.renaming_collection = Some(id);
        self.collection_name_input.update(cx, |state, cx| {
            state.set_value(name, window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    fn cancel_collection_rename(&mut self, cx: &mut Context<Self>) {
        self.renaming_collection = None;
        cx.notify();
    }

    fn commit_collection_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.renaming_collection else {
            return;
        };
        let raw = self.collection_name_input.read(cx).value().to_string();
        let Ok(name) = NonEmptyStr::try_new(raw.trim()) else {
            self.cancel_collection_rename(cx);
            return;
        };
        let worker = self.table.read(cx).delegate().worker().clone();
        self.renaming_collection = None;
        cx.spawn_in(window, async move |app, window| {
            let result = worker.rename_collection(id, name).await;
            _ = app.update_in(window, |app, window, cx| {
                if result.is_ok() {
                    app.refresh_counts(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn nest_collection_under(
        &mut self,
        child_id: CollectionId,
        parent_id: CollectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let result = worker.add_collection_parent(child_id, parent_id).await;
            _ = app.update_in(window, |app, window, cx| {
                if result.is_ok() {
                    app.expanded_collections.insert(parent_id);
                    app.collection_focus_parent = Some(parent_id);
                    app.refresh_counts(window, cx);
                    app.select_sidebar_nav(SidebarNav::Collection(child_id), window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Drag-drop reparent: drop the edge from `from_parent` (if any), then nest under `to_parent`.
    fn reparent_collection(
        &mut self,
        child_id: CollectionId,
        from_parent: Option<CollectionId>,
        to_parent: CollectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if from_parent == Some(to_parent) {
            return;
        }
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            if let Some(old_parent) = from_parent {
                if worker
                    .remove_collection_parent(child_id, old_parent)
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let result = worker.add_collection_parent(child_id, to_parent).await;
            _ = app.update_in(window, |app, window, cx| {
                if result.is_ok() {
                    app.expanded_collections.insert(to_parent);
                    app.collection_focus_parent = Some(to_parent);
                    app.refresh_counts(window, cx);
                    app.select_sidebar_nav(SidebarNav::Collection(child_id), window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn unnest_collection(
        &mut self,
        child_id: CollectionId,
        parent_id: CollectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let result = worker.remove_collection_parent(child_id, parent_id).await;
            _ = app.update_in(window, |app, window, cx| {
                if result.is_ok() {
                    app.collection_focus_parent = None;
                    app.refresh_counts(window, cx);
                    app.select_sidebar_nav(SidebarNav::Collection(child_id), window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn delete_sidebar_collection(
        &mut self,
        id: CollectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let result = worker.delete_collection(id).await;
            _ = app.update_in(window, |app, window, cx| {
                if result.is_ok() {
                    app.collection_focus_parent = None;
                    app.renaming_collection = None;
                    app.select_sidebar_nav(SidebarNav::All, window, cx);
                    app.refresh_counts(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_collection_expanded(&mut self, id: CollectionId, cx: &mut Context<Self>) {
        if !self.expanded_collections.remove(&id) {
            self.expanded_collections.insert(id);
        }
        cx.notify();
    }

    fn select_collection_row(
        &mut self,
        id: CollectionId,
        parent: Option<CollectionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .renaming_collection
            .is_some_and(|renaming| renaming != id)
        {
            self.renaming_collection = None;
        }
        self.collection_focus_parent = parent;
        self.select_sidebar_nav(SidebarNav::Collection(id), window, cx);
    }

    /// Rebuild the edit-form tag Combobox from the library list minus tags
    /// already staged on the form, so suggestions stay attachable.
    fn sync_tag_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let staged = match &self.edit {
            Some(EditFetch::Ready(form)) => form.tags.clone(),
            _ => Vec::new(),
        };
        let staged_ids: HashSet<TagId> = staged.iter().map(|tag| tag.id).collect();
        let available: Vec<Tag> = self
            .tags
            .iter()
            .filter(|tag| !staged_ids.contains(&tag.id))
            .cloned()
            .collect();
        let blocked: Vec<String> = staged
            .iter()
            .map(|tag| tag.text.as_str().to_lowercase())
            .collect();
        self.tag_picker.update(cx, |state, cx| {
            state.set_items(TagDelegate::new(&available, &blocked), window, cx);
        });
    }

    /// Stage a Combobox pick onto the open edit form (create a new library
    /// tag when the row was Create). Does not write the resource until Save.
    fn stage_tag_on_edit_form(
        &mut self,
        choice: TagChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.edit, Some(EditFetch::Ready(_))) {
            return;
        }
        match choice {
            TagChoice::Existing(tag_id) => {
                let Some(tag) = self.tags.iter().find(|tag| tag.id == tag_id).cloned() else {
                    return;
                };
                if let Some(EditFetch::Ready(form)) = &mut self.edit {
                    if !form.tags.iter().any(|t| t.id == tag.id) {
                        form.tags.push(tag);
                    }
                }
                self.sync_tag_picker(window, cx);
                cx.notify();
            }
            TagChoice::Create(raw) => {
                let Ok(text) = NonEmptyStr::try_new(raw.trim()) else {
                    return;
                };
                let worker = self.table.read(cx).delegate().worker().clone();
                cx.spawn_in(window, async move |app, window| {
                    let result = match worker.find_tag_by_text(text.clone()).await {
                        Ok(Some(tag)) => Ok(tag),
                        Ok(None) => worker.create_tag(text).await,
                        Err(err) => Err(err),
                    };
                    _ = app.update_in(window, |app, window, cx| {
                        match result {
                            Ok(tag) => {
                                if !app.tags.iter().any(|t| t.id == tag.id) {
                                    app.tags.push(tag.clone());
                                    app.tags
                                        .sort_by(|a, b| a.text.as_str().cmp(b.text.as_str()));
                                }
                                if let Some(EditFetch::Ready(form)) = &mut app.edit {
                                    if !form.tags.iter().any(|t| t.id == tag.id) {
                                        form.tags.push(tag);
                                    }
                                    form.error = None;
                                }
                                app.sync_tag_picker(window, cx);
                            }
                            Err(err) => {
                                if let Some(EditFetch::Ready(form)) = &mut app.edit {
                                    form.error = Some(err);
                                }
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        }
    }

    /// Drop one staged tag from the open edit form.
    fn unstage_tag_on_edit_form(
        &mut self,
        tag_id: TagId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(EditFetch::Ready(form)) = &mut self.edit else {
            return;
        };
        form.tags.retain(|tag| tag.id != tag_id);
        self.sync_tag_picker(window, cx);
        cx.notify();
    }

    fn select_sidebar_nav(&mut self, nav: SidebarNav, window: &mut Window, cx: &mut Context<Self>) {
        let (selected_id, input) = {
            let table = self.table.read(cx);
            let delegate = table.delegate();
            (
                self.selected,
                delegate
                    .filter_inputs
                    .as_ref()
                    .map(|inputs| inputs.kind.clone()),
            )
        };
        let kind = match nav {
            SidebarNav::Kind(kind) => Some(kind),
            SidebarNav::All
            | SidebarNav::Favorites
            | SidebarNav::Tag(_)
            | SidebarNav::Collection(_) => None,
        };
        let favorite = match nav {
            SidebarNav::Favorites => Some(true),
            SidebarNav::All
            | SidebarNav::Kind(_)
            | SidebarNav::Tag(_)
            | SidebarNav::Collection(_) => None,
        };
        let tag = match nav {
            SidebarNav::Tag(id) => Some(id),
            SidebarNav::All
            | SidebarNav::Favorites
            | SidebarNav::Kind(_)
            | SidebarNav::Collection(_) => None,
        };
        let collection = match nav {
            SidebarNav::Collection(id) => Some(id),
            SidebarNav::All | SidebarNav::Favorites | SidebarNav::Kind(_) | SidebarNav::Tag(_) => {
                None
            }
        };
        if let Some(input) = input {
            input.update(cx, |state, cx| {
                state.set_selected_values(&kind.into_iter().collect::<Vec<_>>(), window, cx);
            });
        }
        self.table.update(cx, |table, cx| {
            let delegate = table.delegate_mut();
            // Apply predicates in one reload so nav never briefly flashes
            // the previous listing.
            let kind_changed = delegate.filter.kind != kind;
            let favorite_changed = delegate.filter.favorite != favorite;
            let tag_changed = delegate.filter.tag != tag;
            let collection_changed = delegate.filter.collection != collection;
            if !kind_changed && !favorite_changed && !tag_changed && !collection_changed {
                return;
            }
            delegate.filter.kind = kind;
            delegate.filter.favorite = favorite;
            delegate.filter.tag = tag;
            delegate.filter.collection = collection;
            delegate.reload_from_top(selected_id, None, window, cx);
        });
        if !matches!(nav, SidebarNav::Collection(_)) {
            self.collection_focus_parent = None;
            self.renaming_collection = None;
        }
        cx.notify();
    }

    /// Flip the selected resource's favorite flag from the detail panel.
    fn toggle_favorite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            return;
        };
        let Some(current) = self
            .detail
            .as_ref()
            .and_then(|(loaded, fetch)| (*loaded == id).then_some(fetch))
            .and_then(|fetch| match fetch {
                DetailFetch::Loaded(detail) => Some(detail.favorite),
                _ => None,
            })
        else {
            return;
        };
        let next = !current;
        let worker = self.table.read(cx).delegate().worker().clone();
        // Bump the generation so an in-flight pre-toggle detail cannot
        // overwrite the starred state after the toggle lands.
        let epoch = self.detail_gens.entry(id).or_insert(0);
        *epoch += 1;
        let fetch_epoch = *epoch;
        self.detail_cache.invalidate(&id);
        cx.spawn_in(window, async move |app, window| {
            let reply = worker.set_resource_favorite(id, next).await;
            _ = app.update_in(window, |app, window, cx| {
                match reply {
                    Ok(resource) => {
                        // Refetch detail so names/container stay resolved.
                        app.detail_inflight.remove(&id);
                        if *app.detail_gens.get(&id).unwrap_or(&0) != fetch_epoch {
                            return;
                        }
                        // Optimistically patch favorite on a cached/loaded
                        // detail if present, then refetch for consistency.
                        if let Some((_, DetailFetch::Loaded(detail))) = app.detail.as_mut() {
                            if detail.id == id {
                                detail.favorite = resource.favorite;
                            }
                        }
                        app.fetch_detail(id, window, cx);
                        app.refresh_counts(window, cx);
                        // Favorites view drops unstarred rows immediately.
                        let filtering_favorites =
                            app.table.read(cx).delegate().filter.favorite == Some(true);
                        if filtering_favorites {
                            let selected_id = app.selected;
                            app.table.update(cx, |table, cx| {
                                table
                                    .delegate_mut()
                                    .reload_from_top(selected_id, None, window, cx);
                            });
                        }
                    }
                    Err(err) => {
                        if app.selected == Some(id) {
                            app.detail = Some((id, DetailFetch::Failed(err)));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Row selection: the panel follows the id immediately, then prefetch
    /// warms the rows around it so the next step up or down is instant.
    fn select_row(&mut self, row_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Leaving the inline row cancels in-row editors (create draft included).
        let leave_inline = self
            .table
            .read(cx)
            .delegate()
            .inline
            .as_ref()
            .is_some_and(|session| session.row_ix() != row_ix);
        if leave_inline {
            self.cancel_inline(cx);
        }

        let Some(id) = self.table.read(cx).delegate().row_id(row_ix) else {
            // Create-draft row: no stored id, no detail panel.
            self.selected = None;
            self.detail = None;
            cx.notify();
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
            self.sync_tag_picker(window, cx);
            return;
        }
        if self
            .detail
            .as_ref()
            .is_some_and(|(loaded, _)| *loaded == id)
        {
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
            _ = app.update_in(window, |app, window, cx| {
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
                            app.sync_tag_picker(window, cx);
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

    /// Drop every predicate, empty the header controls, hide the filter
    /// row, and reload when a predicate was active. Widget clears use
    /// silent setters so they do not stack extra fetches.
    fn clear_all_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected_id = self.selected;
        let inputs = self.table.read(cx).delegate().filter_inputs.clone();
        if let Some(inputs) = inputs {
            inputs.title.update(cx, |state, cx| {
                state.set_value("", window, cx);
            });
            inputs.published_from.update(cx, |state, cx| {
                state.set_value("", window, cx);
            });
            inputs.published_to.update(cx, |state, cx| {
                state.set_value("", window, cx);
            });
            inputs.kind.update(cx, |state, cx| {
                state.set_selected_values(&[] as &[ResourceKind], window, cx);
            });
            inputs.creator.update(cx, |state, cx| {
                state.set_selected_values(&[] as &[PersonId], window, cx);
            });
        }
        self.table.update(cx, |table, cx| {
            let delegate = table.delegate_mut();
            let was_active = delegate.filter.is_active();
            delegate.filters_visible = false;
            if was_active {
                delegate.filter = ResourceFilter::none();
                delegate.reload_from_top(selected_id, None, window, cx);
            } else {
                cx.notify();
            }
        });
        cx.notify();
    }

    /// With a filterable column selected, show the filter row and focus
    /// that column's control. Columns without a filter are a no-op.
    fn focus_selected_column_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(col_ix) = self.table.read(cx).selected_col() else {
            return;
        };
        let Some(field) = filter_field_for_col(col_ix) else {
            return;
        };
        if field == FilterField::Title {
            self.focus_search(window, cx);
            return;
        }
        let inputs = self.table.read(cx).delegate().filter_inputs.clone();
        let Some(inputs) = inputs else {
            return;
        };
        self.table.update(cx, |table, cx| {
            let delegate = table.delegate_mut();
            if !delegate.filters_visible {
                delegate.filters_visible = true;
                cx.notify();
            }
        });
        match field {
            FilterField::Kind => {
                let handle = inputs.kind.focus_handle(cx);
                window.focus(&handle, cx);
            }
            FilterField::Title => inputs.title.update(cx, |state, cx| {
                state.focus(window, cx);
            }),
            FilterField::Creators => {
                let handle = inputs.creator.focus_handle(cx);
                window.focus(&handle, cx);
            }
            FilterField::Published => inputs.published_from.update(cx, |state, cx| {
                state.focus(window, cx);
            }),
        }
        cx.notify();
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Keep the inline editor's focus trap intact while a draft is open.
        if self.has_inline(cx) {
            return;
        }
        let input = self
            .table
            .read(cx)
            .delegate()
            .filter_inputs
            .as_ref()
            .map(|inputs| inputs.title.clone());
        if let Some(input) = input {
            input.update(cx, |state, cx| state.focus(window, cx));
        }
    }

    /// Space on a highlighted sortable column steps that column's sort
    /// cycle. Kind has no sort field, so Space is a no-op there.
    fn cycle_column_sort(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(col_ix) = self.table.read(cx).selected_col() else {
            return;
        };
        let Some(field) = sort_field_for_col(col_ix) else {
            return;
        };
        let selected_id = self.selected;
        self.table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .request_sort(field, selected_id, Some(col_ix), window, cx);
        });
    }

    fn toggle_detail_panel(&mut self, cx: &mut Context<Self>) {
        // The detail is fetched on selection whether or not the panel shows,
        // so opening it never waits on a fetch.
        self.detail_open = !self.detail_open;
        cx.notify();
    }

    fn toggle_left_panel(&mut self, cx: &mut Context<Self>) {
        self.left_panel_open = !self.left_panel_open;
        cx.notify();
    }

    /// Enter opens the sidebar edit form (and the sidebar if it was closed)
    /// and focuses the title field; a second press closes without saving.
    fn toggle_edit_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.edit {
            Some(EditFetch::Ready(_)) => self.cancel_edit(window, cx),
            Some(EditFetch::Loading(_)) => {}
            None => self.request_edit(window, cx),
        }
    }

    fn focus_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.table.focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// Whether an inline edit or create session is open on the table.
    fn has_inline(&self, cx: &App) -> bool {
        self.table.read(cx).delegate().inline.is_some()
    }

    /// Drop the inline session without writing.
    fn cancel_inline(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |table, _| {
            table.delegate_mut().inline = None;
            // Column selection is suppressed while editing so Tab/arrows
            // can drive cell focus instead.
            table.col_selectable = true;
        });
        cx.notify();
    }

    /// Mark which inline cell owns the caret / menu; drives the active border.
    fn set_inline_active_field(&mut self, field: InlineField, cx: &mut Context<Self>) {
        let changed = self.table.update(cx, |table, cx| {
            let Some(form) = table
                .delegate_mut()
                .inline
                .as_mut()
                .and_then(|s| s.ready_form_mut())
            else {
                return false;
            };
            if form.active_field == field {
                return false;
            }
            form.active_field = field;
            cx.notify();
            true
        });
        if changed {
            cx.notify();
        }
    }

    /// Keep `active_field` in sync when the user clicks into a cell (Tab
    /// already updates it via [`Self::focus_inline_delta`]).
    fn wire_inline_active_tracking(
        form: &InlineRowEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (field, input) in [
            (InlineField::Title, &form.title),
            (InlineField::Published, &form.published_year),
            (InlineField::Publisher, &form.publisher),
        ] {
            cx.subscribe_in(input, window, move |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Focus) {
                    this.set_inline_active_field(field, cx);
                }
            })
            .detach();
        }
        for (field, combo) in [
            (InlineField::Kind, form.kind.focus_handle(cx)),
            (InlineField::Creators, form.creators.focus_handle(cx)),
        ] {
            cx.on_focus(&combo, window, move |this, _window, cx| {
                this.set_inline_active_field(field, cx);
            })
            .detach();
        }
    }

    /// Both author comboboxes create people through the worker, then select the
    /// saved id on the same still-open form. A late reply cannot edit a new form.
    fn wire_author_creation(
        picker: &Entity<ComboboxState<PersonDelegate>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.subscribe_in(
            picker,
            window,
            |app, picker, event: &ComboboxEvent<PersonDelegate>, window, cx| {
                let ComboboxEvent::Change(choices) = event else {
                    return;
                };
                let Some(raw) = choices.iter().find_map(|choice| match choice {
                    PersonChoice::Create(raw) => Some(raw.clone()),
                    PersonChoice::Existing(_) => None,
                }) else {
                    return;
                };
                let existing: Vec<_> = choices
                    .iter()
                    .filter(|choice| matches!(choice, PersonChoice::Existing(_)))
                    .cloned()
                    .collect();
                picker.update(cx, |state, cx| {
                    state.set_selected_values(&existing, window, cx)
                });
                let inline = app
                    .table
                    .read(cx)
                    .delegate()
                    .inline
                    .as_ref()
                    .and_then(|session| session.ready_form())
                    .is_some_and(|form| form.creators == *picker);
                let sidebar =
                    matches!(&app.edit, Some(EditFetch::Ready(form)) if form.creators == *picker);
                if !inline && !sidebar {
                    return;
                }
                let busy = if inline {
                    app.table
                        .read(cx)
                        .delegate()
                        .inline
                        .as_ref()
                        .and_then(|s| s.ready_form())
                        .is_some_and(|f| f.author_creating)
                } else {
                    matches!(&app.edit, Some(EditFetch::Ready(f)) if f.author_creating)
                };
                if busy {
                    return;
                }
                let Ok(name) = person_name_from_query(&raw) else {
                    return;
                };
                if inline {
                    app.table.update(cx, |table, cx| {
                        if let Some(form) = table
                            .delegate_mut()
                            .inline
                            .as_mut()
                            .and_then(|s| s.ready_form_mut())
                        {
                            form.author_creating = true;
                            form.error = None;
                            cx.notify();
                        }
                    });
                } else if let Some(EditFetch::Ready(form)) = &mut app.edit {
                    form.author_creating = true;
                    form.error = None;
                    cx.notify();
                }
                let picker = picker.clone();
                let worker = app.table.read(cx).delegate().worker().clone();
                cx.spawn_in(window, async move |app, window| {
                    let result = match worker.create_person(name).await {
                        Ok(person) => worker.list_persons().await.map(|people| (person, people)),
                        Err(err) => Err(err),
                    };
                    _ = app.update_in(window, |app, window, cx| {
                        let still_open = if inline {
                            app.table
                                .read(cx)
                                .delegate()
                                .inline
                                .as_ref()
                                .and_then(|s| s.ready_form())
                                .is_some_and(|f| f.creators == picker)
                        } else {
                            matches!(&app.edit, Some(EditFetch::Ready(f)) if f.creators == picker)
                        };
                        if !still_open {
                            return;
                        }
                        let error = match result {
                            Ok((person, people)) => {
                                let mut values: Vec<_> = picker
                                    .read(cx)
                                    .selected_values()
                                    .into_iter()
                                    .filter(|choice| matches!(choice, PersonChoice::Existing(_)))
                                    .collect();
                                values.push(PersonChoice::Existing(person.id));
                                picker.update(cx, |state, cx| {
                                    state.set_items(PersonDelegate::new(&people), window, cx);
                                    state.set_selected_values(&values, window, cx);
                                });
                                None
                            }
                            Err(err) => Some(err),
                        };
                        if inline {
                            app.table.update(cx, |table, cx| {
                                if let Some(form) = table
                                    .delegate_mut()
                                    .inline
                                    .as_mut()
                                    .and_then(|s| s.ready_form_mut())
                                {
                                    form.author_creating = false;
                                    form.error = error;
                                    cx.notify();
                                }
                            });
                        } else if let Some(EditFetch::Ready(form)) = &mut app.edit {
                            form.author_creating = false;
                            form.error = error;
                        }
                        cx.notify();
                    });
                })
                .detach();
            },
        )
        .detach();
    }

    /// Focus the inline editor for `col_ix`, keeping the edit row selected.
    fn focus_inline_col(&mut self, col_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let field = InlineField::from_col(col_ix);
        let (row_ix, entities, active) = {
            let delegate = self.table.read(cx).delegate();
            let session = delegate.inline.as_ref();
            let row_ix = session.map(|s| s.row_ix());
            let form = session.and_then(|s| s.ready_form()).map(|f| {
                (
                    f.kind.clone(),
                    f.title.clone(),
                    f.creators.clone(),
                    f.published_year.clone(),
                    f.publisher.clone(),
                    f.active_field,
                )
            });
            let (entities, active) = match form {
                Some((kind, title, creators, published, publisher, active)) => {
                    (Some((kind, title, creators, published, publisher)), active)
                }
                None => (None, InlineField::Title),
            };
            (row_ix, entities, active)
        };
        // Non-editable cols (Added/Edited) still restore the row so a
        // DataTable column loop cannot leave selection mode as Column.
        let field = field.unwrap_or(active);
        self.table.update(cx, |table, cx| {
            if let Some(row_ix) = row_ix {
                table.delegate_mut().selected_row = Some(row_ix);
                table.delegate_mut().selected_col = None;
                table.set_selected_row(row_ix, cx);
            }
            if let Some(form) = table
                .delegate_mut()
                .inline
                .as_mut()
                .and_then(|s| s.ready_form_mut())
            {
                form.active_field = field;
            }
        });
        if let Some((kind, title, creators, published, publisher)) = entities {
            focus_inline_entities(
                field, &kind, &title, &creators, &published, &publisher, window, cx,
            );
        }
        cx.notify();
    }

    /// Tab / Shift-Tab / ← / → while inline: move between editable cells.
    fn focus_inline_delta(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        step_inline_field(&self.table, delta, window, cx);
        cx.notify();
    }

    /// E toggles spreadsheet-style inline edit for the selected row.
    fn toggle_inline_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_inline(cx) {
            self.cancel_inline(cx);
            self.focus_table(window, cx);
            return;
        }
        self.begin_inline_edit(window, cx);
    }

    /// Fetch the selected record and open borderless cell editors on its row.
    fn begin_inline_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            return;
        };
        let Some(row_ix) = self.table.read(cx).selected_row() else {
            return;
        };
        // Sidebar and inline are mutually exclusive.
        self.edit = None;
        self.table.update(cx, |table, cx| {
            table.col_selectable = false;
            table.delegate_mut().selected_col = None;
            table.delegate_mut().inline = Some(InlineSession::Loading { id, row_ix });
            table.set_selected_row(row_ix, cx);
        });
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let record = worker.resource(id).await;
            let people = worker.list_persons().await.unwrap_or_default();
            _ = app.update_in(window, |app, window, cx| {
                let still = matches!(
                    app.table.read(cx).delegate().inline.as_ref(),
                    Some(InlineSession::Loading {
                        id: loading,
                        row_ix: _
                    }) if *loading == id
                );
                if !still || app.selected != Some(id) {
                    return;
                }
                match record {
                    Ok(record) => {
                        let form = InlineRowEdit::for_edit(record, &people, row_ix, window, cx);
                        // Land the form in the delegate before focusing so
                        // the next paint owns the focused Input entity.
                        let title = form.title.clone();
                        Self::wire_inline_active_tracking(&form, window, cx);
                        Self::wire_author_creation(&form.creators, window, cx);
                        app.table.update(cx, |table, _| {
                            table.delegate_mut().inline =
                                Some(InlineSession::Ready(Box::new(form)));
                        });
                        let handle = title.read(cx).focus_handle(cx);
                        window.focus(&handle, cx);
                    }
                    Err(err) => {
                        app.table.update(cx, |table, _| {
                            table.delegate_mut().inline = None;
                            table.col_selectable = true;
                        });
                        if app.selected == Some(id) {
                            app.detail = Some((id, DetailFetch::Failed(err)));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// N opens a new Book draft as table row 0 in the same inline chrome.
    fn begin_new_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            self.table.read(cx).delegate().inline.as_ref(),
            Some(InlineSession::Create(_))
        ) {
            return;
        }
        self.edit = None;
        self.table.update(cx, |table, _| {
            table.delegate_mut().inline = None;
        });
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let people = worker.list_persons().await.unwrap_or_default();
            _ = app.update_in(window, |app, window, cx| {
                let form = InlineRowEdit::for_create(&people, window, cx);
                let title = form.title.clone();
                Self::wire_inline_active_tracking(&form, window, cx);
                Self::wire_author_creation(&form.creators, window, cx);
                app.table.update(cx, |table, cx| {
                    table.col_selectable = false;
                    table.delegate_mut().inline = Some(InlineSession::Create(Box::new(form)));
                    table.delegate_mut().selected_row = Some(0);
                    table.delegate_mut().selected_col = None;
                    table.set_selected_row(0, cx);
                });
                let handle = title.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
                app.selected = None;
                app.detail = None;
                cx.notify();
            });
        })
        .detach();
    }

    /// ⌘↩ while inline: update or create from the cell editors.
    fn save_inline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self
            .table
            .read(cx)
            .delegate()
            .inline
            .as_ref()
            .and_then(|s| s.ready_form())
        else {
            return;
        };
        if form.saving || form.author_creating {
            return;
        }
        let draft = form.draft(cx);
        let record = form.record.clone();
        let payload = if let Some(record) = &record {
            build_update(record, &draft)
        } else {
            build_create(&draft)
        };
        let new = match payload {
            Ok(new) => new,
            Err(message) => {
                self.table.update(cx, |table, _| {
                    if let Some(form) = table
                        .delegate_mut()
                        .inline
                        .as_mut()
                        .and_then(|s| s.ready_form_mut())
                    {
                        form.error = Some(message);
                    }
                });
                cx.notify();
                return;
            }
        };
        self.table.update(cx, |table, _| {
            if let Some(form) = table
                .delegate_mut()
                .inline
                .as_mut()
                .and_then(|s| s.ready_form_mut())
            {
                form.saving = true;
                form.error = None;
            }
        });
        self.focus_table(window, cx);
        let worker = self.table.read(cx).delegate().worker().clone();
        let update_id = record.as_ref().map(|r| r.id);
        cx.spawn_in(window, async move |app, window| {
            let reply = if let Some(id) = update_id {
                worker.update_resource(id, new).await.map(|r| r.id)
            } else {
                worker.create_resource(new).await.map(|r| r.id)
            };
            _ = app.update_in(window, |app, window, cx| {
                match reply {
                    Ok(id) => {
                        app.table.update(cx, |table, _| {
                            table.delegate_mut().inline = None;
                            table.col_selectable = true;
                        });
                        app.selected = Some(id);
                        *app.detail_gens.entry(id).or_default() += 1;
                        app.detail_inflight.remove(&id);
                        app.detail_cache.invalidate(&id);
                        app.detail = Some((id, DetailFetch::Loading));
                        app.fetch_detail(id, window, cx);
                        app.refresh_counts(window, cx);
                        app.table.update(cx, |table, cx| {
                            table
                                .delegate_mut()
                                .reload_from_top(Some(id), None, window, cx);
                        });
                    }
                    Err(err) => {
                        app.table.update(cx, |table, _| {
                            if let Some(form) = table
                                .delegate_mut()
                                .inline
                                .as_mut()
                                .and_then(|s| s.ready_form_mut())
                            {
                                form.error = Some(err);
                                form.saving = false;
                            }
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
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
        self.cancel_inline(cx);
        self.edit = Some(EditFetch::Loading(id));
        self.detail_open = true;
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let reply = match worker.resource(id).await {
                Ok(record) => worker.list_persons().await.map(|people| (record, people)),
                Err(err) => Err(err),
            };
            _ = app.update_in(window, |app, window, cx| {
                match reply {
                    Ok((record, people)) => {
                        // The selection moved or re-clicked Edit since the
                        // request: the reply belongs to a form nobody is
                        // waiting for, so it is dropped.
                        if app.selected == Some(id)
                            && matches!(app.edit, Some(EditFetch::Loading(_)))
                        {
                            let parent_id = record.parent_id;
                            let rule = record.kind.parent_rule();
                            let library_collections = flatten_collections(&app.collection_forest);
                            let form = EditForm::new(
                                record,
                                &app.tags,
                                &library_collections,
                                &worker,
                                &people,
                                window,
                                cx,
                            );
                            let title = form.title.clone();
                            Self::wire_author_creation(&form.creators, window, cx);
                            app.edit = Some(EditFetch::Ready(Box::new(form)));
                            // Land the caret in the title so `e` opens ready
                            // to type; parent options load in the background.
                            title.update(cx, |state, cx| {
                                state.focus(window, cx);
                            });
                            app.sync_tag_picker(window, cx);
                            app.sync_collection_picker(window, cx);
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
            ParentRule::Optional { expected } | ParentRule::Required { expected } => Some(expected),
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
                missing_parent =
                    stored_parent.filter(|parent| !items.iter().any(|item| item.id == *parent));
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
    fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit = None;
        self.sync_tag_picker(window, cx);
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
        if form.saving || form.author_creating {
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
        // Hand the keyboard back to the grid so arrow keys work while the
        // save runs; a rejected draft above keeps focus on the form.
        self.focus_table(window, cx);
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
                        app.refresh_counts(window, cx);
                        // The row may have moved under the current sort;
                        // the reload re-walks the listing and keeps the
                        // highlight on the edited id.
                        app.table.update(cx, |table, cx| {
                            table
                                .delegate_mut()
                                .reload_from_top(Some(id), None, window, cx);
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

    fn sidebar_row(
        &self,
        index: usize,
        nav: SidebarNav,
        label: String,
        icon: Icon,
        count: Option<u64>,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let hover = cx.theme().accent;
        h_flex()
            .id(("sidebar-entry", index))
            .w_full()
            .min_w_0()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(px(8.))
            .when(selected, |row| {
                row.bg(sidebar_selected_background(cx.theme()))
            })
            .hover(move |row| row.bg(hover))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_sidebar_nav(nav, window, cx);
            }))
            .child(icon.with_size(px(16.)))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .when_some(count, |row, count| {
                row.child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(count.to_string()),
                )
            })
    }

    fn render_left_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let filter = self.table.read(cx).delegate().filter.clone();
        let selected_kind = filter.kind;
        let selected_tag = filter.tag;
        let selected_collection = filter.collection;
        let all_selected = filter.kind.is_none()
            && filter.favorite.is_none()
            && filter.tag.is_none()
            && filter.collection.is_none();
        let favorites_selected = filter.favorite == Some(true);
        let collection_rows =
            visible_collection_rows(&self.collection_forest, &self.expanded_collections);
        let kinds = ResourceKind::ALL
            .into_iter()
            .enumerate()
            .filter(|(_, kind)| {
                self.counts.as_ref().is_none_or(|counts| {
                    counts.by_kind.get(kind).copied().unwrap_or(0) > 0
                        || selected_kind == Some(*kind)
                })
            })
            .collect::<Vec<_>>();
        v_flex()
            .id("library-navigation")
            .size_full()
            .p_2()
            .overflow_y_scroll()
            .text_size(px(12.))
            .child(self.sidebar_row(
                0,
                SidebarNav::All,
                "All items".into(),
                symbol("books.vertical", AssetIconName::Library),
                self.counts.as_ref().map(|counts| counts.all_items),
                all_selected,
                cx,
            ))
            .child(
                self.sidebar_row(
                    1,
                    SidebarNav::Favorites,
                    "Favorites".into(),
                    symbol("star", AssetIconName::Star)
                        .text_color(favorite_star_color(cx.theme().is_dark())),
                    self.counts.as_ref().map(|counts| counts.favorites),
                    favorites_selected,
                    cx,
                ),
            )
            .child(
                div()
                    .pt_4()
                    .pb_1()
                    .px_2()
                    .text_color(cx.theme().muted_foreground)
                    .child("Kinds"),
            )
            .children(kinds.into_iter().map(|(index, kind)| {
                self.sidebar_row(
                    index + 2,
                    SidebarNav::Kind(kind),
                    sidebar_kind_label(kind),
                    kind_symbol(kind).text_color(kind_color(kind, cx.theme().is_dark())),
                    self.counts
                        .as_ref()
                        .map(|counts| counts.by_kind.get(&kind).copied().unwrap_or(0)),
                    selected_kind == Some(kind),
                    cx,
                )
            }))
            .child(
                div()
                    .pt_4()
                    .pb_1()
                    .px_2()
                    .text_color(cx.theme().muted_foreground)
                    .child("Tags"),
            )
            .children(self.tags.iter().enumerate().filter_map(|(index, tag)| {
                let count = self
                    .counts
                    .as_ref()
                    .map(|counts| counts.by_tag.get(&tag.id).copied().unwrap_or(0))
                    .unwrap_or(0);
                if count == 0 && selected_tag != Some(tag.id) {
                    return None;
                }
                Some(self.sidebar_row(
                    index + 100,
                    SidebarNav::Tag(tag.id),
                    tag.text.as_str().to_owned(),
                    symbol("tag", AssetIconName::Library).with_size(px(16.)),
                    Some(count),
                    selected_tag == Some(tag.id),
                    cx,
                ))
            }))
            .child(
                h_flex()
                    .id("collections-header")
                    .pt_4()
                    .pb_1()
                    .px_2()
                    .items_center()
                    .gap_1()
                    .rounded(px(8.))
                    .can_drop(|dragged: &dyn Any, _, _| {
                        dragged
                            .downcast_ref::<CollectionDrag>()
                            .is_some_and(|drag| drag.from_parent.is_some())
                    })
                    .on_drop(cx.listener(|this, drag: &CollectionDrag, window, cx| {
                        if let Some(parent_id) = drag.from_parent {
                            this.unnest_collection(drag.id, parent_id, window, cx);
                        }
                    }))
                    .drag_over::<CollectionDrag>(|style, _, _, cx| style.bg(cx.theme().accent))
                    .child(
                        div()
                            .flex_1()
                            .text_color(cx.theme().muted_foreground)
                            .child("Collections"),
                    )
                    .child(
                        Button::new("new-collection")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Plus).xsmall())
                            .tooltip(if selected_collection.is_some() {
                                "New collection at this level"
                            } else {
                                "New collection"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.create_sidebar_collection(window, cx)
                            })),
                    ),
            )
            .children(collection_rows.into_iter().enumerate().map(
                |(index, (collection, depth, has_children, tree_parent))| {
                    let id = collection.id;
                    let name = collection.name.as_str().to_owned();
                    let drag_name = SharedString::from(name.as_str());
                    let expanded = self.expanded_collections.contains(&id);
                    let selected = selected_collection == Some(id);
                    let renaming = self.renaming_collection == Some(id);
                    let hover = cx.theme().accent;
                    let nest_targets = nest_parent_candidates(&self.collection_forest, id);
                    let forest = self.collection_forest.clone();
                    let app = cx.weak_entity();
                    h_flex()
                        .id(("collection-row", index))
                        .w_full()
                        .items_center()
                        .gap_0p5()
                        .pl(px(depth as f32 * 8.))
                        .pr_2()
                        .py_1()
                        .rounded(px(8.))
                        .when(selected, |row| {
                            row.bg(sidebar_selected_background(cx.theme()))
                        })
                        .hover(move |row| row.bg(hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_collection_row(id, tree_parent, window, cx);
                        }))
                        .when(!renaming, |row| {
                            row.on_drag(
                                CollectionDrag {
                                    id,
                                    name: drag_name,
                                    from_parent: tree_parent,
                                },
                                |drag, _, _, cx| {
                                    cx.new(|_| CollectionDragGhost {
                                        name: drag.name.clone(),
                                    })
                                },
                            )
                        })
                        .can_drop(move |dragged: &dyn Any, _, _| {
                            dragged
                                .downcast_ref::<CollectionDrag>()
                                .is_some_and(|drag| can_nest_under(&forest, drag.id, id))
                        })
                        .on_drop(cx.listener(move |this, drag: &CollectionDrag, window, cx| {
                            this.reparent_collection(drag.id, drag.from_parent, id, window, cx);
                        }))
                        .drag_over::<CollectionDrag>(|style, _, _, cx| style.bg(cx.theme().accent))
                        .context_menu({
                            let nest_targets = nest_targets.clone();
                            move |menu, window, menu_cx| {
                                let app_rename = app.clone();
                                let app_unnest = app.clone();
                                let app_delete = app.clone();
                                let mut menu = menu.min_w(px(0.)).item(compact_menu_item(
                                    "Rename",
                                    move |_, window, cx| {
                                        let _ = app_rename.update(cx, |this, cx| {
                                            this.select_collection_row(id, tree_parent, window, cx);
                                            this.begin_collection_rename(id, window, cx);
                                        });
                                    },
                                ));
                                if !nest_targets.is_empty() {
                                    let app_nest = app.clone();
                                    menu = menu.submenu("Nest under", window, menu_cx, {
                                        let nest_targets = nest_targets.clone();
                                        move |menu, _, _| {
                                            let mut menu = menu.min_w(px(0.));
                                            for target in &nest_targets {
                                                let parent_id = target.id;
                                                let label = target.name.as_str().to_owned();
                                                let app_nest = app_nest.clone();
                                                menu = menu.item(compact_menu_item(
                                                    label,
                                                    move |_, window, cx| {
                                                        let _ = app_nest.update(cx, |this, cx| {
                                                            this.nest_collection_under(
                                                                id, parent_id, window, cx,
                                                            );
                                                        });
                                                    },
                                                ));
                                            }
                                            menu
                                        }
                                    });
                                }
                                if let Some(parent_id) = tree_parent {
                                    menu = menu.item(compact_menu_item(
                                        "Unnest",
                                        move |_, window, cx| {
                                            let _ = app_unnest.update(cx, |this, cx| {
                                                this.unnest_collection(id, parent_id, window, cx);
                                            });
                                        },
                                    ));
                                }
                                menu.item(PopupMenuItem::separator())
                                    .item(compact_menu_item("Delete", move |_, window, cx| {
                                        let _ = app_delete.update(cx, |this, cx| {
                                            this.delete_sidebar_collection(id, window, cx);
                                        });
                                    }))
                            }
                        })
                        .child(
                            div()
                                .w(px(12.))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(cx.theme().muted_foreground)
                                .when(has_children, |slot| {
                                    let chevron = if expanded {
                                        symbol("chevron.down", AssetIconName::ChevronDown)
                                    } else {
                                        symbol("chevron.right", AssetIconName::ChevronRight)
                                    }
                                    .with_size(px(12.));
                                    slot.child(
                                        Button::new(SharedString::from(format!(
                                            "toggle-collection-{id}"
                                        )))
                                        .ghost()
                                        .xsmall()
                                        .size(px(12.))
                                        .icon(chevron)
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.toggle_collection_expanded(id, cx)
                                            }),
                                        ),
                                    )
                                }),
                        )
                        .child(
                            symbol(
                                if expanded && has_children {
                                    "folder.fill"
                                } else {
                                    "folder"
                                },
                                if expanded && has_children {
                                    AssetIconName::FolderOpen
                                } else {
                                    AssetIconName::Folder
                                },
                            )
                            .with_size(px(14.))
                            .text_color(cx.theme().muted_foreground)
                            .flex_shrink_0(),
                        )
                        .child(if renaming {
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&self.collection_name_input).small().w_full())
                                .into_any_element()
                        } else {
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(name)
                                .into_any_element()
                        })
                },
            ))
            .when_some(self.counts_error.as_ref(), |panel, error| {
                panel.child(
                    div()
                        .px_2()
                        .pt_2()
                        .text_color(cx.theme().danger)
                        .child(format!("Counts unavailable: {error}")),
                )
            })
    }

    /// The right-side sidebar: the edit form while one is open, the detail
    /// fields otherwise. The form keeps the sidebar's anchor, width, and
    /// surface. The detail shows the edit pencil below the byline; no
    /// row selected means nothing to edit, and an open form means the
    /// sidebar already is the editor.
    fn render_side_panel(&self, cx: &mut Context<Self>) -> Div {
        match &self.edit {
            Some(EditFetch::Ready(form)) => self.render_edit_panel(form, cx),
            Some(EditFetch::Loading(_)) => render_panel(DetailView::Loading, None, None, cx),
            None => {
                let favorited = match self.detail_view() {
                    DetailView::Fields(detail) => Some(detail.favorite),
                    _ => None,
                };
                let actions = self.selected.is_some().then(|| {
                    let star = match favorited {
                        Some(true) => symbol("star.fill", AssetIconName::StarFill),
                        _ => symbol("star", AssetIconName::Star),
                    }
                    .with_size(px(16.))
                    .text_color(favorite_star_color(cx.theme().is_dark()));
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("toggle-favorite")
                                .ghost()
                                .icon(star)
                                .tooltip(if favorited == Some(true) {
                                    "Remove from favorites"
                                } else {
                                    "Add to favorites"
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_favorite(window, cx)
                                })),
                        )
                        .child(
                            Button::new("edit-resource")
                                .ghost()
                                .icon(symbol("pencil", AssetIconName::PenLine).with_size(px(16.)))
                                .tooltip("Edit resource")
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.request_edit(window, cx)
                                    }),
                                ),
                        )
                        .into_any_element()
                });
                // Read-only hashtags; editing happens on the edit form.
                let tags_value = match self.detail_view() {
                    DetailView::Fields(detail) if !detail.tags.is_empty() => Some(
                        h_flex()
                            .flex_wrap()
                            .gap_x_2()
                            .gap_y_1()
                            .children(
                                detail
                                    .tags
                                    .iter()
                                    .map(|tag| div().child(format!("#{}", tag.text.as_str()))),
                            )
                            .into_any_element(),
                    ),
                    _ => None,
                };
                render_panel(self.detail_view(), actions, tags_value, cx)
            }
        }
    }

    /// The form: Save/Cancel above, one labelled row per scalar, the error
    /// line under the buttons. Scrollable like the detail sidebar, so a
    /// short window still reaches every row. Parent is a searchable
    /// dropdown of library titles, not an id to type: users pick the
    /// containing work by name.
    fn render_edit_panel(&self, form: &EditForm, cx: &mut Context<Self>) -> Div {
        let saving = form.saving || form.author_creating;
        let error = form.error.clone();
        let danger = cx.theme().danger;

        v_flex()
            .h_full()
            .flex_shrink_0()
            .w_full()
            .bg(cx.theme().background)
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
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel_edit(window, cx)
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
                    .child(edit_field("Title", padded_input(&form.title).w_full()))
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
                        "Authors",
                        Combobox::new(&form.creators)
                            .small()
                            .placeholder("Add authors")
                            .search_placeholder("Search people")
                            .cleanable(true)
                            .w_full(),
                    ))
                    .child(edit_field(
                        "Publisher",
                        padded_input(&form.publisher).w_full(),
                    ))
                    .child(edit_field(
                        "Published",
                        padded_input(&form.published_year).w_full(),
                    ))
                    .child(edit_field(
                        "Accessed",
                        padded_input(&form.accessed_year).w_full(),
                    ))
                    .child(edit_field("Series", padded_input(&form.series).w_full()))
                    .child(edit_field(
                        "Series no.",
                        padded_input(&form.series_number).w_full(),
                    ))
                    .child(edit_field(
                        "Tags",
                        v_flex()
                            .gap_1()
                            .w_full()
                            .child(
                                Combobox::new(&self.tag_picker)
                                    .small()
                                    .placeholder("Add tag")
                                    .search_placeholder("Search or create...")
                                    .w_full(),
                            )
                            .children(form.tags.iter().map(|tag| {
                                let tag_id = tag.id;
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(tag.text.as_str().to_owned()),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "unstage-tag-{tag_id}"
                                        )))
                                        .ghost()
                                        .xsmall()
                                        .icon(Icon::new(IconName::Close).xsmall())
                                        .tooltip("Remove tag")
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.unstage_tag_on_edit_form(tag_id, window, cx)
                                            }),
                                        ),
                                    )
                            })),
                    ))
                    .child(edit_field(
                        "Collections",
                        v_flex()
                            .gap_1()
                            .w_full()
                            .child(
                                Combobox::new(&self.collection_picker)
                                    .small()
                                    .placeholder("Add collection")
                                    .search_placeholder("Search collections...")
                                    .w_full(),
                            )
                            .children(form.collections.iter().map(|collection| {
                                let collection_id = collection.id;
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(collection.name.as_str().to_owned()),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "unstage-collection-{collection_id}"
                                        )))
                                        .ghost()
                                        .xsmall()
                                        .icon(Icon::new(IconName::Close).xsmall())
                                        .tooltip("Remove collection")
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.unstage_collection_on_edit_form(
                                                    collection_id,
                                                    window,
                                                    cx,
                                                )
                                            }),
                                        ),
                                    )
                            })),
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
        .child(div().flex_1().min_w_0().child(control))
}

/// Bordered field input: small kit size, then a little extra padding so the
/// caret is not hard against the edge. Focus paint is the darker accent
/// border from the theme (`focus_ring = false`), not an outer halo.
fn padded_input(state: &Entity<InputState>) -> Input {
    Input::new(state).small().px(px(10.)).py(px(5.))
}

/// Keep a usable center region, shrinking both sidebars on small windows
/// without overwriting the user's preferred widths.
fn sidebar_target_widths(
    viewport: Pixels,
    left: Option<Option<f64>>,
    right: Option<Option<f64>>,
) -> (Pixels, Pixels) {
    fn width(value: Option<Option<f64>>, default: f32, min: f32, max: f32) -> f32 {
        match value {
            None => 0.,
            Some(value) => value
                .filter(|v| v.is_finite())
                .map(|v| v as f32)
                .unwrap_or(default)
                .clamp(min, max),
        }
    }
    let left = width(left, 180., 140., 420.);
    let right = width(right, f32::from(PANEL_WIDTH), 240., 640.);
    // Outer padding and two gutters occupy 32px; reserve 240px for the grid.
    let available = (f32::from(viewport) - 272.).max(0.);
    let scale = if left + right > available {
        available / (left + right)
    } else {
        1.
    };
    (px(left * scale), px(right * scale))
}

// Sidebar navigation is neutral; the table keeps its blue active-row tint.
fn sidebar_selected_background(theme: &ThemeColor) -> Hsla {
    theme.accent
}

/// Warm gold for favorite stars (outline and fill). Dark mode lifts the
/// same hue so the glyph stays legible on the darker island surface.
fn favorite_star_color(dark: bool) -> Hsla {
    rgb(if dark { 0xf5c542 } else { 0xd4a017 }).into()
}

fn toggle_theme(cx: &mut App) {
    let mode = if cx.theme().is_dark() {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    crate::theme::apply(mode, cx);
}

fn sidebar_kind_label(kind: ResourceKind) -> String {
    match kind {
        ResourceKind::JournalArticle => "Articles".into(),
        ResourceKind::Thesis => "Theses".into(),
        _ => format!("{}s", kind_label(kind)),
    }
}

const LIGHT_GRADIENT: (u32, u32) = (0xeae8e2, 0xd9dde5);
const DARK_GRADIENT: (u32, u32) = (0x292722, 0x262c35);
const ISLAND_RADIUS: Pixels = px(16.);

fn mix_rgb(from: Rgba, to: Rgba, amount: f32) -> Rgba {
    Rgba {
        r: from.r + (to.r - from.r) * amount,
        g: from.g + (to.g - from.g) * amount,
        b: from.b + (to.b - from.b) * amount,
        a: 1.,
    }
}

/// The far edge borrows the active theme's accent, not a second hard-coded blue.
/// Share the stops with the corner mask so its cutouts match the backdrop.
fn workspace_gradient_stops(dark: bool, accent: Hsla) -> (Rgba, Rgba) {
    let (start, end) = if dark { DARK_GRADIENT } else { LIGHT_GRADIENT };
    let amount = if dark { 0.16 } else { 0.12 };
    (rgb(start), mix_rgb(rgb(end), accent.to_rgb(), amount))
}

/// GPUI's overflow clip is rectangular. Paint just the four regions outside
/// the island's round outline over its square-edged children, without insetting
/// the table or inspector. This canvas has no hitbox and cannot block clicks.
fn island_corner_mask(stops: (Rgba, Rgba)) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let (start, end) = stops;
            let radius = f32::from(ISLAND_RADIUS);
            let x0 = f32::from(bounds.origin.x);
            let x1 = x0 + f32::from(bounds.size.width);
            let y0 = f32::from(bounds.origin.y);
            let y1 = y0 + f32::from(bounds.size.height);
            let width = f32::from(window.viewport_size().width).max(1.);
            for right in [false, true] {
                let x = if right { x1 } else { x0 };
                let t = (x / width).clamp(0., 1.);
                let color = Hsla::from(mix_rgb(start, end, t));
                for bottom in [false, true] {
                    let y = if bottom { y1 } else { y0 };
                    let sx = if right { -1. } else { 1. };
                    let sy = if bottom { -1. } else { 1. };
                    let at = |u: f32, v: f32| point(px(x + sx * u), px(y + sy * v));
                    let mut path = PathBuilder::fill();
                    path.move_to(at(0., 0.));
                    path.line_to(at(radius, 0.));
                    let control = radius * (1. - 0.552_284_8);
                    path.cubic_bezier_to(at(0., radius), at(control, 0.), at(0., control));
                    path.close();
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                }
            }
        },
    )
    .absolute()
    .inset_0()
    .size_full()
}
impl Render for LibraryApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let delegate = self.table.read(cx).delegate();
        let error = delegate.error().map(str::to_owned);
        let filters_visible = delegate.filters_visible;
        let filter_active = delegate.filter.is_active();
        let search_input = delegate
            .filter_inputs
            .as_ref()
            .map(|inputs| inputs.title.clone());
        let danger = cx.theme().danger;
        let settings = &delegate.settings;
        let (left_target, right_target) = sidebar_target_widths(
            window.viewport_size().width,
            self.left_panel_open.then_some(settings.left_sidebar_width),
            self.detail_open.then_some(settings.right_sidebar_width),
        );
        let motion = cx.theme().motion_tokens();
        // Dragging follows the pointer without easing; toggles still animate.
        let duration = if self.resizing_sidebar.is_some() {
            Duration::ZERO
        } else {
            motion.duration_normal
        };
        let panel_transition = MotionTransition::new(duration).easing(motion.easing_move.clone());
        let sidebar_width = transition(
            "detail-sidebar-width",
            right_target,
            panel_transition.clone(),
            window,
            cx,
        );
        let left_panel_width = transition(
            "left-panel-width",
            left_target,
            panel_transition,
            window,
            cx,
        );

        // The accent caret stays visible on the pale selection tint.
        let editing = self.has_inline(cx);

        let stops = workspace_gradient_stops(cx.theme().is_dark(), cx.theme().primary);
        let background = linear_gradient(
            90.,
            linear_color_stop(Hsla::from(stops.0), 0.),
            linear_color_stop(Hsla::from(stops.1), 1.),
        );

        v_flex()
            .size_full()
            .bg(background.clone())
            .text_color(cx.theme().foreground)
            .on_drag_move(cx.listener(Self::resize_sidebar))
            .capture_any_mouse_up(cx.listener(|this, event: &MouseUpEvent, _, cx| {
                if event.button == MouseButton::Left {
                    this.finish_sidebar_resize(cx);
                }
            }))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_sidebar_resize(cx)),
            )
            // Handlers sit on this ancestor so bindings fire for any focused
            // descendant (table, toolbar, sidebar), not only the DataTable.
            // `InlineEdit` tags the tree so Tab bindings can target this mode.
            .key_context(if editing {
                "LibraryApp InlineEdit"
            } else {
                "LibraryApp"
            })
            .on_action(cx.listener(|this, _: &ToggleDetailPanel, _window, cx| {
                this.toggle_detail_panel(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleLeftPanel, _window, cx| {
                this.toggle_left_panel(cx);
            }))
            .on_action(cx.listener(|this, _: &CycleColumnSort, window, cx| {
                this.cycle_column_sort(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleEditMode, window, cx| {
                this.toggle_edit_mode(window, cx);
            }))
            .on_action(cx.listener(|this, _: &BeginInlineEdit, window, cx| {
                this.toggle_inline_edit(window, cx);
            }))
            .on_action(cx.listener(|this, _: &BeginNewRow, window, cx| {
                this.begin_new_row(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusColumnFilter, window, cx| {
                this.focus_selected_column_filter(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.focus_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &InlineNextField, window, cx| {
                if this.has_inline(cx) {
                    this.focus_inline_delta(1, window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &InlinePrevField, window, cx| {
                if this.has_inline(cx) {
                    this.focus_inline_delta(-1, window, cx);
                } else {
                    cx.propagate();
                }
            }))
            // Single-line inputs bind ⌘↩ to Enter{secondary} and propagate;
            // catch that here while sidebar or inline edit is open.
            .on_action(cx.listener(|this, enter: &Enter, window, cx| {
                if enter.secondary && this.has_inline(cx) {
                    this.save_inline(window, cx);
                } else if enter.secondary && this.edit.is_some() {
                    this.save_edit(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .child(
                // The toolbar doubles as the window's title bar; reserve space
                // for the native traffic lights without adding a bottom rule.
                TitleBar::new()
                    .h(px(40.))
                    .pl_0()
                    .border_0()
                    .bg(background)
                    .child(
                        h_flex()
                            .w_full()
                            .h_full()
                            .items_center()
                            .gap_2()
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    // Equal flex side regions keep search centered despite asymmetric controls.
                                    // Keep edge insets on the controls, not these regions,
                                    // so padding cannot offset the search from window center.
                                    .items_center()
                                    .child(
                                        Button::new("toggle-left-panel")
                                            .ghost()
                                            .ml(if cfg!(target_os = "macos") {
                                                px(80.)
                                            } else {
                                                px(12.)
                                            })
                                            .icon(if self.left_panel_open {
                                                Icon::new(AssetIconName::PanelLeftClose)
                                                    .with_size(px(12.))
                                            } else {
                                                Icon::new(AssetIconName::PanelLeftOpen)
                                                    .with_size(px(12.))
                                            })
                                            .tooltip(if self.left_panel_open {
                                                "Hide left panel ([)"
                                            } else {
                                                "Show left panel ([)"
                                            })
                                            .on_click(cx.listener(|this, _, _window, cx| {
                                                this.toggle_left_panel(cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("toggle-filters")
                                            .ghost()
                                            .icon(
                                                symbol(
                                                    "line.3.horizontal.decrease",
                                                    IconName::Settings2,
                                                )
                                                .with_size(px(12.)),
                                            )
                                            .tooltip("Toggle filters")
                                            .selected(filters_visible)
                                            .when(filter_active, |button| {
                                                button.text_color(cx.theme().primary)
                                            })
                                            .on_click(cx.listener(|this, _, _window, cx| {
                                                this.toggle_filters(cx)
                                            })),
                                    ),
                            )
                            .when_some(search_input.as_ref(), |bar, input| {
                                bar.child(
                                    div()
                                        .id("toolbar-search")
                                        .w(px(440.))
                                        .max_w(relative(0.5))
                                        // Selecting text must not start dragging the window.
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .on_click(|_, _, cx| cx.stop_propagation())
                                        .child(
                                            Input::new(input)
                                                // Input::h only affects multiline editors; constrain
                                                // the single-line frame through its style instead.
                                                .min_h(px(30.))
                                                .rounded(px(8.))
                                                .bg(cx.theme().muted)
                                                .border_color(cx.theme().border)
                                                .px(px(12.))
                                                .py(px(6.))
                                                .text_size(px(12.))
                                                .prefix(
                                                    Icon::new(IconName::Search)
                                                        .with_size(px(13.))
                                                        .text_color(cx.theme().muted_foreground),
                                                )
                                                .suffix(
                                                    div()
                                                        .pr_1()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child("/"),
                                                )
                                                .cleanable(true)
                                                .w_full(),
                                        ),
                                )
                            })
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .items_center()
                                    .justify_end()
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .mr(px(12.))
                                            .gap_1()
                                            .pl_2()
                                            .rounded(px(8.))
                                            .border_1()
                                            .border_color(cx.theme().border)
                                            .bg(cx.theme().background)
                                            .id("toolbar-theme")
                                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                cx.stop_propagation()
                                            })
                                            .on_click(
                                                cx.listener(|_, _, _window, cx| toggle_theme(cx)),
                                            )
                                            .child(
                                                div()
                                                    .size(px(10.))
                                                    .rounded_full()
                                                    .bg(cx.theme().primary),
                                            )
                                            .child(
                                                Button::new("toggle-theme")
                                                    .ghost()
                                                    .icon(if cx.theme().is_dark() {
                                                        symbol("sun.max", IconName::Sun)
                                                            .with_size(px(12.))
                                                    } else {
                                                        symbol("moon", IconName::Moon)
                                                            .with_size(px(12.))
                                                    })
                                                    .tooltip(if cx.theme().is_dark() {
                                                        "Switch to light theme"
                                                    } else {
                                                        "Switch to dark theme"
                                                    })
                                                    .on_click(cx.listener(|_, _, _window, cx| {
                                                        cx.stop_propagation();
                                                        toggle_theme(cx);
                                                    })),
                                            ),
                                    ),
                            ),
                    ),
            )
            .child(
                // Each pane owns its surface; the row's gutters reveal the
                // window gradient without changing the table or inspector.
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .items_stretch()
                    .gap_2()
                    .p_2()
                    .when(left_panel_width > px(0.5), |el| {
                        el.child(
                            div()
                                .id("left-panel-clip")
                                .relative()
                                .w(left_panel_width)
                                .flex_shrink_0()
                                .h_full()
                                .overflow_hidden()
                                .rounded(ISLAND_RADIUS)
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().background)
                                .child(self.render_left_panel(cx))
                                .when(self.left_panel_open, |el| {
                                    el.child(self.sidebar_resize_handle(true, left_panel_width, cx))
                                }),
                        )
                    })
                    .child(
                        // `h_flex` centers its children on the cross axis
                        // instead of stretching them, so the wrapper must
                        // claim the row's height itself: without `h_full`
                        // it falls back to content height, every
                        // percentage height below it (the table, its
                        // scroll body, the rows list) collapses, and the
                        // header-only strip floats mid-window with no
                        // rows and no empty state.
                        {
                            let table = DataTable::new(&self.table)
                                .with_size(Size::XSmall)
                                .bordered(false)
                                .stripe(false)
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
                                });
                            let wrap = div()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .relative()
                                .rounded(ISLAND_RADIUS)
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().background)
                                .overflow_hidden();
                            // While inline editing, keep Root Tab/TabPrev inside
                            // the grid so focus cannot walk to toolbar/sidebar.
                            if editing {
                                wrap.track_focus(&self.inline_trap)
                                    .focus_trap("inline-edit-trap", &self.inline_trap)
                                    .child(table)
                                    .child(island_corner_mask(stops))
                                    .into_any_element()
                            } else {
                                wrap.child(table)
                                    .child(island_corner_mask(stops))
                                    .into_any_element()
                            }
                        },
                    )
                    // Clip the panel during the width animation; keep it mounted
                    // animates; keep it mounted until the close finishes.
                    .when(sidebar_width > px(0.5), |el| {
                        el.child(
                            div()
                                .id("detail-sidebar-clip")
                                .relative()
                                .h_full()
                                .w(sidebar_width)
                                .overflow_hidden()
                                .rounded(ISLAND_RADIUS)
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().background)
                                .flex_shrink_0()
                                .child(self.render_side_panel(cx))
                                .child(island_corner_mask(stops))
                                .when(self.detail_open, |el| {
                                    el.child(self.sidebar_resize_handle(false, sidebar_width, cx))
                                }),
                        )
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
mod panel_width_tests {
    use super::sidebar_target_widths;
    use gpui_kit::px;

    #[gpui_kit::test]
    fn dragging_sidebars_saves_widths(cx: &mut gpui_kit::TestAppContext) {
        use super::{LibraryApp, LibraryDelegate, Settings};
        use crate::db::DbWorker;
        use gpui_kit::test::TestWindowExt as _;
        use gpui_kit::{AppContext as _, point, size};

        let dir = tempfile::tempdir().unwrap();
        // The production DbWorker completes requests on its own OS thread.
        cx.executor().allow_parking();
        let worker = DbWorker::spawn(
            oeuvre_app::AppState::builder(oeuvre_app::testing::MemoryStore::new())
                .surface(oeuvre_app::Surface::Test)
                .build(),
        );
        let first = worker.list_page_blocking(None, 50).unwrap();
        let (handle, _) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(
                gpui_kit::WindowOptions {
                    window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                        origin: Default::default(),
                        size: size(px(1100.), px(700.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let delegate = LibraryDelegate::new(
                        worker,
                        first,
                        Settings {
                            left_sidebar_width: Some(200.),
                            right_sidebar_width: Some(360.),
                            ..Settings::default()
                        },
                    );
                    let table = cx.new(|cx| {
                        gpui_kit::component::table::TableState::new(delegate, window, cx)
                    });
                    cx.new(|cx| LibraryApp::new(table, dir.path().to_path_buf(), window, cx))
                },
            )
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // Both handles exist on the first frame, including the default-open inspector.
            let left = window.find("resize-left-sidebar").bounds().center();
            let right = window.find("resize-right-sidebar").bounds().center();
            assert!((left.x - px(208.)).abs() < px(8.));
            assert!((right.x - px(732.)).abs() < px(8.));
            window.drag(left, point(left.x + px(50.), left.y), cx);
            let settings = Settings::load(dir.path());
            assert!((230. ..=250.).contains(&settings.left_sidebar_width.unwrap()));
            assert_eq!(settings.right_sidebar_width, Some(360.));
            let right = window.find("resize-right-sidebar").bounds().center();
            window.drag(right, point(right.x - px(60.), right.y), cx);
            let settings = Settings::load(dir.path());
            assert!(
                (400. ..=420.).contains(&settings.right_sidebar_width.unwrap()),
                "{settings:?}; handle={right:?}"
            );
            assert!((230. ..=250.).contains(&settings.left_sidebar_width.unwrap()));
        })
        .unwrap();
    }

    #[test]
    fn default_widths_and_closed_panels() {
        assert_eq!(
            sidebar_target_widths(px(1000.), Some(None), Some(None)),
            (px(180.), px(320.))
        );
        assert_eq!(
            sidebar_target_widths(px(1000.), None, None),
            (px(0.), px(0.))
        );
    }

    #[test]
    fn restored_widths_are_bounded_and_invalid_values_use_defaults() {
        assert_eq!(
            sidebar_target_widths(px(2000.), Some(Some(230.)), Some(Some(410.))),
            (px(230.), px(410.))
        );
        assert_eq!(
            sidebar_target_widths(px(2000.), Some(Some(-10.)), Some(Some(900.))),
            (px(140.), px(640.))
        );
        assert_eq!(
            sidebar_target_widths(px(2000.), Some(Some(f64::NAN)), Some(Some(f64::INFINITY))),
            (px(180.), px(320.))
        );
    }

    #[test]
    fn narrow_windows_keep_the_grid_usable() {
        assert_eq!(
            sidebar_target_widths(px(522.), Some(None), Some(None)),
            (px(90.), px(160.))
        );
        assert_eq!(
            sidebar_target_widths(px(240.), Some(None), Some(None)),
            (px(0.), px(0.))
        );
    }
}

#[cfg(test)]
mod sidebar_selection_tests {
    use super::sidebar_selected_background;
    use gpui_kit::component::ThemeColor;
    use gpui_kit::{Hsla, rgb};

    #[test]
    fn selected_navigation_uses_neutral_surface_not_blue_list_selection() {
        let neutral: Hsla = rgb(0xe7e7e7).into();
        let blue: Hsla = rgb(0x0064ff).into();
        let theme = ThemeColor {
            accent: neutral,
            list_active: blue,
            ..ThemeColor::default()
        };
        assert_eq!(sidebar_selected_background(&theme), neutral);
        assert_ne!(sidebar_selected_background(&theme), theme.list_active);
    }
}

#[cfg(test)]
mod gradient_tests {
    use super::workspace_gradient_stops;
    use gpui_kit::{Hsla, rgb};

    #[test]
    fn workspace_gradient_follows_the_accent_in_both_themes() {
        for dark in [false, true] {
            let (start, blue_end) = workspace_gradient_stops(dark, Hsla::from(rgb(0x0064ff)));
            let (_, red_end) = workspace_gradient_stops(dark, Hsla::from(rgb(0xff6400)));
            assert_eq!(start, rgb(if dark { 0x292722 } else { 0xeae8e2 }));
            assert!(blue_end.b > red_end.b);
            assert!(red_end.r > blue_end.r);
        }
    }
}
#[cfg(test)]
mod column_width_tests {
    // No `use super::*`: the glob would re-import gpui's `test` proc
    // macro (via the module's `gpui_kit::*`), hijacking `#[test]`.
    use super::{
        COL_CREATORS, COL_PUBLISHED, COL_PUBLISHER, COL_TITLE, COLUMN_KEYS, COLUMNS,
        DEFAULT_TITLE_WIDTH, stored_width, widths_by_key,
    };
    use gpui_kit::px;
    use std::collections::BTreeMap;

    #[test]
    fn widths_by_key_keys_each_column_by_its_config_key() {
        let map = widths_by_key(&[
            px(28.),
            px(310.),
            px(260.),
            px(140.),
            px(160.),
            px(110.),
            px(110.),
        ]);
        assert_eq!(map.len(), COLUMNS);
        assert_eq!(map[COLUMN_KEYS[COL_TITLE]], 310.);
        assert_eq!(map[COLUMN_KEYS[COL_PUBLISHED]], 140.);
        assert_eq!(map[COLUMN_KEYS[COL_PUBLISHER]], 160.);
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
        COL_CREATORS, COL_KIND, COL_PUBLISHED, COL_TITLE, FilterField, cell_on_accent,
        cell_text_colors, creator_options, filter_field_for_col, header_row_height, next_sort,
        sort_field_for_col, title_predicate, year_instant, year_range,
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
        assert!(
            options
                .iter()
                .all(|(_, id)| id.as_ulid().timestamp_ms() > 0)
        );
    }

    #[test]
    fn header_columns_map_to_filter_fields() {
        assert_eq!(filter_field_for_col(COL_KIND), Some(FilterField::Kind));
        assert_eq!(filter_field_for_col(COL_TITLE), Some(FilterField::Title));
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
    fn accent_tint_keeps_dark_text_readable() {
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
        assert_eq!(primary, colors.foreground);
        assert_eq!(
            (secondary.h, secondary.s, secondary.l),
            (
                colors.foreground.h,
                colors.foreground.s,
                colors.foreground.l
            )
        );
    }

    #[test]
    fn cell_accent_follows_selected_row_or_column_not_both_axes() {
        // Row highlight: every column of that row is on the accent.
        assert!(cell_on_accent(Some(2), None, 2, 0));
        assert!(cell_on_accent(Some(2), None, 2, 4));
        assert!(!cell_on_accent(Some(2), None, 1, 0));
        // Column highlight: every row of that column is on the accent.
        assert!(cell_on_accent(None, Some(3), 0, 3));
        assert!(cell_on_accent(None, Some(3), 9, 3));
        assert!(!cell_on_accent(None, Some(3), 0, 1));
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
            favorite: false,
            tags: vec![],
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
        assert_eq!(
            cache.get(&a).unwrap().title.as_str(),
            "Citation Networks 000"
        );
        assert_eq!(
            cache.get(&b).unwrap().title.as_str(),
            "Provenance Studies 001"
        );
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
    use oeuvre_core::{NonEmptyStr, ResourceId, ResourceKind, ResourceSummary, UtcMillis};

    fn summary(title: &str) -> ResourceSummary {
        ResourceSummary {
            id: ResourceId::generate(),
            kind: ResourceKind::Book,
            title: NonEmptyStr::try_new(title).expect("title non-empty"),
            creators_label: String::new(),
            published_label: None,
            publisher: None,
            container_title: None,
            date_added: UtcMillis::from_unix_ms(0),
            date_edited: UtcMillis::from_unix_ms(0),
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
        let labels: Vec<String> = options
            .iter()
            .map(ParentItem::title)
            .map(String::from)
            .collect();
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
        let labels: Vec<String> = options
            .iter()
            .map(ParentItem::title)
            .map(String::from)
            .collect();
        assert_eq!(labels, ["Zzz Journal"]);
    }
}
