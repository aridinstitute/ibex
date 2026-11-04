//! Virtualized bibliographic tree grid with independently paged sibling lists.
//!
//! Title disclosure buttons and Alt+Right/Alt+Left expand/collapse resources;
//! Expansion loads all direct children; Alt+Right also retries a failed expansion.
//! Filtering retains matching resources' ancestors as navigation context.

#[path = "resource_grid.rs"]
mod resource_grid;
use resource_grid::ResourceGrid;
#[path = "series_picker.rs"]
mod series_picker;
use series_picker::{SeriesChoice, SeriesDelegate};

use std::{
    any::Any,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    ops::Range,
    path::PathBuf,
    time::{Duration, Instant},
};

use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::base::{Transition as MotionTransition, transition};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::combobox::{Combobox, ComboboxEvent, ComboboxState};
use gpui_kit::component::input::{Enter, Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::resizable::resize_handle;
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::table::{Column, DataTable, TableDelegate, TableEvent, TableState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, FocusTrapElement as _, FocusableExt as _, Icon, IconName, IndexPath,
    Selectable as _, Sizable as _, Size, ThemeColor, ThemeMode, TitleBar, WindowExt as _, h_flex,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{Focusable as _, *};

use ibex_core::{
    Collection, CollectionId, CollectionTreeNode, EntryChoice, EntryProposal, ImportChoices,
    NonEmptyStr, ParentRule, Person, PersonChoice as ImportPersonChoice, PersonId,
    PersonMatchPreview, PublishedRange, Resource, ResourceCounts, ResourceDetail, ResourceFilter,
    ResourceGridPage, ResourceGridParent, ResourceId, ResourceKind, ResourceQuery, ResourceSort,
    ResourceSummary, SortDirection, Tag, TagId, UtcMillis, format_recorded_date,
    keyword_suggestions,
};

use crate::classification_rules::{ClassificationRules, ClassificationRulesSaved};
use crate::collections::{CollectionChoice, CollectionDelegate, flatten_collections};
use crate::command_palette::{
    ClassifyWithJevRequest, CollectionClassificationRequest, CommandPalette, IdentifierRequest,
    JevSettingRequest, OpenClassificationRules, PaletteRequest, SetThemeColor,
    ToggleResourceGrouping,
};
use crate::db::DbWorker;
use crate::detail::{
    DetailFetch, DetailView, FIELD_LABEL_WIDTH, FIELD_TEXT_SIZE, PANEL_WIDTH, render_panel,
};
use crate::documentation::{DocumentationWindow, OpenDocumentation};
use crate::edit::{
    BibliographicField, EditDraft, KindItem, build_create, build_update, kind_items, prefill_draft,
};
use crate::icons::symbol;
use crate::import_dialog::{ConflictPolicy, ImportConfirmed, ImportDialog};
use crate::inline_edit::{
    InlineField, InlineRowEdit, InlineSession, PersonChoice, PersonDelegate,
    creators_badge_trigger, person_name_from_query, selected_person_ids,
};
use crate::kind::{kind_color, kind_label, kind_symbol};
use crate::settings::{Settings, ThemeColorTarget};
use crate::tags::{TagChoice, TagDelegate};
use crate::theme_editor::{ThemeEditor, ThemeEditorEvent};
use gpui_kit::component::Disableable as _;
use gpui_kit::component::popover::Popover;
use ibex_app::command::ImportReport;

/// Rows fetched per scroll trigger. Small enough to feel incremental, large
/// enough that the demo library spans several pages.
pub const PAGE_SIZE: u32 = 50;

/// Skip entries that still need review; create new people for ambiguous names.
fn safe_skip_choices(preview: &ibex_core::ImportPreview) -> ImportChoices {
    let mut choices = ImportChoices::default();
    for entry in &preview.entries {
        if !entry.needs_review() {
            continue;
        }
        choices
            .entries
            .insert(entry.entry_key.clone(), EntryChoice::Skip);
        for person in &entry.people {
            if matches!(person.match_, PersonMatchPreview::Ambiguous { .. }) {
                choices.people.insert(
                    ImportChoices::person_key(&entry.entry_key, person.index),
                    ImportPersonChoice::CreateNew,
                );
            }
        }
    }
    choices
}

/// Import publication duplicates explicitly; create new people when ambiguous.
fn intentional_duplicate_choices(preview: &ibex_core::ImportPreview) -> ImportChoices {
    let mut choices = ImportChoices::default();
    for entry in &preview.entries {
        if matches!(entry.proposal, EntryProposal::PublicationDuplicate { .. }) {
            choices
                .entries
                .insert(entry.entry_key.clone(), EntryChoice::IntentionalDuplicate);
        } else if entry.needs_review()
            && !matches!(entry.proposal, EntryProposal::AlreadyCommitted { .. })
        {
            // Container / other conflicts: skip rather than invent merges.
            if entry
                .conflicts
                .iter()
                .any(|c| c.code != ibex_core::import_codes::AMBIGUOUS_CREATOR)
            {
                choices
                    .entries
                    .insert(entry.entry_key.clone(), EntryChoice::Skip);
            }
        }
        for person in &entry.people {
            if matches!(person.match_, PersonMatchPreview::Ambiguous { .. }) {
                choices.people.insert(
                    ImportChoices::person_key(&entry.entry_key, person.index),
                    ImportPersonChoice::CreateNew,
                );
            }
        }
    }
    choices
}

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
/// A small rounded label used for collection memberships and suggestions.
fn membership_chip(label: String, cx: &App) -> Div {
    h_flex()
        .items_center()
        .gap_0p5()
        .pl_1p5()
        .pr_0p5()
        .min_h(px(20.))
        .rounded(px(6.))
        .border_1()
        .border_color(cx.theme().border)
        .child(div().pr_1().child(label))
}

fn compact_menu_item(
    label: impl Into<SharedString>,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> PopupMenuItem {
    let label = label.into();
    PopupMenuItem::element(move |_, _| div().text_size(rems(1.)).child(label.clone()))
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
            .text_size(rems(1.))
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

const CELL_TEXT_SIZE: Rems = rems(1.);
/// Inline editors read slightly larger than static cells so the caret and
/// typed text stay obvious on the accent row.
const INLINE_CELL_TEXT_SIZE: Rems = rems(1.1);
/// Kind dropdown is wider than the 28px icon column so labels aren't clipped.
const INLINE_KIND_MENU_WIDTH: Pixels = px(220.);
/// Header labels are one step below the cell text; the kit's default
/// `text_xs` reads too small at this base font size.
const HEADER_TEXT_SIZE: Rems = rems(0.9);

pub struct LibraryDelegate {
    worker: DbWorker,
    rows: Vec<ResourceSummary>,
    grid: ResourceGrid,
    grouped: bool,
    reopen: HashSet<ResourceId>,
    reveal_selection: Option<ResourceId>,
    loading: bool,
    reloading: bool,
    next: Option<u64>,
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
    selected_ids: HashSet<ResourceId>,
    selection_anchor: Option<ResourceId>,
    preserve_selection: bool,
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
    fn row_selected(&self, row_ix: usize) -> bool {
        self.row_id(row_ix)
            .is_some_and(|id| self.selected_ids.contains(&id))
            || (self.selected_ids.is_empty() && self.selected_row == Some(row_ix))
    }
    /// Reconcile editors by identity before publishing a new positional snapshot.
    fn rebuild_rows(&mut self) {
        self.rows = self.grid.visible();
        let draft = self.inline.as_ref().is_some_and(InlineSession::is_create) as usize;
        if let Some(session) = self.inline.as_mut() {
            match session {
                InlineSession::Loading { id, row_ix } => {
                    if let Some(ix) = self.rows.iter().position(|row| row.id == *id) {
                        *row_ix = ix + draft;
                    }
                }
                InlineSession::Ready(form) => {
                    if let Some(ix) = form
                        .record
                        .as_ref()
                        .and_then(|record| self.rows.iter().position(|row| row.id == record.id))
                    {
                        form.row_ix = ix + draft;
                    }
                }
                InlineSession::Create(_) => {}
            }
        }
    }

    fn request_children(
        &mut self,
        id: ResourceId,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        if self.reloading || !self.grouped {
            return;
        }
        let Some((token, offset)) = self.grid.begin(id) else {
            return;
        };
        let worker = self.worker.clone();
        let query = ResourceQuery {
            sort: self.sort,
            filter: self.filter.clone(),
        };
        let generation = self.listing_gen;
        cx.spawn_in(window, async move |table, window| {
            let reply = worker
                .list_grid_page(ResourceGridParent::Children(id), query, offset, u32::MAX)
                .await;
            _ = table.update_in(window, |table, window, cx| {
                if table.delegate().listing_gen != generation {
                    return;
                }
                let selected = table
                    .selected_row()
                    .and_then(|ix| table.delegate().row_id(ix));
                let delegate = table.delegate_mut();
                if !delegate.grid.finish(id, token, reply) {
                    return;
                }
                delegate.rebuild_rows();
                delegate.resume_expansions(window, cx);
                let selected = selected.or(delegate.reveal_selection);
                restore_grid_selection(table, selected, None, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn resume_expansions(&mut self, window: &mut Window, cx: &mut Context<TableState<Self>>) {
        if !self.grouped {
            return;
        }
        let ids: Vec<_> = self
            .grid
            .positions
            .iter()
            .filter_map(|(id, (_, _, has_children))| {
                (*has_children && self.reopen.contains(id)).then_some(*id)
            })
            .collect();
        for id in ids {
            self.grid.expanded.insert(id);
            let needs_page = self
                .grid
                .children
                .get(&id)
                .is_none_or(|page| !page.loaded && page.error.is_none());
            if needs_page {
                self.request_children(id, window, cx);
            }
        }
    }
    /// Start from a first page fetched before the window opens, so the grid
    /// never renders an empty bootstrap state.
    pub fn new(worker: DbWorker, first: ResourceGridPage, settings: Settings) -> Self {
        let next = first.next_offset;
        let mut grid = ResourceGrid::new(first);
        let rows = grid.visible();
        Self {
            worker,
            rows,
            grid,
            grouped: true,
            reopen: HashSet::new(),
            reveal_selection: None,
            loading: false,
            reloading: false,
            next,
            sort: None,
            filter: ResourceFilter::none(),
            filter_inputs: None,
            filters_visible: false,
            error: None,
            selected_row: None,
            selected_ids: HashSet::new(),
            selection_anchor: None,
            preserve_selection: false,
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
        self.selected_ids.clear();
        self.selection_anchor = None;
        self.reopen.extend(self.grid.expanded.iter().copied());
        self.reveal_selection = selected_id.filter(|_| restore_col.is_none());
        for page in self.grid.children.values_mut() {
            page.request = None;
        }
        self.loading = true;
        self.reloading = true;
        self.error = None;
        // A fresh listing invalidates in-row editors (row indices move).
        self.inline = None;
        let worker = self.worker.clone();
        let sort = self.sort;
        let filter = self.filter.clone();
        let issued_gen = self.listing_gen;
        let grouped = self.grouped;
        cx.spawn_in(window, async move |table, window| {
            let reply = listing_page(&worker, grouped, ResourceQuery { sort, filter }, 0).await;
            _ = table.update_in(window, |table, window, cx| {
                let delegate = table.delegate_mut();
                // A newer sort or keystroke superseded this fetch; its
                // rows belong to a listing nobody is looking at.
                if delegate.listing_gen != issued_gen {
                    return;
                }
                delegate.loading = false;
                delegate.reloading = false;
                match reply {
                    Ok(page) => {
                        delegate.next = page.next_offset;
                        delegate.grid = ResourceGrid::new(page);
                        delegate.rebuild_rows();
                        delegate.resume_expansions(window, cx);
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
                    restore_grid_selection(table, selected_id, None, cx);
                }
                table.col_selectable = true;
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
        let data_ix = self.data_row_ix(row_ix)?;
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

/// Flat mode uses the existing query contract so sorting/filtering span all resources.
async fn listing_page(
    worker: &DbWorker,
    grouped: bool,
    query: ResourceQuery,
    offset: u64,
) -> Result<ResourceGridPage, String> {
    if grouped {
        worker
            .list_grid_page(ResourceGridParent::Roots, query, offset, PAGE_SIZE)
            .await
    } else {
        worker
            .list_query_page(query, offset, PAGE_SIZE)
            .await
            .map(|page| ResourceGridPage {
                rows: page
                    .rows
                    .into_iter()
                    .map(|summary| ibex_core::ResourceGridRow {
                        summary,
                        has_children: false,
                    })
                    .collect(),
                next_offset: page.next_offset,
            })
    }
}

/// Selection is reconciled in domain coordinates before another event can use it.
fn restore_grid_selection(
    table: &mut TableState<LibraryDelegate>,
    selected: Option<ResourceId>,
    fallback: Option<ResourceId>,
    cx: &mut Context<TableState<LibraryDelegate>>,
) {
    let Some(id) = selected else {
        return;
    };
    let delegate = table.delegate_mut();
    let draft = delegate
        .inline
        .as_ref()
        .is_some_and(InlineSession::is_create) as usize;
    let ix = delegate
        .rows
        .iter()
        .position(|row| row.id == id)
        .or_else(|| fallback.and_then(|id| delegate.rows.iter().position(|row| row.id == id)));
    if let Some(ix) = ix {
        delegate.selected_row = Some(ix + draft);
        // Structural restores move keyboard focus, not the user's selection.
        if !delegate.selected_ids.is_empty() {
            let visible: HashSet<_> = delegate.rows.iter().map(|row| row.id).collect();
            delegate.selected_ids.retain(|id| visible.contains(id));
            delegate.selected_ids.insert(delegate.rows[ix].id);
            delegate.preserve_selection = true;
            if delegate
                .selection_anchor
                .is_some_and(|id| !visible.contains(&id))
            {
                delegate.selection_anchor = Some(delegate.rows[ix].id);
            }
        }
        if delegate.reveal_selection == Some(id) {
            delegate.reveal_selection = None;
        }
        table.set_selected_row(ix + draft, cx);
    } else {
        delegate.selected_row = None;
        table.clear_selection(cx);
    }
}

fn change_resource_expansion(
    table: &mut TableState<LibraryDelegate>,
    id: ResourceId,
    expand: bool,
    window: &mut Window,
    cx: &mut Context<TableState<LibraryDelegate>>,
) {
    // Editors retain positional focus. Do not let pointer or keyboard disclosure
    // hide their row; asynchronous page insertion is reconciled by identity.
    if table.delegate().inline.is_some() || table.delegate().reloading {
        return;
    }
    if !table
        .delegate()
        .grid
        .positions
        .get(&id)
        .is_some_and(|(_, _, children)| *children)
    {
        return;
    }
    let selected = table
        .selected_row()
        .and_then(|ix| table.delegate().row_id(ix));
    let delegate = table.delegate_mut();
    if expand {
        delegate.grid.expanded.insert(id);
        delegate.reopen.insert(id);
        let unloaded = delegate
            .grid
            .children
            .get(&id)
            .is_none_or(|page| !page.loaded);
        if unloaded {
            delegate.request_children(id, window, cx);
        }
    } else {
        delegate.grid.collapse(id);
        delegate.reopen = delegate.grid.expanded.clone();
    }
    delegate.rebuild_rows();
    restore_grid_selection(table, selected, Some(id), cx);
    cx.notify();
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
            .id(SharedString::from(
                self.row_id(row_ix)
                    .map(|id| format!("resource-row-{id}"))
                    .unwrap_or_else(|| "resource-create-draft".into()),
            ))
            .border_1()
            .border_color(transparent_white())
            .when(self.row_selected(row_ix), |row| {
                row.bg(cx.theme().table_active)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |table, event: &MouseDownEvent, window, cx| {
                    if table.delegate().inline.is_some()
                        || !(event.modifiers.secondary(cx) || event.modifiers.shift)
                    {
                        return;
                    }
                    let delegate = table.delegate_mut();
                    let Some(id) = delegate.row_id(row_ix) else {
                        return;
                    };
                    if event.modifiers.shift {
                        let anchor = delegate
                            .selection_anchor
                            .and_then(|id| delegate.rows.iter().position(|r| r.id == id))
                            .unwrap_or(row_ix);
                        let range = anchor.min(row_ix)..=anchor.max(row_ix);
                        if !event.modifiers.secondary(cx) {
                            delegate.selected_ids.clear();
                        }
                        delegate
                            .selected_ids
                            .extend(delegate.rows[range].iter().map(|r| r.id));
                    } else {
                        if !delegate.selected_ids.remove(&id) {
                            delegate.selected_ids.insert(id);
                        }
                        // Keep the range origin stable while toggling disjoint rows.
                        delegate.selection_anchor = delegate.selection_anchor.or(Some(id));
                    }
                    let focused = if delegate.selected_ids.contains(&id) {
                        Some(row_ix)
                    } else {
                        delegate
                            .rows
                            .iter()
                            .position(|r| delegate.selected_ids.contains(&r.id))
                    };
                    delegate.preserve_selection = focused.is_some();
                    if let Some(ix) = focused {
                        table.set_selected_row(ix, cx);
                    } else {
                        table.clear_selection(cx);
                    }
                    table.focus_handle(cx).focus(window, cx);
                    cx.notify();
                }),
            )
            .when(self.row_selected(row_ix), |row| {
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
            let on_accent = cell_on_accent(
                self.row_selected(row_ix).then_some(row_ix),
                self.selected_col,
                row_ix,
                col_ix,
            );
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
        let on_accent = cell_on_accent(
            self.row_selected(row_ix).then_some(row_ix),
            self.selected_col,
            row_ix,
            col_ix,
        );
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
            COL_TITLE => {
                let id = row.id;
                let (depth, _, has_children) = self
                    .grid
                    .positions
                    .get(&id)
                    .copied()
                    .unwrap_or((0, None, false));
                let expanded = self.grid.expanded.contains(&id);
                let disabled = self.inline.is_some() || self.reloading;
                let title = row.title.as_str().to_owned();
                let mut cell = cell.pl(rems(depth as f32)).gap_1();
                if has_children {
                    let label = format!(
                        "{} {} ({})",
                        if expanded { "Collapse" } else { "Expand" },
                        title,
                        if expanded { "Alt+Left" } else { "Alt+Right" }
                    );
                    cell = cell.child(
                        Button::new(format!("resource-expand-{id}"))
                            .ghost()
                            .xsmall()
                            .compact()
                            .icon(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .accessibility_label(label.clone())
                            .tooltip(label)
                            .disabled(disabled)
                            .on_click(cx.listener(move |table, _, window, cx| {
                                cx.stop_propagation();
                                change_resource_expansion(table, id, !expanded, window, cx);
                            })),
                    );
                } else {
                    cell = cell.child(div().w_4().flex_shrink_0());
                }
                cell = cell.child(
                    div()
                        .id(SharedString::from(format!("resource-title-{id}")))
                        .test_support()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .child(title),
                );
                if expanded && let Some(page) = self.grid.children.get(&id) {
                    let loading = page.request.is_some();
                    if loading || page.error.is_some() {
                        let label = if loading {
                            "Loading children…"
                        } else {
                            "Children failed · Retry"
                        };
                        let tooltip = page
                            .error
                            .clone()
                            .unwrap_or_else(|| format!("Load children of {}", row.title.as_str()));
                        cell = cell.child(
                            Button::new(format!("resource-children-{id}"))
                                .ghost()
                                .xsmall()
                                .compact()
                                .label(label)
                                .tooltip(tooltip)
                                .disabled(disabled || loading)
                                .on_click(cx.listener(move |table, _, window, cx| {
                                    cx.stop_propagation();
                                    table.delegate_mut().request_children(id, window, cx);
                                })),
                        );
                    }
                }
                cell
            }
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
        !self.loading && self.error.is_none() && self.next.is_some()
    }

    fn load_more_threshold(&self) -> usize {
        10
    }

    fn load_more(&mut self, window: &mut Window, cx: &mut Context<TableState<Self>>) {
        if self.loading {
            return;
        }
        let Some(offset) = self.next else {
            return;
        };
        self.loading = true;
        self.error = None;
        let worker = self.worker.clone();
        let query = ResourceQuery {
            sort: self.sort,
            filter: self.filter.clone(),
        };
        let issued_gen = self.listing_gen;
        let grouped = self.grouped;
        cx.spawn_in(window, async move |table, window| {
            let reply = listing_page(&worker, grouped, query, offset).await;
            _ = table.update_in(window, |table, window, cx| {
                let selected = table
                    .selected_row()
                    .and_then(|ix| table.delegate().row_id(ix));
                let delegate = table.delegate_mut();
                if delegate.listing_gen != issued_gen {
                    return;
                }
                delegate.loading = false;
                match reply {
                    Ok(page) => {
                        delegate.next = page.next_offset;
                        delegate.grid.append_roots(page);
                        delegate.rebuild_rows();
                        delegate.resume_expansions(window, cx);
                    }
                    Err(err) => delegate.error = Some(err),
                }
                restore_grid_selection(table, selected, None, cx);
                cx.notify();
            });
        })
        .detach();
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
    options.sort_by_key(|a| a.0.to_lowercase());
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
        .filter(|row| row.id != edited && expected.contains(&row.kind))
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
// Takes each editor entity separately so the caller can release its borrow first.
#[allow(clippy::too_many_arguments)]
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
    ImportBiblatexFile,
    OpenCommandPalette,
    SelectAllResources,
    ExtendResourceSelectionUp,
    ExtendResourceSelectionDown,
    ToggleLeftPanel,
    ToggleDetailPanel,
    CycleColumnSort,
    ToggleEditMode,
    BeginInlineEdit,
    BeginNewRow,
    FocusColumnFilter,
    ToggleFavorite,
    ClassifyWithJev,
    FocusSearch,
    InlineNextField,
    InlinePrevField,
    ExpandResource,
    CollapseResource,
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
/// details can inherit fields from other records, so resource edits discard
/// the cache. Otherwise the oldest falls out past [`DETAIL_CACHE_CAPACITY`].
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

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SidebarSection {
    Kinds,
    Tags,
    Collections,
}

impl SidebarSection {
    fn key(self) -> &'static str {
        match self {
            Self::Kinds => "kinds",
            Self::Tags => "tags",
            Self::Collections => "collections",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImportState {
    Idle,
    Choosing,
    Importing,
}
pub struct LibraryApp {
    import_state: ImportState,
    import_gen: u64,
    import_task: Option<Task<()>>,
    table: Entity<TableState<LibraryDelegate>>,
    /// App data directory holding config.toml, where settings persist.
    settings_dir: PathBuf,
    theme_editor: Entity<ThemeEditor>,
    theme_editor_open: bool,
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
    /// Independent disclosure state for the three sidebar sections.
    collapsed_sections: HashSet<SidebarSection>,
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
    attachment_pending: HashSet<ResourceId>,
    attachment_error: Option<(ResourceId, String)>,
    /// Ignore repeated keyboard/button requests until this id's write completes.
    favorite_pending: HashSet<ResourceId>,
    /// Resources with a suggestion-accept write in flight.
    suggestion_pending: HashSet<ResourceId>,
    /// Latest Jev invocation; only the newest generation may land.
    jev: Option<JevState>,
    jev_gen: u64,
    /// Decisions API endpoint; tests point it at a local server.
    jev_endpoint: String,
    /// The classification rules window while it is open.
    rules_window: Option<(AnyWindowHandle, Entity<ClassificationRules>)>,
    /// The edit form while one is open: loading while the raw record is
    /// fetched, ready once the inputs exist. One form at a time, owned by
    /// the row it was opened on.
    edit: Option<EditFetch>,
    edit_generation: u64,
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

enum BibliographicInput {
    Single(Entity<InputState>),
    Multiline(Entity<TextareaState>),
}
impl BibliographicInput {
    fn value(&self, cx: &App) -> String {
        match self {
            Self::Single(input) => input.read(cx).value().to_string(),
            Self::Multiline(input) => input.read(cx).value().to_string(),
        }
    }
    fn render(&self, field: BibliographicField, disabled: bool) -> AnyElement {
        match self {
            Self::Single(input) => Input::new(input)
                .small()
                .aria_label(field.label())
                .disabled(disabled)
                .w_full()
                .into_any_element(),
            Self::Multiline(input) => Textarea::new(input)
                .small()
                .aria_label(field.label())
                .disabled(disabled)
                .w_full()
                .into_any_element(),
        }
    }
}

#[derive(Clone)]
struct CreatorRoleItem(ibex_core::CreatorRole);
impl SearchableListItem for CreatorRoleItem {
    type Value = ibex_core::CreatorRole;
    fn title(&self) -> SharedString {
        format!("{:?}", self.0).into()
    }
    fn value(&self) -> &Self::Value {
        &self.0
    }
}
struct CreatorRow {
    key: u64,
    person_id: PersonId,
    name: String,
    role: Entity<ComboboxState<SearchableVec<CreatorRoleItem>>>,
    credited_as: Entity<InputState>,
}
impl CreatorRow {
    fn new(
        key: u64,
        spec: ibex_core::CreatorSpec,
        name: String,
        window: &mut Window,
        cx: &mut Context<LibraryApp>,
    ) -> Self {
        use ibex_core::CreatorRole::*;
        let roles = vec![Author, Editor, Translator, Contributor, Host, Director];
        let selected = roles.iter().position(|role| *role == spec.role).unwrap();
        Self {
            key,
            person_id: spec.person_id,
            name,
            role: cx.new(|cx| {
                ComboboxState::new(
                    SearchableVec::new(roles.into_iter().map(CreatorRoleItem).collect::<Vec<_>>()),
                    vec![IndexPath::new(selected)],
                    window,
                    cx,
                )
                .searchable(true)
            }),
            credited_as: prefill_input(
                spec.credited_as
                    .map(|s| s.as_str().to_owned())
                    .unwrap_or_default(),
                "Credited as (optional)",
                window,
                cx,
            ),
        }
    }
    fn spec(&self, cx: &App) -> ibex_core::CreatorSpec {
        ibex_core::CreatorSpec {
            person_id: self.person_id,
            role: self
                .role
                .read(cx)
                .selected_values()
                .first()
                .copied()
                .unwrap_or(ibex_core::CreatorRole::Author),
            credited_as: NonEmptyStr::try_new(self.credited_as.read(cx).value().trim()).ok(),
        }
    }
}

/// The edit form's widgets plus the raw record they edit. Inputs are
/// prefilled from the record at construction; saving reads them back into
/// an [`EditDraft`] and builds the full-replace payload. The parent box
/// starts empty — its options need a listing, which is a second fetch —
/// and fills in the same flow that opens the form (see
/// [`LibraryApp::request_edit`]).
struct EditForm {
    record: Box<Resource>,
    favorite: bool,
    title: Entity<InputState>,
    kind: Entity<ComboboxState<SearchableVec<KindItem>>>,
    parent: Entity<ComboboxState<SearchableVec<ParentItem>>>,
    creators: Entity<ComboboxState<PersonDelegate>>,
    creator_rows: Vec<CreatorRow>,
    next_creator_key: u64,
    people_names: HashMap<PersonId, String>,
    publisher: Entity<InputState>,
    fields: BTreeMap<BibliographicField, BibliographicInput>,
    series: Entity<ComboboxState<SeriesDelegate>>,
    series_options: Vec<ibex_core::Series>,
    series_selected: Option<ibex_core::SeriesId>,
    series_loading: bool,
    series_creating: bool,
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
    parent_options_kind: Option<&'static [ResourceKind]>,
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
        let creators = cx.new(|cx| {
            ComboboxState::new(PersonDelegate::new(people), Vec::new(), window, cx).searchable(true)
        });
        let people_names: HashMap<_, _> = people.iter().map(|p| (p.id, p.name.display())).collect();
        let creator_rows = draft
            .creators
            .iter()
            .enumerate()
            .map(|(ix, spec)| {
                CreatorRow::new(
                    ix as u64,
                    spec.clone(),
                    people_names
                        .get(&spec.person_id)
                        .cloned()
                        .unwrap_or_else(|| "Unknown person".into()),
                    window,
                    cx,
                )
            })
            .collect();
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
            creator_rows,
            next_creator_key: draft.creators.len() as u64,
            people_names,
            publisher: prefill_input(draft.publisher, "Publisher", window, cx),
            fields: BibliographicField::ALL
                .iter()
                .map(|field| {
                    let value = draft.fields.get(field).cloned().unwrap_or_default();
                    let input = if field.multiline() {
                        let input = cx.new(|cx| {
                            TextareaState::new(window, cx)
                                .rows(3)
                                .placeholder(field.label())
                        });
                        input.update(cx, |input, cx| input.set_value(value, window, cx));
                        BibliographicInput::Multiline(input)
                    } else {
                        BibliographicInput::Single(prefill_input(value, field.label(), window, cx))
                    };
                    (*field, input)
                })
                .collect(),
            series: cx.new(|cx| {
                ComboboxState::new(SeriesDelegate::new(&[]), Vec::new(), window, cx)
                    .searchable(true)
            }),
            series_options: Vec::new(),
            series_selected: record.series_id,
            series_loading: true,
            series_creating: false,
            series_number: prefill_input(draft.series_number, "e.g. LCL 26", window, cx),
            tags,
            collections,
            author_creating: false,
            favorite: record.favorite,
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
        let original = prefill_draft(&self.record);
        EditDraft {
            fields: self
                .fields
                .iter()
                .map(|(field, input)| (*field, input.value(cx)))
                .collect(),
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
            published_year: original.published_year,
            accessed_year: original.accessed_year,
            series: match self.series.read(cx).selected_values().first() {
                Some(SeriesChoice::Existing(id)) => id.to_string(),
                _ => String::new(),
            },
            series_number: value(&self.series_number),
            creators: self.creator_rows.iter().map(|row| row.spec(cx)).collect(),
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
        cx.bind_keys([
            KeyBinding::new(
                "shift-up",
                ExtendResourceSelectionUp,
                Some("LibraryApp > DataTable && !InlineEdit"),
            ),
            KeyBinding::new(
                "shift-down",
                ExtendResourceSelectionDown,
                Some("LibraryApp > DataTable && !InlineEdit"),
            ),
            KeyBinding::new(
                "cmd-a",
                SelectAllResources,
                Some("LibraryApp > DataTable && !InlineEdit"),
            ),
            KeyBinding::new(
                "ctrl-a",
                SelectAllResources,
                Some("LibraryApp > DataTable && !InlineEdit"),
            ),
            KeyBinding::new(
                "alt-right",
                ExpandResource,
                Some("LibraryApp && !Input && !InlineEdit"),
            ),
            KeyBinding::new(
                "alt-left",
                CollapseResource,
                Some("LibraryApp && !Input && !InlineEdit"),
            ),
        ]);
        cx.subscribe_in(
            &table,
            window,
            |this, _table, event: &TableEvent, window, cx| {
                match event {
                    TableEvent::ColumnWidthsChanged(widths) => {
                        this.save_column_widths(widths, cx);
                    }
                    TableEvent::SelectRow(row_ix) => {
                        if this.table.read(cx).selected_row() != Some(*row_ix) {
                            return;
                        }
                        this.table.update(cx, |table, cx| {
                            let delegate = table.delegate_mut();
                            delegate.selected_row = Some(*row_ix);
                            if !std::mem::take(&mut delegate.preserve_selection) {
                                delegate.selected_ids =
                                    delegate.row_id(*row_ix).into_iter().collect();
                                delegate.selection_anchor = delegate.row_id(*row_ix);
                            }
                            delegate.selected_col = None;
                            delegate.reveal_selection = None;
                            cx.notify();
                        });
                        this.select_row(*row_ix, window, cx);
                    }
                    TableEvent::SelectColumn(col_ix) => {
                        this.table
                            .update(cx, |table, _| table.delegate_mut().reveal_selection = None);
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
                            delegate.selected_ids.clear();
                            delegate.selection_anchor = None;
                            delegate.selected_col = Some(*col_ix);
                            cx.notify();
                        });
                        cx.notify();
                    }
                    TableEvent::ClearSelection => {
                        if this.table.read(cx).selected_row().is_some()
                            || this.table.read(cx).selected_col().is_some()
                        {
                            return;
                        }
                        this.table.update(cx, |table, cx| {
                            let delegate = table.delegate_mut();
                            delegate.selected_row = None;
                            delegate.selected_ids.clear();
                            delegate.selection_anchor = None;
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
            if this.theme_editor_open
                || event
                    .context_stack
                    .iter()
                    .any(|ctx| ctx.contains("ThemeColorEditor"))
            {
                this.last_escape_at = None;
                return;
            }
            if event
                .context_stack
                .iter()
                .any(|ctx| ctx.contains("ResourcePalette"))
            {
                this.last_escape_at = None;
                return;
            }
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

        let theme_editor = cx.new(|cx| ThemeEditor::new(window, cx));
        cx.subscribe_in(
            &theme_editor,
            window,
            |app, editor, event: &ThemeEditorEvent, window, cx| {
                let mode = if cx.theme().is_dark() {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                };
                match event {
                    ThemeEditorEvent::Preview(theme) if app.theme_editor_open => {
                        crate::theme::apply(mode, theme, cx);
                        cx.notify();
                    }
                    ThemeEditorEvent::Save(theme) if app.theme_editor_open => {
                        let started = Instant::now();
                        let mut settings = app.table.read(cx).delegate().settings.clone();
                        settings.theme = theme.clone();
                        let result = settings.save(&app.settings_dir);
                        eprintln!(
                            "{}",
                            serde_json::json!({
                                "level": "INFO", "command": "save_theme_colors", "surface": "gui",
                                "mode": if mode.is_dark() { "dark" } else { "light" },
                                "duration_us": started.elapsed().as_micros() as u64,
                                "status": if result.is_ok() { "ok" } else { "error" },
                                "error": result.as_ref().err().map(|error| error.to_string()),
                            })
                        );
                        match result {
                            Ok(()) => {
                                app.table.update(cx, |table, cx| {
                                    table.delegate_mut().settings.theme = theme.clone();
                                    cx.notify();
                                });
                                app.set_theme_editor_open(false, window, cx);
                            }
                            Err(error) => editor.update(cx, |editor, cx| {
                                editor.failed(format!("Could not save colors: {error}"), cx)
                            }),
                        }
                    }
                    ThemeEditorEvent::Cancel => app.set_theme_editor_open(false, window, cx),
                    _ => {}
                }
            },
        )
        .detach();
        Self {
            import_state: ImportState::Idle,
            import_gen: 0,
            import_task: None,
            table,
            settings_dir,
            theme_editor,
            theme_editor_open: false,
            published_from_input,
            published_to_input,
            detail_open: true,
            left_panel_open: true,
            resizing_sidebar: None,
            counts: None,
            counts_error: None,
            counts_gen: 0,
            collapsed_sections: HashSet::new(),
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
            attachment_pending: HashSet::new(),
            attachment_error: None,
            favorite_pending: HashSet::new(),
            suggestion_pending: HashSet::new(),
            jev: None,
            jev_gen: 0,
            jev_endpoint: crate::jev::DECISIONS_URL.to_owned(),
            rules_window: None,
            edit: None,
            edit_generation: 0,
            detail_gens: HashMap::new(),
            last_escape_at: None,
            inline_trap: cx.focus_handle(),
        }
    }

    fn import_biblatex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.import_state != ImportState::Idle || window.has_active_dialog(cx) {
            return;
        }
        if self.edit.is_some() || self.has_inline(cx) {
            window.push_notification(
                "Save or cancel your edit before importing a bibliography",
                cx,
            );
            return;
        }
        self.import_state = ImportState::Choosing;
        self.import_gen = self.import_gen.wrapping_add(1);
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import BibLaTeX (.bib)".into()),
        });
        let worker = self.table.read(cx).delegate().worker().clone();
        self.import_task = Some(cx.spawn_in(window, async move |app, window| {
            let result: Result<Option<crate::db::BiblatexImportPlan>, String> = async {
                let paths = paths
                    .await
                    .map_err(|e| format!("File picker closed: {e}"))?
                    .map_err(|e| format!("Unable to open file picker: {e}"))?;
                let Some(path) = paths.and_then(|paths| paths.into_iter().next()) else {
                    return Ok(None);
                };
                app.update_in(window, |app, _, cx| {
                    app.import_state = ImportState::Importing;
                    cx.notify();
                })
                .map_err(|e| e.to_string())?;
                worker.preview_biblatex(path).await.map(Some)
            }
            .await;
            _ = app.update_in(window, |app, window, cx| {
                app.import_state = ImportState::Idle;
                match result {
                    Ok(None) => {}
                    Ok(Some(plan)) => app.show_import_dialog(plan, window, cx),
                    Err(error) => app.show_import_report(Err(error), window, cx),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn commit_biblatex_import(
        &mut self,
        source_text: String,
        choices: ImportChoices,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.import_state != ImportState::Idle {
            return;
        }
        self.import_state = ImportState::Importing;
        let worker = self.table.read(cx).delegate().worker().clone();
        self.import_task = Some(cx.spawn_in(window, async move |app, window| {
            let result = worker.import_biblatex(source_text, choices).await;
            _ = app.update_in(window, |app, window, cx| {
                app.import_state = ImportState::Idle;
                app.show_import_report(result, window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Open the import dialog for a previewed bibliography. Nothing is
    /// written until the user confirms; Cancel and Escape close it as-is and
    /// the dialog root restores the previous focus.
    fn show_import_dialog(
        &mut self,
        plan: crate::db::BiblatexImportPlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let crate::db::BiblatexImportPlan {
            source_text,
            preview,
        } = plan;
        let tags = self.tags.clone();
        let collections = flatten_collections(&self.collection_forest);
        let dialog_preview = preview.clone();
        let content = cx.new(|cx| ImportDialog::new(dialog_preview, tags, collections, window, cx));
        cx.subscribe_in(
            &content,
            window,
            move |this, _, event: &ImportConfirmed, window, cx| {
                let mut choices = match event.policy {
                    None => ImportChoices::default(),
                    Some(ConflictPolicy::SkipConflicted) => safe_skip_choices(&preview),
                    Some(ConflictPolicy::ImportDuplicates) => {
                        intentional_duplicate_choices(&preview)
                    }
                };
                choices.assignments = event.assignments.clone();
                window.close_dialog(cx);
                this.commit_biblatex_import(source_text.clone(), choices, window, cx);
            },
        )
        .detach();
        window.open_dialog(cx, move |dialog, _, cx| {
            let footer = content.update(cx, |content, cx| content.footer(cx));
            dialog
                .title("Import bibliography")
                .entrance(gpui_kit::component::dialog::DialogEntrance::None)
                .width(px(560.))
                .child(content.clone())
                .footer(footer)
        });
    }

    /// Reload library tags and collections after a write that may have
    /// created some (an import with new batch targets).
    fn reload_tags_and_collections(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let tags = worker.list_tags().await;
            let forest = worker.list_collection_forest().await;
            _ = app.update_in(window, |app, window, cx| {
                if let Ok(tags) = tags {
                    app.tags = tags;
                    app.sync_tag_picker(window, cx);
                }
                if let Ok(forest) = forest {
                    app.set_collection_forest(forest, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn show_import_report(
        &mut self,
        result: Result<ImportReport, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Refresh even when later units failed — earlier commits remain.
        self.refresh_counts(window, cx);
        self.reload_tags_and_collections(window, cx);
        let selected = self.selected;
        self.table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .reload_from_top(selected, None, window, cx);
        });
        let (title, summary, issues) = match result {
            Ok(report) => {
                let summary = format!(
                    "{} imported · {} reused · {} skipped · {} conflicted · {} failed · {} not attempted · {} warnings",
                    report.imported.len(),
                    report.reused.len(),
                    report.skipped.len(),
                    report.conflicted.len(),
                    report.failed.len(),
                    report.not_attempted.len(),
                    report.warnings.len()
                );
                let count = report.skipped.len()
                    + report.warnings.len()
                    + report.conflicted.len()
                    + report.failed.len()
                    + report.not_attempted.len();
                let mut issues: Vec<String> = report
                    .skipped
                    .iter()
                    .map(|issue| ("Skipped", issue))
                    .chain(report.conflicted.iter().map(|issue| ("Conflict", issue)))
                    .chain(report.failed.iter().map(|issue| ("Failed", issue)))
                    .chain(
                        report
                            .not_attempted
                            .iter()
                            .map(|issue| ("Not attempted", issue)),
                    )
                    .chain(report.warnings.iter().map(|issue| ("Warning", issue)))
                    .take(100)
                    .map(|(kind, issue)| {
                        format!(
                            "{kind} — {}: {}",
                            issue.entry_key.as_deref().unwrap_or("bibliography"),
                            issue.message
                        )
                    })
                    .collect();
                if count > 100 {
                    issues.push(format!("Showing the first 100 of {count} issues."));
                }
                let targets = |names: Vec<String>| names.join(", ");
                if !report.assignments.tags.is_empty() {
                    issues.insert(
                        0,
                        format!(
                            "Tags — {}",
                            targets(
                                report
                                    .assignments
                                    .tags
                                    .iter()
                                    .map(|t| if t.created {
                                        format!("{} (new)", t.name)
                                    } else {
                                        t.name.clone()
                                    })
                                    .collect()
                            )
                        ),
                    );
                }
                if !report.assignments.collections.is_empty() {
                    issues.insert(
                        0,
                        format!(
                            "Collections — {}",
                            targets(
                                report
                                    .assignments
                                    .collections
                                    .iter()
                                    .map(|c| if c.created {
                                        format!("{} (new)", c.name)
                                    } else {
                                        c.name.clone()
                                    })
                                    .collect()
                            )
                        ),
                    );
                }
                ("BibLaTeX import complete", summary, issues)
            }
            Err(error) => (
                "BibLaTeX import failed",
                error,
                vec!["The import could not start; no entries were written.".into()],
            ),
        };
        window.open_dialog(cx, move |dialog, _, cx| {
            dialog.title(title).entrance(gpui_kit::component::dialog::DialogEntrance::None).width(px(560.)).child(
                v_flex().id("biblatex-import-report").test_support().aria_label(summary.clone()).gap_3()
                    .child(summary.clone())
                    .child(div().id("biblatex-import-issues").max_h(px(320.)).overflow_y_scroll()
                        .child(v_flex().gap_2().children(issues.iter().map(|issue| div().text_size(rems(1.)).child(issue.clone())))))
                    .child(div().text_size(rems(1.)).text_color(cx.theme().muted_foreground)
                        .child("Existing records are not overwritten. Same-source retries reuse committed entries. Current library filters still apply."))
            )
        });
    }

    fn set_theme_editor_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if open == self.theme_editor_open {
            return;
        }
        let mode = if cx.theme().is_dark() {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
        let settings = self.table.read(cx).delegate().settings.theme.clone();
        if open {
            if window.has_active_dialog(cx)
                || self.import_state != ImportState::Idle
                || self.edit.is_some()
                || self.has_inline(cx)
            {
                return;
            }
            let accent = cx.theme().primary;
            let secondary = workspace_gradient_stops(
                mode.is_dark(),
                settings.color(mode, ThemeColorTarget::Secondary),
            )
            .1;
            self.theme_editor.update(cx, |editor, cx| {
                editor.begin(mode, settings, [accent, secondary.into()], window, cx)
            });
        } else {
            crate::theme::apply(mode, &settings, cx);
        }
        self.last_escape_at = None;
        self.theme_editor_open = open;
        if !open {
            self.focus_table(window, cx);
        }
        cx.notify();
    }

    fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(window, cx);
    }

    /// Open the palette straight on one collection's classification screen.
    fn open_collection_classification(
        &mut self,
        id: CollectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(palette) = self.open_palette(window, cx) {
            palette.update(cx, |palette, cx| {
                palette.open_classification(id, window, cx)
            });
        }
    }

    fn open_palette(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<CommandPalette>> {
        if self.wait_for_library_write(window, cx) {
            return None;
        }
        if self.theme_editor_open {
            return None;
        }
        if self.import_state != ImportState::Idle {
            window.push_notification("Wait for the bibliography import to finish", cx);
            return None;
        }
        if window.has_active_dialog(cx) {
            return None;
        }
        if self.edit.is_some() || self.has_inline(cx) {
            window.push_notification(
                "Save or cancel your edit before opening resource commands",
                cx,
            );
            return None;
        }
        self.last_escape_at = None;
        let targets: Vec<_> = {
            let delegate = self.table.read(cx).delegate();
            if delegate.selected_ids.is_empty() {
                self.selected.into_iter().collect()
            } else {
                delegate
                    .rows
                    .iter()
                    .filter(|r| delegate.selected_ids.contains(&r.id))
                    .map(|r| r.id)
                    .collect()
            }
        };
        let grouped = self.table.read(cx).delegate().grouped;
        let preferences = &self.table.read(cx).delegate().settings.theme;
        let secondary = preferences
            .color(
                if cx.theme().is_dark() {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                },
                ThemeColorTarget::Secondary,
            )
            .map(rgb)
            .unwrap_or_else(|| workspace_gradient_stops(cx.theme().is_dark(), None).1);
        let hex = |color: Rgba| {
            format!(
                "#{:02X}{:02X}{:02X}",
                (color.r * 255.).round() as u8,
                (color.g * 255.).round() as u8,
                (color.b * 255.).round() as u8
            )
        };
        let colors = (hex(cx.theme().primary.to_rgb()), hex(secondary));
        let library_collections = flatten_collections(&self.collection_forest);
        let jev_settings = {
            let settings = &self.table.read(cx).delegate().settings;
            (
                settings.openrouter_api_key.is_some(),
                settings.jev_model.clone(),
            )
        };
        let palette = cx.new(|cx| {
            let mut palette = if targets.len() <= 1 {
                CommandPalette::new(targets.first().copied(), grouped, window, cx)
            } else {
                CommandPalette::new_for_targets(targets.clone(), grouped, window, cx)
            };
            palette.set_colors(colors.0, colors.1);
            palette.set_library_collections(library_collections);
            palette.set_jev_settings(jev_settings.0, jev_settings.1);
            palette
        });
        cx.subscribe_in(
            &palette,
            window,
            |app, palette, request: &JevSettingRequest, window, cx| {
                let started = std::time::Instant::now();
                let mut settings = app.table.read(cx).delegate().settings.clone();
                let (command, result) = match request {
                    JevSettingRequest::ApiKey(key) => (
                        "set_openrouter_api_key",
                        settings.set_openrouter_api_key(key, &app.settings_dir),
                    ),
                    JevSettingRequest::Model(model) => (
                        "set_jev_model",
                        settings.set_jev_model(model, &app.settings_dir),
                    ),
                };
                // The key itself is never logged, only whether one is set.
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "level": "INFO", "command": command, "surface": "gui",
                        "jev.key_set": settings.openrouter_api_key.is_some(),
                        "jev.model": settings.jev_model(),
                        "duration_us": started.elapsed().as_micros() as u64,
                        "status": if result.is_ok() { "ok" } else { "error" },
                        "error": result.as_ref().err().map(|error| error.to_string()),
                    })
                );
                match result {
                    Ok(()) => {
                        let message = match request {
                            JevSettingRequest::ApiKey(key) if key.trim().is_empty() => {
                                "Removed the OpenRouter API key".to_owned()
                            }
                            JevSettingRequest::ApiKey(_) => {
                                "Saved the OpenRouter API key".to_owned()
                            }
                            JevSettingRequest::Model(_) => {
                                format!("Classification model set to {}", settings.jev_model())
                            }
                        };
                        app.table.update(cx, |table, cx| {
                            table.delegate_mut().settings = settings;
                            cx.notify();
                        });
                        window.close_dialog(cx);
                        app.focus_table(window, cx);
                        window.push_notification(message, cx);
                        cx.notify();
                    }
                    Err(error) => palette.update(cx, |palette, cx| {
                        palette.identifier_failed(format!("Could not save: {error}"), cx)
                    }),
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &palette,
            window,
            |app, _, _: &OpenClassificationRules, window, cx| {
                window.close_dialog(cx);
                app.focus_table(window, cx);
                app.open_classification_rules(window, cx);
            },
        )
        .detach();
        cx.subscribe_in(
            &palette,
            window,
            |app, _, _: &ClassifyWithJevRequest, window, cx| {
                window.close_dialog(cx);
                app.focus_table(window, cx);
                app.classify_with_jev(window, cx);
            },
        )
        .detach();
        cx.subscribe_in(
            &palette,
            window,
            |app, palette, request: &CollectionClassificationRequest, window, cx| {
                let request = request.clone();
                let palette = palette.downgrade();
                let worker = app.table.read(cx).delegate().worker().clone();
                cx.spawn_in(window, async move |app, window| {
                    let result = worker
                        .set_collection_classification(
                            request.id,
                            request.keywords,
                            request.jev_enabled,
                        )
                        .await;
                    let forest = match &result {
                        Ok(_) => worker.list_collection_forest().await.ok(),
                        Err(_) => None,
                    };
                    _ = app.update_in(window, |app, window, cx| match result {
                        Ok(collection) => {
                            if let Some(forest) = forest {
                                app.set_collection_forest(forest, window, cx);
                            }
                            if palette.upgrade().is_some() {
                                window.close_dialog(cx);
                            }
                            app.focus_table(window, cx);
                            window.push_notification(
                                format!("Saved classification for {}", collection.name.as_str()),
                                cx,
                            );
                            cx.notify();
                        }
                        Err(error) => {
                            _ = palette
                                .update(cx, |palette, cx| palette.identifier_failed(error, cx));
                        }
                    });
                })
                .detach();
            },
        )
        .detach();
        cx.subscribe_in(
            &palette,
            window,
            |app, palette, request: &SetThemeColor, window, cx| {
                let started = std::time::Instant::now();
                let mut settings = app.table.read(cx).delegate().settings.clone();
                let result = settings.set_theme_color(
                    if cx.theme().is_dark() {
                        ThemeMode::Dark
                    } else {
                        ThemeMode::Light
                    },
                    request.target,
                    &request.hex,
                    &app.settings_dir,
                );
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "level": "INFO", "command": "set_theme_color", "surface": "gui",
                        "mode": if cx.theme().is_dark() { "dark" } else { "light" },
                        "target": format!("{:?}", request.target), "color": request.hex,
                        "duration_us": started.elapsed().as_micros() as u64,
                        "status": if result.is_ok() { "ok" } else { "error" },
                        "error": result.as_ref().err().map(|error| error.to_string()),
                    })
                );
                match result {
                    Ok(()) => {
                        crate::theme::apply(
                            if cx.theme().is_dark() {
                                ThemeMode::Dark
                            } else {
                                ThemeMode::Light
                            },
                            &settings.theme,
                            cx,
                        );
                        app.table.update(cx, |table, cx| {
                            table.delegate_mut().settings = settings;
                            cx.notify();
                        });
                        window.close_dialog(cx);
                        app.focus_table(window, cx);
                        cx.notify();
                    }
                    Err(error) => palette.update(cx, |palette, cx| {
                        palette
                            .identifier_failed(format!("Could not save theme color: {error}"), cx)
                    }),
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &palette,
            window,
            |app, _, _: &ToggleResourceGrouping, window, cx| {
                window.close_dialog(cx);
                let selected = app.selected;
                app.table.update(cx, |table, cx| {
                    let delegate = table.delegate_mut();
                    delegate.grouped = !delegate.grouped;
                    delegate.reload_from_top(selected, None, window, cx);
                });
                app.focus_table(window, cx);
            },
        )
        .detach();
        cx.subscribe_in(
            &palette,
            window,
            |app, palette, request: &PaletteRequest, window, cx| {
                let request = request.clone();
                let palette = palette.downgrade();
                let worker = app.table.read(cx).delegate().worker().clone();
                cx.spawn_in(window, async move |app, window| {
                    let result = apply_palette_batch(&worker, &request).await;
                    _ = app.update_in(window, |app, window, cx| {
                        for id in &result.updated {
                            *app.detail_gens.entry(*id).or_default() += 1;
                            app.detail_inflight.remove(id);
                            app.detail_cache.invalidate(id);
                            if app.selected == Some(*id) {
                                app.detail = Some((*id, DetailFetch::Loading));
                                app.fetch_detail(*id, window, cx);
                            }
                        }
                        if !result.updated.is_empty() {
                            app.refresh_counts(window, cx);
                            let selected = app.selected;
                            app.table.update(cx, |table, cx| {
                                table
                                    .delegate_mut()
                                    .reload_from_top(selected, None, window, cx)
                            });
                        }
                        if result.failed.is_empty() {
                            if palette.upgrade().is_some() {
                                window.close_dialog(cx);
                            }
                        } else {
                            let failures = result
                                .failed
                                .iter()
                                .map(|(id, error)| format!("{id}: {error}"))
                                .collect::<Vec<_>>()
                                .join("; ");
                            let message = format!(
                                "{} updated; {} failed. Retry the same changes, or cancel to start a new edit. {failures}",
                                result.updated.len(),
                                result.failed.len()
                            );
                            _ = palette.update(cx, |palette, cx| palette.failed(message, cx));
                        }
                        cx.notify();
                    });
                })
                .detach();
            },
        )
        .detach();
        cx.subscribe_in(
            &palette,
            window,
            |app, palette, request: &IdentifierRequest, window, cx| {
                let request = request.clone();
                let palette = palette.downgrade();
                let worker = app.table.read(cx).delegate().worker().clone();
                cx.spawn_in(window, async move |app, window| {
                    let before = match request.target {
                        Some(id) => worker.resource(id).await.ok(),
                        None => None,
                    };
                    let result = worker
                        .resolve_identifier(request.identifier, request.target)
                        .await;
                    _ = app.update_in(window, |app, window, cx| match result {
                        Ok(resource) => {
                            if before.as_ref().is_some_and(|before| {
                                classification_input_changed(&before.to_new(), &resource.to_new())
                            }) {
                                app.invalidate_jev(resource.id);
                            }
                            *app.detail_gens.entry(resource.id).or_default() += 1;
                            app.detail_inflight.remove(&resource.id);
                            app.detail_cache.invalidate(&resource.id);
                            if app.selected == Some(resource.id) {
                                app.detail = Some((resource.id, DetailFetch::Loading));
                                app.fetch_detail(resource.id, window, cx);
                            }
                            app.refresh_counts(window, cx);
                            let selected = app.selected;
                            app.table.update(cx, |table, cx| {
                                table
                                    .delegate_mut()
                                    .reload_from_top(selected, None, window, cx)
                            });
                            if palette.upgrade().is_some() {
                                window.close_dialog(cx);
                            }
                            window.push_notification(
                                format!("Saved {}", resource.title.as_str()),
                                cx,
                            );
                        }
                        Err(error) => {
                            _ = palette
                                .update(cx, |palette, cx| palette.identifier_failed(error, cx));
                        }
                    });
                })
                .detach();
            },
        )
        .detach();

        let initial_focus = std::cell::Cell::new(true);
        let content = palette.clone();
        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                content.update(cx, |palette, cx| palette.focus(window, cx));
            }
            dialog
                .close_button(false)
                .overlay_closable(false)
                .keyboard(false)
                // Commands should appear in place, not slide beneath the pointer.
                .entrance(gpui_kit::component::dialog::DialogEntrance::None)
                .width(px(480.))
                .p_0()
                .child(content.clone())
        });
        if !targets.is_empty() {
            let worker = self.table.read(cx).delegate().worker().clone();
            let palette = palette.downgrade();
            cx.spawn_in(window, async move |_app, window| {
                let result = async {
                    let mut records = Vec::new();
                    for id in targets {
                        records.push(worker.resource(id).await?);
                    }
                    let tags = worker.list_tags().await?;
                    let forest = worker.list_collection_forest().await?;
                    Ok::<_, String>((records, tags, flatten_collections(&forest)))
                }
                .await;
                _ = palette.update_in(window, |palette, _, cx| match result {
                    Ok((mut records, tags, collections)) => {
                        if records.len() == 1 {
                            palette.loaded(records.pop().unwrap(), tags, collections, cx);
                        } else {
                            palette.loaded_resources(records, tags, collections, cx);
                        }
                    }
                    Err(error) => palette.failed(error, cx),
                });
            })
            .detach();
        }
        Some(palette)
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

    /// Focus the rules window, opening it first when needed.
    fn open_classification_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((handle, _)) = &self.rules_window {
            if cx
                .update_window(*handle, |_, window, _| window.activate_window())
                .is_ok()
            {
                return;
            }
            self.rules_window = None;
        }
        let worker = self.table.read(cx).delegate().worker().clone();
        let collections = flatten_collections(&self.collection_forest);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(760.), px(520.)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Classification rules".into()),
                traffic_light_position: Some(point(px(9.), px(13.))),
                ..TitleBar::title_bar_options()
            }),
            ..Default::default()
        };
        let opened = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| ClassificationRules::new(worker, collections, window, cx))
        });
        let (handle, rules) = match opened {
            Ok(opened) => opened,
            Err(err) => {
                eprintln!("warning: could not open classification rules: {err:#}");
                return;
            }
        };
        cx.subscribe_in(
            &rules,
            window,
            |app, _, _: &ClassificationRulesSaved, window, cx| {
                app.refresh_collections_after_rule_save(window, cx);
            },
        )
        .detach();
        self.rules_window = Some((handle, rules));
    }

    /// Rules feed suggestions and auto-classify candidates, so reload the
    /// forest those read from.
    fn refresh_collections_after_rule_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let Ok(forest) = worker.list_collection_forest().await else {
                return;
            };
            _ = app.update_in(window, |app, window, cx| {
                app.set_collection_forest(forest, window, cx);
                cx.notify();
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
        let mut previous_parents = HashSet::new();
        expand_all_parents(&self.collection_forest, &mut previous_parents);
        let mut parents = HashSet::new();
        expand_all_parents(&forest, &mut parents);
        // New branches open by default; known branches keep the user's choice.
        self.expanded_collections.retain(|id| parents.contains(id));
        self.expanded_collections
            .extend(parents.difference(&previous_parents).copied());
        self.collection_forest = forest;
        self.sync_collection_picker(window, cx);
        if let Some((handle, rules)) = self.rules_window.clone() {
            let collections = flatten_collections(&self.collection_forest);
            let synced = cx.update_window(handle, |_, window, cx| {
                rules.update(cx, |rules, cx| {
                    rules.set_collections(collections, window, cx)
                })
            });
            if synced.is_err() {
                self.rules_window = None;
            }
        }
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
        let CollectionChoice::Existing(choice) = choice else {
            return;
        };
        let Some(collection) = flatten_collections(&self.collection_forest)
            .into_iter()
            .find(|c| c.id == choice)
        else {
            return;
        };
        let Some(EditFetch::Ready(form)) = &mut self.edit else {
            return;
        };
        if form.collections.iter().any(|c| c.id == choice) {
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
        // Keep the newly created row and inline rename visible even when the
        // collection list was collapsed before the + button was pressed.
        self.collapsed_sections.remove(&SidebarSection::Collections);
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
            if let Some(old_parent) = from_parent
                && worker
                    .remove_collection_parent(child_id, old_parent)
                    .await
                    .is_err()
            {
                return;
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
                if let Some(EditFetch::Ready(form)) = &mut self.edit
                    && !form.tags.iter().any(|t| t.id == tag.id)
                {
                    form.tags.push(tag);
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

    /// Ask Jev to rank the selected resource against every Jev-enabled
    /// collection. Failures land in the panel rather than a notification, so
    /// the reason stays visible next to the button that caused it.
    fn classify_with_jev(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_inline(cx) || self.edit.is_some() {
            return;
        }
        let Some(id) = self.selected else {
            return;
        };
        if self.table.read(cx).delegate().selected_ids.len() > 1 {
            return;
        }
        if self
            .jev
            .as_ref()
            .is_some_and(|jev| jev.resource_id == id && jev.status == JevStatus::Pending)
        {
            return;
        }
        self.jev_gen += 1;
        let generation = self.jev_gen;
        let fail = |message: &str| JevState {
            resource_id: id,
            status: JevStatus::Failed(message.to_owned()),
        };
        let settings = &self.table.read(cx).delegate().settings;
        let Some(key) = settings
            .openrouter_api_key
            .as_ref()
            .map(|k| k.expose().to_owned())
        else {
            self.jev = Some(fail(
                "Set an OpenRouter API key in the command palette (⌘K) to auto-classify",
            ));
            cx.notify();
            return;
        };
        let model = settings.jev_model().to_owned();
        let DetailView::Fields(detail) = self.detail_view() else {
            self.jev = Some(fail(
                "Wait for the resource details to load, then try again",
            ));
            cx.notify();
            return;
        };
        let detail = detail.clone();
        let candidates: Vec<Collection> = flatten_collections(&self.collection_forest)
            .into_iter()
            .filter(|c| c.jev_enabled)
            .collect();
        if candidates.is_empty() {
            self.jev = Some(fail(
                "No collections are included in auto-classify; turn it on in Classification rules",
            ));
            cx.notify();
            return;
        }
        // Exclude memberships before Jev's top-five cap, otherwise existing
        // collections can crowd every useful new match out of the response.
        let candidates: Vec<_> = candidates
            .into_iter()
            .filter(|c| !detail.collections.iter().any(|member| member.id == c.id))
            .collect();
        if candidates.is_empty() {
            self.jev = Some(fail(
                "This resource already belongs to every collection included in auto-classify",
            ));
            cx.notify();
            return;
        }
        self.jev = Some(JevState {
            resource_id: id,
            status: JevStatus::Pending,
        });
        let worker = self.table.read(cx).delegate().worker().clone();
        let url = self.jev_endpoint.clone();
        let candidate_count = candidates.len();
        let started = Instant::now();
        cx.spawn_in(window, async move |app, window| {
            let result = worker
                .jev_classify(url, detail, candidates, key, model.clone())
                .await;
            eprintln!(
                "{}",
                serde_json::json!({
                    "level": "INFO", "command": "jev_classify", "surface": "gui",
                    "resource.id": id.to_string(), "jev.model": model,
                    "jev.candidate_count": candidate_count,
                    "jev.suggestion_count": result.as_ref().map(Vec::len).unwrap_or(0),
                    "duration_us": started.elapsed().as_micros() as u64,
                    "status": if result.is_ok() { "ok" } else { "error" },
                    "error": result.as_ref().err(),
                })
            );
            _ = app.update_in(window, |app, _, cx| {
                app.finish_jev(generation, id, result, cx)
            });
        })
        .detach();
        cx.notify();
    }

    /// Keep the reply for its resource even while another row is selected.
    fn finish_jev(
        &mut self,
        generation: u64,
        id: ResourceId,
        result: Result<crate::jev::JevSuggestions, String>,
        cx: &mut Context<Self>,
    ) {
        if self.jev_gen != generation || !self.jev.as_ref().is_some_and(|jev| jev.resource_id == id)
        {
            return;
        }
        self.jev = Some(JevState {
            resource_id: id,
            status: match result {
                Ok(picks) => JevStatus::Ready(picks),
                Err(error) => JevStatus::Failed(error),
            },
        });
        cx.notify();
    }

    /// Classification outlives selection, but never the content it classified.
    fn invalidate_jev(&mut self, id: ResourceId) {
        if self.jev.as_ref().is_some_and(|jev| jev.resource_id == id) {
            self.jev_gen += 1;
            self.jev = None;
        }
    }

    /// Toggle by stored identity even when the detail panel is still loading.
    fn toggle_favorite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.wait_for_library_write(window, cx) {
            return;
        }
        if self.has_inline(cx) {
            return;
        }
        let Some(id) = self.selected else {
            return;
        };
        if !self.favorite_pending.insert(id) {
            return;
        }
        let worker = self.table.read(cx).delegate().worker().clone();
        // Bump the generation so an in-flight pre-toggle detail cannot
        // overwrite the starred state after the toggle lands.
        let epoch = self.detail_gens.entry(id).or_insert(0);
        *epoch += 1;
        let fetch_epoch = *epoch;
        self.detail_cache.invalidate(&id);
        cx.spawn_in(window, async move |app, window| {
            let reply = async {
                let current = worker.resource(id).await?;
                worker.set_resource_favorite(id, !current.favorite).await
            }
            .await;
            _ = app.update_in(window, |app, window, cx| {
                app.favorite_pending.remove(&id);
                // A generation guards read freshness, not persisted writes:
                // concurrent metadata reads may still precede this write.
                // Only an obsolete error should leave newer detail untouched.
                if reply.is_err() && *app.detail_gens.get(&id).unwrap_or(&0) != fetch_epoch {
                    cx.notify();
                    return;
                }
                app.detail_inflight.remove(&id);
                // Reads started at any generation during the write may contain
                // the old favorite. Reject their replies and cached results.
                *app.detail_gens.entry(id).or_insert(0) += 1;
                app.detail_cache.invalidate(&id);
                match reply {
                    Ok(resource) => {
                        // Optimistically patch favorite on a cached/loaded
                        // detail if present, then refetch for consistency.
                        if let Some((_, DetailFetch::Loaded(detail))) = app.detail.as_mut()
                            && detail.id == id
                        {
                            detail.favorite = resource.favorite;
                        }
                        if app.selected == Some(id) {
                            app.fetch_detail(id, window, cx);
                        }
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
        if !editing_this || self.table.read(cx).delegate().selected_ids.len() > 1 {
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
                // The record this reply describes was edited after the
                // fetch was issued; it must not enter the cache or the
                // panel. The save's own refetch (issued at the new
                // epoch) fills both.
                if *app.detail_gens.get(&id).unwrap_or(&0) != fetch_epoch {
                    return;
                }
                app.detail_inflight.remove(&id);
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

    fn add_attachment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        if self.attachment_pending.contains(&id) || window.has_active_dialog(cx) {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Add attachment".into()),
        });
        self.attachment_pending.insert(id);
        self.attachment_error = None;
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let result: Result<Option<ibex_core::Attachment>, String> = async {
                let paths = paths
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                let Some(path) = paths.and_then(|paths| paths.into_iter().next()) else {
                    return Ok(None);
                };
                worker.add_attachment(id, path).await.map(Some)
            }
            .await;
            _ = app.update_in(window, |app, window, cx| {
                app.attachment_pending.remove(&id);
                match result {
                    Ok(Some(_)) => {
                        app.detail_inflight.remove(&id);
                        *app.detail_gens.entry(id).or_insert(0) += 1;
                        app.detail_cache.invalidate(&id);
                        if app.selected == Some(id) {
                            app.detail = Some((id, DetailFetch::Loading));
                            app.fetch_detail(id, window, cx);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => app.attachment_error = Some((id, error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_attachment(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.selected;
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            #[cfg(target_os = "macos")]
            let result = std::process::Command::new("open").arg(&path).status();
            #[cfg(target_os = "linux")]
            let result = std::process::Command::new("xdg-open").arg(&path).status();
            #[cfg(target_os = "windows")]
            let result = std::process::Command::new("explorer.exe")
                .arg(&path)
                .status();
            let _ = tx.send(result.map_err(|e| e.to_string()).and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("launcher exited with {status}"))
                }
            }));
        });
        cx.spawn_in(window, async move |app, window| {
            if let Ok(Err(error)) = rx.await {
                _ = app.update_in(window, |app, _, cx| {
                    if let Some(id) = id {
                        app.attachment_error =
                            Some((id, format!("Could not open attachment: {error}")));
                        cx.notify();
                    }
                });
            }
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

    fn disclose_selected_resource(
        &mut self,
        expand: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.table.update(cx, |table, cx| {
            let Some(id) = table
                .selected_row()
                .and_then(|ix| table.delegate().row_id(ix))
            else {
                return;
            };
            let target = if !expand && !table.delegate().grid.expanded.contains(&id) {
                table.delegate().grid.parent(id).unwrap_or(id)
            } else {
                id
            };
            if expand && table.delegate().grid.expanded.contains(&target) {
                if table.delegate().inline.is_none() {
                    table.delegate_mut().request_children(target, window, cx);
                }
            } else {
                change_resource_expansion(table, target, expand, window, cx);
            }
        });
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

    fn extend_resource_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.has_inline(cx) || self.edit.is_some() {
            return;
        }
        self.table.update(cx, |table, cx| {
            let current = table.selected_row();
            let delegate = table.delegate_mut();
            let Some(last) = delegate.rows.len().checked_sub(1) else {
                return;
            };
            let next = current.map_or(if delta < 0 { last } else { 0 }, |ix| {
                ix.saturating_add_signed(delta).min(last)
            });
            let anchor = delegate
                .selection_anchor
                .and_then(|id| delegate.rows.iter().position(|row| row.id == id))
                .unwrap_or(current.unwrap_or(next).min(last));
            delegate.selection_anchor = Some(delegate.rows[anchor].id);
            // Keep the origin fixed so reversing direction shrinks the range.
            delegate.selected_ids = delegate.rows[anchor.min(next)..=anchor.max(next)]
                .iter()
                .map(|row| row.id)
                .collect();
            delegate.preserve_selection = true;
            table.set_selected_row(next, cx);
        });
        cx.notify();
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
    fn move_creator(&mut self, key: u64, direction: isize, cx: &mut Context<Self>) {
        let Some(EditFetch::Ready(form)) = &mut self.edit else {
            return;
        };
        if form.saving {
            return;
        }
        let Some(ix) = form.creator_rows.iter().position(|r| r.key == key) else {
            return;
        };
        if let Some(next) = ix
            .checked_add_signed(direction)
            .filter(|next| *next < form.creator_rows.len())
        {
            form.creator_rows.swap(ix, next);
            cx.notify();
        }
    }

    fn add_creator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(EditFetch::Ready(form)) = &mut self.edit else {
            return;
        };
        if form.saving || form.author_creating {
            return;
        }
        for id in selected_person_ids(&form.creators.read(cx).selected_values()) {
            let name = form
                .people_names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| "Unknown person".into());
            let spec = ibex_core::CreatorSpec {
                person_id: id,
                role: ibex_core::CreatorRole::Author,
                credited_as: None,
            };
            form.creator_rows.push(CreatorRow::new(
                form.next_creator_key,
                spec,
                name,
                window,
                cx,
            ));
            form.next_creator_key += 1;
        }
        cx.notify();
    }

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
                                if !inline && let Some(EditFetch::Ready(form)) = &mut app.edit {
                                    form.people_names.insert(person.id, person.name.display());
                                }
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
        if self.wait_for_library_write(window, cx) {
            return;
        }
        if self.import_state != ImportState::Idle {
            window.push_notification("Wait for the bibliography import to finish", cx);
            return;
        }
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
                        let row_ix = app
                            .table
                            .read(cx)
                            .delegate()
                            .inline
                            .as_ref()
                            .expect("session checked above")
                            .row_ix();
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
        if self.wait_for_library_write(window, cx) {
            return;
        }
        if self.import_state != ImportState::Idle {
            window.push_notification("Wait for the bibliography import to finish", cx);
            return;
        }
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
        let import_gen = self.import_gen;
        cx.spawn_in(window, async move |app, window| {
            let people = worker.list_persons().await.unwrap_or_default();
            _ = app.update_in(window, |app, window, cx| {
                if app.import_gen != import_gen || app.import_state != ImportState::Idle {
                    return;
                }
                if app.wait_for_library_write(window, cx) {
                    return;
                }
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
        let content_changed = record
            .as_ref()
            .is_some_and(|record| classification_input_changed(&record.to_new(), &new));
        cx.spawn_in(window, async move |app, window| {
            let reply = if let Some(id) = update_id {
                worker.update_resource(id, new).await.map(|r| r.id)
            } else {
                worker.create_resource(new).await.map(|r| r.id)
            };
            _ = app.update_in(window, |app, window, cx| {
                match reply {
                    Ok(id) => {
                        if content_changed {
                            app.invalidate_jev(id);
                        }
                        app.table.update(cx, |table, _| {
                            table.delegate_mut().inline = None;
                            table.col_selectable = true;
                        });
                        app.selected = Some(id);
                        // Any cached article may inherit from the edited container.
                        // Reject pre-edit replies as well as cached projections.
                        for pending in app.detail_inflight.drain() {
                            *app.detail_gens.entry(pending).or_default() += 1;
                        }
                        app.detail_cache = DetailCache::new();
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

    fn load_series_options(
        &mut self,
        picker: Entity<ComboboxState<SeriesDelegate>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let result = worker.list_series().await;
            _ = app.update_in(window, |app, window, cx| {
                let Some(EditFetch::Ready(form)) = &mut app.edit else {
                    return;
                };
                if form.series != picker {
                    return;
                }
                match result {
                    Ok(options) => {
                        let selected = form.record.series_id.map(SeriesChoice::Existing);
                        picker.update(cx, |state, cx| {
                            state.set_items(SeriesDelegate::new(&options), window, cx);
                            state.set_selected_values(
                                &selected.into_iter().collect::<Vec<_>>(),
                                window,
                                cx,
                            );
                        });
                        form.series_options = options;
                        form.series_loading = false;
                        form.error = None;
                    }
                    Err(err) => {
                        form.error = Some(format!(
                            "Could not load series: {err}. Cancel and reopen to retry."
                        ))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn wire_series_creation(
        picker: &Entity<ComboboxState<SeriesDelegate>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.subscribe_in(
            picker,
            window,
            |app, picker, event: &ComboboxEvent<SeriesDelegate>, window, cx| {
                let ComboboxEvent::Change(choices) = event else {
                    return;
                };
                let Some(EditFetch::Ready(form)) = &mut app.edit else {
                    return;
                };
                if form.series != *picker
                    || form.series_creating
                    || form.series_loading
                    || form.saving
                {
                    return;
                }
                let Some(SeriesChoice::Create(title)) = choices.first() else {
                    form.series_selected = choices.first().and_then(|choice| match choice {
                        SeriesChoice::Existing(id) => Some(*id),
                        SeriesChoice::Create(_) => None,
                    });
                    return;
                };
                let title = title.clone();
                let previous = form.series_selected.map(SeriesChoice::Existing);
                picker.update(cx, |state, cx| {
                    state.set_selected_values(&previous.into_iter().collect::<Vec<_>>(), window, cx)
                });
                form.series_creating = true;
                form.error = None;
                let picker = picker.clone();
                let worker = app.table.read(cx).delegate().worker().clone();
                cx.notify();
                cx.spawn_in(window, async move |app, window| {
                    let result = worker.create_series(title).await;
                    _ = app.update_in(window, |app, window, cx| {
                        let Some(EditFetch::Ready(form)) = &mut app.edit else {
                            return;
                        };
                        if form.series != picker {
                            return;
                        }
                        form.series_creating = false;
                        match result {
                            Ok(series) => {
                                let id = series.id;
                                form.series_options.push(series);
                                picker.update(cx, |state, cx| {
                                    state.set_items(
                                        SeriesDelegate::new(&form.series_options),
                                        window,
                                        cx,
                                    );
                                    state.set_selected_values(
                                        &[SeriesChoice::Existing(id)],
                                        window,
                                        cx,
                                    );
                                });
                            }
                            Err(err) => {
                                form.error = Some(format!("Could not create series: {err}"))
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
            },
        )
        .detach();
    }

    /// Open the edit form for the selected resource: fetch the raw record
    /// (the panel's projection drops the ids the form needs), then open
    /// the prefilled widgets — the parent box empty until its options land,
    /// a second fetch in the same flow. One form at a time; the sidebar
    /// swaps to it while it exists.
    fn request_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.wait_for_library_write(window, cx) {
            return;
        }
        if self.table.read(cx).delegate().selected_ids.len() > 1 {
            window.push_notification("Select one resource to edit its details", cx);
            return;
        }
        if self.import_state != ImportState::Idle {
            window.push_notification("Wait for the bibliography import to finish", cx);
            return;
        }
        let Some(id) = self.selected else {
            return;
        };
        if self.edit.is_some() {
            return;
        }
        self.cancel_inline(cx);
        self.edit = Some(EditFetch::Loading(id));
        self.edit_generation += 1;
        let generation = self.edit_generation;
        self.detail_open = true;
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let reply = match worker.resource(id).await {
                Ok(record) => worker.list_persons().await.map(|people| (record, people)),
                Err(err) => Err(err),
            };
            _ = app.update_in(window, |app, window, cx| {
                if app.edit_generation != generation
                    || !matches!(app.edit, Some(EditFetch::Loading(loading)) if loading == id)
                {
                    return;
                }
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
                            Self::wire_series_creation(&form.series, window, cx);
                            let series_picker = form.series.clone();
                            app.edit = Some(EditFetch::Ready(Box::new(form)));
                            app.load_series_options(series_picker, window, cx);
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
        // Single expected kind: filter at the store. Multiple (e.g. article →
        // issue|journal): fetch a broader page and narrow client-side.
        let query = ResourceQuery {
            sort: None,
            filter: if expected.len() == 1 {
                ResourceFilter::by_kind(expected[0])
            } else {
                ResourceFilter::none()
            },
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
        if matches!(&self.edit, Some(EditFetch::Ready(form)) if form.saving) {
            return;
        }
        self.edit_generation += 1;
        self.focus_table(window, cx);
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
        if form.saving || form.author_creating || form.series_loading || form.series_creating {
            return;
        }
        let record = form.record.clone();
        let favorite = form.favorite;
        let form_identity = form.title.clone();
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
        let content_changed = classification_input_changed(&record.to_new(), &new);
        if let Some(EditFetch::Ready(form)) = &mut self.edit {
            form.saving = true;
            form.error = None;
        }
        // Hand the keyboard back to the grid so arrow keys work while the
        // save runs; a rejected draft above keeps focus on the form.
        self.focus_table(window, cx);
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let reply = match worker.update_resource(id, new).await {
                Ok(resource) if favorite != resource.favorite => worker.set_resource_favorite(id, favorite).await
                    .map_err(|err| format!("Bibliography saved, but favorite could not be saved: {err}. Retry Save.")),
                other => other,
            };
            _ = app.update_in(window, |app, window, cx| {
                if !matches!(&app.edit, Some(EditFetch::Ready(form)) if form.title == form_identity) { return; }
                match reply {
                    Ok(_) => {
                        if content_changed {
                            app.invalidate_jev(id);
                        }
                        app.edit = None;
                        // Any cached article may inherit from the edited container.
                        // Reject pre-edit replies as well as cached projections.
                        for pending in app.detail_inflight.drain() {
                            *app.detail_gens.entry(pending).or_default() += 1;
                        }
                        app.detail_cache = DetailCache::new();
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

    // gpui render helper: row state plus window and context handles.
    #[allow(clippy::too_many_arguments)]
    fn sidebar_row(
        &self,
        index: usize,
        nav: SidebarNav,
        label: String,
        icon: Icon,
        count: Option<u64>,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let hover = cx.theme().accent;
        h_flex()
            .id(("sidebar-entry", index))
            .test_support()
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

    fn sidebar_section_button(&self, section: SidebarSection, cx: &mut Context<Self>) -> Button {
        let collapsed = self.collapsed_sections.contains(&section);
        Button::new(SharedString::from(format!(
            "toggle-section-{}",
            section.key()
        )))
        .ghost()
        .xsmall()
        .icon(if collapsed {
            Icon::new(IconName::ChevronRight).xsmall()
        } else {
            Icon::new(IconName::ChevronDown).xsmall()
        })
        .tooltip(match (section, collapsed) {
            (SidebarSection::Kinds, true) => "Expand Kinds",
            (SidebarSection::Kinds, false) => "Collapse Kinds",
            (SidebarSection::Tags, true) => "Expand Tags",
            (SidebarSection::Tags, false) => "Collapse Tags",
            (SidebarSection::Collections, true) => "Expand Collections",
            (SidebarSection::Collections, false) => "Collapse Collections",
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            if !this.collapsed_sections.remove(&section) {
                this.collapsed_sections.insert(section);
            }
            cx.notify();
        }))
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
        let collection_rows = if self
            .collapsed_sections
            .contains(&SidebarSection::Collections)
        {
            Vec::new()
        } else {
            visible_collection_rows(&self.collection_forest, &self.expanded_collections)
        };
        let kinds = SIDEBAR_KIND_ORDER
            .into_iter()
            .enumerate()
            .filter(|(_, kind)| {
                if self.collapsed_sections.contains(&SidebarSection::Kinds) {
                    return false;
                }
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
            .text_size(rems(1.))
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
                h_flex()
                    .pt_4()
                    .pb_1()
                    .px_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_color(cx.theme().muted_foreground)
                            .child("Kinds"),
                    )
                    .child(self.sidebar_section_button(SidebarSection::Kinds, cx)),
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
                h_flex()
                    .pt_4()
                    .pb_1()
                    .px_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_color(cx.theme().muted_foreground)
                            .child("Tags"),
                    )
                    .child(self.sidebar_section_button(SidebarSection::Tags, cx)),
            )
            .children(self.tags.iter().enumerate().filter_map(|(index, tag)| {
                let count = self
                    .counts
                    .as_ref()
                    .map(|counts| counts.by_tag.get(&tag.id).copied().unwrap_or(0))
                    .unwrap_or(0);
                if self.collapsed_sections.contains(&SidebarSection::Tags)
                    || (count == 0 && selected_tag != Some(tag.id))
                {
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
                    )
                    .child(self.sidebar_section_button(SidebarSection::Collections, cx)),
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
                        .test_support()
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
                                let app_classify = app.clone();
                                let app_rules = app.clone();
                                let app_unnest = app.clone();
                                let app_delete = app.clone();
                                let mut menu = menu
                                    .min_w(px(0.))
                                    .item(compact_menu_item("Rename", move |_, window, cx| {
                                        let _ = app_rename.update(cx, |this, cx| {
                                            this.select_collection_row(id, tree_parent, window, cx);
                                            this.begin_collection_rename(id, window, cx);
                                        });
                                    }))
                                    .item(compact_menu_item(
                                        "Classification…",
                                        move |_, window, cx| {
                                            let _ = app_classify.update(cx, |this, cx| {
                                                this.open_collection_classification(id, window, cx);
                                            });
                                        },
                                    ))
                                    .item(compact_menu_item(
                                        "All classification rules…",
                                        move |_, window, cx| {
                                            let _ = app_rules.update(cx, |this, cx| {
                                                this.open_classification_rules(window, cx);
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
                                                cx.stop_propagation();
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
        let selected_count = self.table.read(cx).delegate().selected_ids.len();
        if selected_count > 1 {
            return render_panel(DetailView::Multiple(selected_count), None, Vec::new(), cx);
        }
        match &self.edit {
            Some(EditFetch::Ready(form)) => self.render_edit_panel(form, cx),
            Some(EditFetch::Loading(_)) => render_panel(DetailView::Loading, None, Vec::new(), cx),
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
                    let has_key = self
                        .table
                        .read(cx)
                        .delegate()
                        .settings
                        .openrouter_api_key
                        .is_some();
                    let jev_pending = self.jev.as_ref().is_some_and(|jev| {
                        Some(jev.resource_id) == self.selected && jev.status == JevStatus::Pending
                    });
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("classify-jev")
                                .ghost()
                                .icon(
                                    symbol("sparkles", AssetIconName::Sparkles)
                                        .with_size(px(16.)),
                                )
                                .loading(jev_pending)
                                .disabled(jev_pending || !has_key || favorited.is_none())
                                .tooltip(if has_key {
                                    "Auto-classify (⌘J)"
                                } else {
                                    "Set an OpenRouter API key in the command palette (⌘K) to auto-classify"
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.classify_with_jev(window, cx)
                                })),
                        )
                        .child(
                            Button::new("toggle-favorite")
                                .ghost()
                                .icon(star)
                                .disabled(
                                    self.selected
                                        .is_some_and(|id| self.favorite_pending.contains(&id)),
                                )
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
                        .child(
                            Button::new("add-attachment")
                                .ghost()
                                .icon(Icon::new(IconName::Plus).with_size(px(16.)))
                                .tooltip("Add attachment")
                                .accessibility_label("Add attachment")
                                .disabled(self.selected.is_some_and(|id| self.attachment_pending.contains(&id)))
                                .on_click(cx.listener(|this, _, window, cx| this.add_attachment(window, cx))),
                        )
                        .into_any_element()
                });
                // Read-only hashtags and memberships; editing happens on the
                // edit form or palette.
                let mut extra_rows: Vec<(SharedString, AnyElement)> = Vec::new();
                if let DetailView::Fields(detail) = self.detail_view() {
                    if !detail.tags.is_empty() {
                        extra_rows.push((
                            "Tags".into(),
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
                        ));
                    }
                    if !detail.attachments.is_empty()
                        || self.attachment_pending.contains(&detail.id)
                    {
                        extra_rows.push((
                            "Attachments".into(),
                            v_flex()
                                .gap_1()
                                .children(detail.attachments.iter().map(|attachment| {
                                    let path = attachment.file_path.clone();
                                    h_flex()
                                        .id(SharedString::from(format!(
                                            "attachment-{}",
                                            attachment.id
                                        )))
                                        .test_support()
                                        .aria_label(format!(
                                            "{} ({})",
                                            attachment.title, attachment.attachment_type
                                        ))
                                        .gap_2()
                                        .items_center()
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .child(attachment.title.clone())
                                                .child(
                                                    div()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child(attachment.attachment_type.clone()),
                                                ),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "open-attachment-{}",
                                                attachment.id
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .icon(Icon::new(IconName::ExternalLink).xsmall())
                                            .tooltip(format!("Open {}", attachment.title))
                                            .accessibility_label(format!(
                                                "Open {}",
                                                attachment.title
                                            ))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.open_attachment(path.clone(), window, cx)
                                            })),
                                        )
                                }))
                                .when(self.attachment_pending.contains(&detail.id), |list| {
                                    list.child("Adding attachment…")
                                })
                                .into_any_element(),
                        ));
                    }
                    if let Some((id, error)) = &self.attachment_error
                        && *id == detail.id
                    {
                        extra_rows.push((
                            "Attachment error".into(),
                            div()
                                .text_color(cx.theme().danger)
                                .child(error.clone())
                                .into_any_element(),
                        ));
                    }
                    // Detail/cache/in-flight projections retain membership IDs, not
                    // authoritative collection names or existence. Use the forest
                    // so sidebar renames and deletes apply to every projection.
                    let memberships: Vec<_> = flatten_collections(&self.collection_forest)
                        .into_iter()
                        .filter(|c| detail.collections.iter().any(|member| member.id == c.id))
                        .collect();
                    if !memberships.is_empty() {
                        extra_rows.push((
                            "Collections".into(),
                            h_flex()
                                .id("detail-collections")
                                .test_support()
                                .aria_label(
                                    memberships
                                        .iter()
                                        .map(|c| c.name.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", "),
                                )
                                .flex_wrap()
                                .gap_1()
                                .children(
                                    memberships
                                        .iter()
                                        .map(|c| membership_chip(c.name.as_str().to_owned(), cx)),
                                )
                                .into_any_element(),
                        ));
                    }
                    if let Some(row) = self.render_suggestions(detail, cx) {
                        extra_rows.push(("Suggested".into(), row));
                    }
                }
                render_panel(self.detail_view(), actions, extra_rows, cx)
            }
        }
    }

    /// The "Suggested" value cell: one chip per suggestion with an accept
    /// button, plus "Add all" when there are several. `None` when there is
    /// nothing to suggest.
    fn render_suggestions(
        &self,
        detail: &ResourceDetail,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let library = flatten_collections(&self.collection_forest);
        let jev = self
            .jev
            .as_ref()
            .filter(|jev| jev.resource_id == detail.id)
            .map(|jev| &jev.status);
        let picks = match jev {
            Some(JevStatus::Ready(picks)) => Some(picks.as_slice()),
            _ => None,
        };
        let suggestions = collection_suggestions(detail, &library, picks);
        let status = match jev {
            Some(JevStatus::Pending) => Some(
                div()
                    .id("jev-pending")
                    .test_support()
                    .text_color(cx.theme().muted_foreground)
                    .child("Auto-classifying…"),
            ),
            Some(JevStatus::Failed(error)) => Some(
                div()
                    .id("jev-error")
                    .test_support()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            ),
            Some(JevStatus::Ready(picks))
                if !picks.iter().any(|(id, _)| {
                    suggestions
                        .iter()
                        .any(|s| s.collection.id == *id && s.probability.is_some())
                }) =>
            {
                Some(
                    div()
                        .id("jev-empty")
                        .test_support()
                        .text_color(cx.theme().muted_foreground)
                        .child("Auto-classify found no new collections"),
                )
            }
            _ => None,
        };
        if suggestions.is_empty() && status.is_none() {
            return None;
        }
        let pending = self.suggestion_pending.contains(&detail.id);
        let all: Vec<CollectionId> = suggestions.iter().map(|s| s.collection.id).collect();
        Some(
            v_flex()
                .gap_1()
                .when(!suggestions.is_empty(), |cell| {
                    cell.child(
                        h_flex()
                            .id("detail-suggestions")
                            .test_support()
                            .flex_wrap()
                            .gap_1()
                            .children(suggestions.iter().enumerate().map(|(ix, suggestion)| {
                                let id = suggestion.collection.id;
                                let name = suggestion.collection.name.as_str().to_owned();
                                membership_chip(suggestion_label(suggestion), cx)
                                    .id(("suggestion", ix))
                                    .test_support()
                                    .child(
                                        Button::new(("accept-suggestion", ix))
                                            .ghost()
                                            .xsmall()
                                            .icon(Icon::new(IconName::Plus).xsmall())
                                            .disabled(pending)
                                            .tooltip(format!("Add to {name}"))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.accept_suggestions(vec![id], window, cx)
                                            })),
                                    )
                            }))
                            .when(all.len() >= 2, |row| {
                                row.child(
                                    Button::new("accept-all-suggestions")
                                        .ghost()
                                        .xsmall()
                                        .label("Add all")
                                        .disabled(pending)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.accept_suggestions(all.clone(), window, cx)
                                        })),
                                )
                            }),
                    )
                })
                .children(status)
                .into_any_element(),
        )
    }

    /// Add the selected resource to `ids` through the palette's persistence
    /// path, then refresh its detail so the accepted chips disappear.
    fn accept_suggestions(
        &mut self,
        ids: Vec<CollectionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // These writes replace the full resource. Do not race an editor's
        // snapshot or another write, and never open a draft the refresh drops.
        if self.has_inline(cx)
            || self.edit.is_some()
            || !self.favorite_pending.is_empty()
            || window.has_active_dialog(cx)
        {
            window.push_notification(
                "Finish the current resource edit before adding collections",
                cx,
            );
            return;
        }
        let Some(id) = self.selected else {
            return;
        };
        if ids.is_empty() || !self.suggestion_pending.insert(id) {
            return;
        }
        let worker = self.table.read(cx).delegate().worker().clone();
        cx.spawn_in(window, async move |app, window| {
            let request = PaletteRequest::EditCollections(id, ids, Vec::new());
            let result = apply_palette_request(&worker, &request).await;
            _ = app.update_in(window, |app, window, cx| {
                app.suggestion_pending.remove(&id);
                match result {
                    Ok(_) => app.refresh_after_write(id, window, cx),
                    Err(error) => window
                        .push_notification(format!("Could not add to collection: {error}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn wait_for_library_write(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.suggestion_pending.is_empty() || self.table.read(cx).delegate().reloading {
            window.push_notification("Wait for the library update to finish", cx);
            true
        } else {
            false
        }
    }

    /// A stored record changed: drop stale detail reads, refetch the panel
    /// when it shows `id`, and reload counts and the listing.
    fn refresh_after_write(&mut self, id: ResourceId, window: &mut Window, cx: &mut Context<Self>) {
        *self.detail_gens.entry(id).or_default() += 1;
        self.detail_inflight.remove(&id);
        self.detail_cache.invalidate(&id);
        if self.selected == Some(id) {
            self.detail = Some((id, DetailFetch::Loading));
            self.fetch_detail(id, window, cx);
        }
        self.refresh_counts(window, cx);
        let selected = self.selected;
        self.table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .reload_from_top(selected, None, window, cx)
        });
    }

    /// The form: Save/Cancel above, one labelled row per scalar, the error
    /// line under the buttons. Scrollable like the detail sidebar, so a
    /// short window still reaches every row. Parent is a searchable
    /// dropdown of library titles, not an id to type: users pick the
    /// containing work by name.
    fn render_edit_panel(&self, form: &EditForm, cx: &mut Context<Self>) -> Div {
        let saving =
            form.saving || form.author_creating || form.series_loading || form.series_creating;
        let error = form.error.clone();
        let danger = cx.theme().danger;

        v_flex()
            .h_full()
            .flex_shrink_0()
            .w_full()
            .bg(cx.theme().background)
            .text_size(FIELD_TEXT_SIZE)
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("cancel-edit")
                                    .ghost()
                                    .small()
                                    .label("Cancel")
                                    .disabled(form.saving)
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
                                    .disabled(saving)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_edit(window, cx)
                                    })),
                            ),
                    )
            .child(
                v_flex().id("edit-panel").test_support().flex_1().min_h_0().w_full().p_3().gap_2().overflow_y_scroll()
                    .capture_action(cx.listener(|app, action: &Enter, window, cx| {
                        if action.secondary { app.save_edit(window, cx); } else { cx.propagate(); }
                    }))
                    .when_some(error, |el, error| {
                        el.child(div().text_xs().text_color(danger).child(error))
                    })
                    .child(edit_field("Title", padded_input(&form.title).w_full()))
                    .child(edit_field("Favorite", Button::new("edit-favorite").ghost().small()
                        .label(if form.favorite { "★ Favorite" } else { "☆ Not favorite" }).selected(form.favorite).disabled(saving)
                        .on_click(cx.listener(|app, _, _, cx| {
                            if let Some(EditFetch::Ready(form)) = &mut app.edit { form.favorite = !form.favorite; cx.notify(); }
                        }))))
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
                    .child(edit_field("Creators",
                        v_flex().w_full().gap_2()
                            .children(form.creator_rows.iter().enumerate().map(|(ix, row)| {
                                let key = row.key;
                                v_flex().id(SharedString::from(format!("creator-row-{key}"))).gap_1().w_full()
                                    .child(div().child(row.name.clone()))
                                    .child(div().id(SharedString::from(format!("creator-role-{key}"))).test_support().w_full().child(
                                        Combobox::new(&row.role).small().placeholder("Role").search_placeholder("Search roles").disabled(saving).w_full()
                                    ))
                                    .child(div().id(SharedString::from(format!("creator-credit-{key}"))).test_support().w_full().child(
                                        Input::new(&row.credited_as).small().aria_label("Credited as").disabled(saving).w_full()
                                    ))
                                    .child(h_flex().gap_1()
                                        .child(Button::new(SharedString::from(format!("creator-up-{key}"))).ghost().xsmall().label("Move up").disabled(saving || ix == 0)
                                            .on_click(cx.listener(move |app, _, _, cx| app.move_creator(key, -1, cx))))
                                        .child(Button::new(SharedString::from(format!("creator-down-{key}"))).ghost().xsmall().label("Move down").disabled(saving || ix + 1 == form.creator_rows.len())
                                            .on_click(cx.listener(move |app, _, _, cx| app.move_creator(key, 1, cx))))
                                        .child(Button::new(SharedString::from(format!("creator-remove-{key}"))).ghost().xsmall().label("Remove").tooltip("Remove creator").disabled(saving)
                                            .on_click(cx.listener(move |app, _, _, cx| {
                                                if let Some(EditFetch::Ready(form)) = &mut app.edit { form.creator_rows.retain(|r| r.key != key); cx.notify(); }
                                            })))
                                    )
                            }))
                            .child(div().id("add-creator-person").test_support().w_full().child(
                                Combobox::new(&form.creators).small().placeholder("Select or create person").search_placeholder("Search people").disabled(saving).w_full()
                            ))
                            .child(Button::new("add-creator").ghost().small().label("Add creator").disabled(saving)
                                .on_click(cx.listener(|app, _, window, cx| app.add_creator(window, cx))))
                    ))
                    .child(edit_field(
                        "Publisher",
                        padded_input(&form.publisher).w_full(),
                    ))
                    .child(edit_field(
                        "Series",
                        div().id("edit-series").test_support().w_full().child(
                            Combobox::new(&form.series)
                                .small()
                                .placeholder(if form.series_loading {
                                    "Loading series…"
                                } else {
                                    "Select series"
                                })
                                .search_placeholder("Search or create series")
                                .cleanable(true)
                                .disabled(saving)
                                .w_full(),
                        ),
                    ))
                    .child(edit_field(
                        "Series no.",
                        padded_input(&form.series_number).w_full(),
                    ))
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Dates: YYYY, YYYY-MM, YYYY-MM-DD or RFC3339; ranges use /; append ? (uncertain) or ~ (approximate). Display text is optional."))
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Lists (identifiers and keywords): one entry per line, not commas."))
                    .children(BibliographicField::ALL.iter().map(|field| {
                        let input = &form.fields[field];
                        v_flex().gap_1().w_full()
                            .when(*field == BibliographicField::Citekey, |el| el.child(div().text_xs().child("Advanced metadata")))
                            .child(edit_field(field.label(), div().id(SharedString::from(format!("edit-field-{field:?}"))).test_support().w_full().child(input.render(*field, saving))))
                    }))
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

fn toggle_theme(settings: &crate::settings::ThemeSettings, cx: &mut App) {
    let mode = if cx.theme().is_dark() {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    crate::theme::apply(mode, settings, cx);
}

fn sidebar_kind_label(kind: ResourceKind) -> String {
    match kind {
        ResourceKind::JournalArticle => "Articles".into(),
        ResourceKind::Thesis => "Theses".into(),
        _ => format!("{}s", kind_label(kind)),
    }
}

// Sidebar reading order groups containers with their contents without changing
// the core enum order used by serialization, editors, and other surfaces.
const SIDEBAR_KIND_ORDER: [ResourceKind; 21] = [
    ResourceKind::Book,
    ResourceKind::BookSection,
    ResourceKind::Journal,
    ResourceKind::Issue,
    ResourceKind::JournalArticle,
    ResourceKind::ConferencePaper,
    ResourceKind::Preprint,
    ResourceKind::Thesis,
    ResourceKind::Report,
    ResourceKind::Manuscript,
    ResourceKind::MagazineArticle,
    ResourceKind::NewspaperArticle,
    ResourceKind::DictionaryEntry,
    ResourceKind::EncyclopediaArticle,
    ResourceKind::Webpage,
    ResourceKind::Blog,
    ResourceKind::BlogPost,
    ResourceKind::Podcast,
    ResourceKind::PodcastEpisode,
    ResourceKind::Dataset,
    ResourceKind::ComputerProgram,
];

const LIGHT_GRADIENT: (u32, u32) = (0xeae8e2, 0xd9dde5);
const DARK_GRADIENT: (u32, u32) = (0x292722, 0x262c35);
// CSS angles rotate clockwise from up: 225° points toward the bottom-left.
const WORKSPACE_GRADIENT_ANGLE: f32 = 225.;

/// Mirror the pinned GPUI renderer's aspect-corrected linear-gradient projection.
fn workspace_gradient_fraction(x: f32, y: f32, width: f32, height: f32) -> f32 {
    let width = width.max(1.);
    let height = height.max(1.);
    let (mut dy, mut dx) = (WORKSPACE_GRADIENT_ANGLE - 90.).to_radians().sin_cos();
    if width > height {
        dy *= height / width;
    } else {
        dx *= width / height;
    }
    let distance = ((x - width / 2.) * dx + (y - height / 2.) * dy) / dx.hypot(dy);
    let span = if dx.abs() > dy.abs() { width } else { height };
    (0.5 + distance / span).clamp(0., 1.)
}
const ISLAND_RADIUS: Pixels = px(16.);

fn mix_rgb(from: Rgba, to: Rgba, amount: f32) -> Rgba {
    Rgba {
        r: from.r + (to.r - from.r) * amount,
        g: from.g + (to.g - from.g) * amount,
        b: from.b + (to.b - from.b) * amount,
        a: 1.,
    }
}

/// A neutral wash from the top-right to the bottom-left end; a configured
/// secondary replaces that end exactly. The accent lives in [`WorkspaceGlow`].
/// Share the stops with the corner mask so its cutouts match the backdrop.
fn workspace_gradient_stops(dark: bool, secondary: Option<u32>) -> (Rgba, Rgba) {
    let (start, end) = if dark { DARK_GRADIENT } else { LIGHT_GRADIENT };
    (rgb(start), secondary.map(rgb).unwrap_or(rgb(end)))
}

/// Gradient fraction at which the accent glow has fully dissolved.
const GLOW_REACH: f32 = 0.4;

/// A vivid accent glow in the top-right corner that dissolves into the wash
/// well before the middle. GPUI gradients carry only two stops, so the glow is
/// a second translucent layer over the neutral → secondary wash.
#[derive(Clone, Copy)]
struct WorkspaceGlow {
    accent: Rgba,
    alpha: f32,
}

impl WorkspaceGlow {
    fn new(dark: bool, accent: Hsla) -> Self {
        Self {
            accent: accent.to_rgb(),
            alpha: if dark { 0.35 } else { 0.25 },
        }
    }

    fn background(self) -> Background {
        let accent = Hsla::from(self.accent);
        linear_gradient(
            WORKSPACE_GRADIENT_ANGLE,
            linear_color_stop(accent.opacity(self.alpha), 0.),
            linear_color_stop(accent.opacity(0.), GLOW_REACH),
        )
    }

    /// The opaque color the glow composited over the wash produces at `t`.
    fn over(self, stops: (Rgba, Rgba), t: f32) -> Rgba {
        let alpha = self.alpha * (1. - t / GLOW_REACH).max(0.);
        mix_rgb(mix_rgb(stops.0, stops.1, t), self.accent, alpha)
    }
}

/// GPUI's overflow clip is rectangular. Paint just the four regions outside
/// the island's round outline over its square-edged children, without insetting
/// the table or inspector. This canvas has no hitbox and cannot block clicks.
fn island_corner_mask(stops: (Rgba, Rgba), glow: WorkspaceGlow) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let radius = f32::from(ISLAND_RADIUS);
            let x0 = f32::from(bounds.origin.x);
            let x1 = x0 + f32::from(bounds.size.width);
            let y0 = f32::from(bounds.origin.y);
            let y1 = y0 + f32::from(bounds.size.height);
            let width = f32::from(window.viewport_size().width);
            let height = f32::from(window.viewport_size().height);
            for right in [false, true] {
                let x = if right { x1 } else { x0 };
                for bottom in [false, true] {
                    let y = if bottom { y1 } else { y0 };
                    let t = workspace_gradient_fraction(x, y, width, height);
                    let color = Hsla::from(glow.over(stops, t));
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
        let theme_preferences = if self.theme_editor_open {
            self.theme_editor.read(cx).draft()
        } else {
            &settings.theme
        };
        let secondary = theme_preferences.color(
            if cx.theme().is_dark() {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            ThemeColorTarget::Secondary,
        );
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

        let stops = workspace_gradient_stops(cx.theme().is_dark(), secondary);
        let background = linear_gradient(
            WORKSPACE_GRADIENT_ANGLE,
            linear_color_stop(Hsla::from(stops.0), 0.),
            linear_color_stop(Hsla::from(stops.1), 1.),
        );
        let glow = WorkspaceGlow::new(cx.theme().is_dark(), cx.theme().primary);

        v_flex()
            .size_full()
            .bg(background)
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
            .on_action(cx.listener(|this, _: &ImportBiblatexFile, window, cx| {
                this.import_biblatex(window, cx);
            }))
            .on_action(cx.listener(|_, _: &OpenDocumentation, window, cx| {
                if let Err(error) = DocumentationWindow::open(cx) {
                    window.push_notification(format!("Could not open documentation: {error}"), cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ExtendResourceSelectionUp, _, cx| {
                this.extend_resource_selection(-1, cx);
            }))
            .on_action(cx.listener(|this, _: &ExtendResourceSelectionDown, _, cx| {
                this.extend_resource_selection(1, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectAllResources, window, cx| {
                if this.has_inline(cx) || this.edit.is_some() {
                    return;
                }
                this.table.update(cx, |table, cx| {
                    let delegate = table.delegate_mut();
                    delegate.selected_ids = delegate.rows.iter().map(|r| r.id).collect();
                    delegate.selection_anchor = delegate.rows.first().map(|r| r.id);
                    delegate.preserve_selection = !delegate.rows.is_empty();
                    if !delegate.rows.is_empty() {
                        table.set_selected_row(0, cx);
                    }
                });
                this.focus_table(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpenCommandPalette, window, cx| {
                this.open_command_palette(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleDetailPanel, _window, cx| {
                this.toggle_detail_panel(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleLeftPanel, _window, cx| {
                this.toggle_left_panel(cx);
            }))
            .on_action(cx.listener(|this, _: &CycleColumnSort, window, cx| {
                this.cycle_column_sort(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ExpandResource, window, cx| {
                this.disclose_selected_resource(true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CollapseResource, window, cx| {
                this.disclose_selected_resource(false, window, cx);
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
            .on_action(cx.listener(|this, _: &ToggleFavorite, window, cx| {
                this.toggle_favorite(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ClassifyWithJev, window, cx| {
                this.classify_with_jev(window, cx);
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
            // First child paints beneath the toolbar and body; it has no hitbox.
            .child(div().absolute().inset_0().bg(glow.background()))
            .child(
                // The toolbar doubles as the window's title bar; reserve space
                // for the native traffic lights without adding a bottom rule.
                TitleBar::new()
                    .h(px(40.))
                    .pl_0()
                    .border_0()
                    // Share the viewport backdrop instead of restarting the gradient here.
                    .bg(transparent_black())
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
                                                .text_size(rems(1.))
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
                                        Button::new("add-resource")
                                            .ghost()
                                            .icon(Icon::new(IconName::Plus).with_size(px(12.)))
                                            .loading(self.import_state != ImportState::Idle)
                                            .disabled(self.import_state != ImportState::Idle)
                                            .tooltip(match self.import_state {
                                                ImportState::Idle => "Add resource",
                                                ImportState::Choosing => "Choosing file…",
                                                ImportState::Importing => "Importing…",
                                            })
                                            .dropdown_menu_with_anchor(Anchor::TopRight, {
                                                let app = cx.entity().downgrade();
                                                move |menu, window, _cx| {
                                                    let Some(app) = app.upgrade() else {
                                                        return menu;
                                                    };
                                                    menu.item(
                                                        PopupMenuItem::new("Add by ISBN or DOI…")
                                                            .on_click(window.listener_for(
                                                                &app,
                                                                |this, _, window, cx| {
                                                                    if let Some(palette) = this
                                                                        .open_palette(window, cx)
                                                                    {
                                                                        palette.update(
                                                                        cx,
                                                                        |palette, cx| {
                                                                            palette.open_identifier(
                                                                                window, cx,
                                                                            )
                                                                        },
                                                                    );
                                                                    }
                                                                },
                                                            )),
                                                    )
                                                    .item(
                                                        PopupMenuItem::new("Import file…")
                                                            .on_click(window.listener_for(
                                                                &app,
                                                                |this, _, window, cx| {
                                                                    this.import_biblatex(window, cx)
                                                                },
                                                            )),
                                                    )
                                                }
                                            }),
                                    )
                                    .child(
                                        Button::new("open-documentation")
                                            .ghost()
                                            .small()
                                            .icon(symbol(
                                                "questionmark.circle",
                                                AssetIconName::CircleQuestionMark,
                                            ))
                                            .accessibility_label("Open documentation")
                                            .tooltip("Open documentation (F1)")
                                            .on_click(cx.listener(|_, _, window, cx| {
                                                cx.stop_propagation();
                                                if let Err(error) = DocumentationWindow::open(cx) {
                                                    window.push_notification(
                                                        format!(
                                                            "Could not open documentation: {error}"
                                                        ),
                                                        cx,
                                                    );
                                                }
                                            })),
                                    )
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
                                            .child(
                                                Popover::new("theme-colors-popover")
                                                    .anchor(Anchor::TopRight)
                                                    .open(self.theme_editor_open)
                                                    .track_focus(
                                                        &self.theme_editor.focus_handle(cx),
                                                    )
                                                    .on_open_change(cx.listener(
                                                        |this, open: &bool, window, cx| {
                                                            this.set_theme_editor_open(
                                                                *open, window, cx,
                                                            );
                                                        },
                                                    ))
                                                    .trigger(
                                                        Button::new("set-accent-color")
                                                            .ghost()
                                                            .label("●")
                                                            .text_color(cx.theme().primary)
                                                            .tooltip("Theme colors")
                                                            .disabled(
                                                                self.import_state
                                                                    != ImportState::Idle
                                                                    || self.edit.is_some()
                                                                    || self.has_inline(cx),
                                                            ),
                                                    )
                                                    .child(self.theme_editor.clone()),
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
                                                    .on_click(cx.listener(
                                                        |this, _, _window, cx| {
                                                            cx.stop_propagation();
                                                            let settings = this
                                                                .table
                                                                .read(cx)
                                                                .delegate()
                                                                .settings
                                                                .theme
                                                                .clone();
                                                            toggle_theme(&settings, cx);
                                                        },
                                                    )),
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
                                .id("library-grid-viewport")
                                .test_support()
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
                                    .child(island_corner_mask(stops, glow))
                                    .into_any_element()
                            } else {
                                wrap.child(table)
                                    .child(island_corner_mask(stops, glow))
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
                                .child(island_corner_mask(stops, glow))
                                .when(self.detail_open, |el| {
                                    el.child(self.sidebar_resize_handle(false, sidebar_width, cx))
                                }),
                        )
                    }),
            )
            .when_some(error, |el, error| {
                el.child(
                    h_flex()
                        .gap_2()
                        .flex_shrink_0()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(danger)
                        .child(error)
                        .child(
                            Button::new("retry-resource-grid")
                                .ghost()
                                .xsmall()
                                .label("Retry")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let selected = this.selected;
                                    this.table.update(cx, |table, cx| {
                                        table
                                            .delegate_mut()
                                            .reload_from_top(selected, None, window, cx);
                                    });
                                })),
                        ),
                )
            })
    }
}

#[derive(Debug, Clone, PartialEq)]
enum JevStatus {
    Pending,
    Ready(crate::jev::JevSuggestions),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
struct JevState {
    resource_id: ResourceId,
    status: JevStatus,
}

/// Only fields that can change the classification input; collection/tag,
/// favorite, and other bibliographic changes do not discard valid predictions.
fn classification_input_changed(
    before: &ibex_core::NewResource,
    after: &ibex_core::NewResource,
) -> bool {
    before.title != after.title
        || before.kind != after.kind
        || before.creators != after.creators
        || before.parent_id != after.parent_id
        || before.abstract_text != after.abstract_text
        || before.keywords != after.keywords
}

/// One collection the detail panel offers to add the resource to.
#[derive(Debug, Clone, PartialEq)]
struct CollectionSuggestion {
    collection: Collection,
    /// Jev's probability; `None` for a keyword-rule match.
    probability: Option<f32>,
}

/// Suggestions for `detail`: Jev's picks first (already ranked), then
/// keyword-rule matches in library order. Current memberships, unknown ids,
/// and duplicates are dropped. Pure so ordering is unit-testable.
fn collection_suggestions(
    detail: &ResourceDetail,
    library: &[Collection],
    jev: Option<&[(CollectionId, f32)]>,
) -> Vec<CollectionSuggestion> {
    let mut seen: HashSet<CollectionId> = detail.collections.iter().map(|c| c.id).collect();
    let mut out = Vec::new();
    for &(id, probability) in jev.unwrap_or_default() {
        if let Some(collection) = library.iter().find(|c| c.id == id)
            && seen.insert(id)
        {
            out.push(CollectionSuggestion {
                collection: collection.clone(),
                probability: Some(probability),
            });
        }
    }
    for id in keyword_suggestions(detail, library) {
        if let Some(collection) = library.iter().find(|c| c.id == id)
            && seen.insert(id)
        {
            out.push(CollectionSuggestion {
                collection: collection.clone(),
                probability: None,
            });
        }
    }
    out
}

fn suggestion_label(suggestion: &CollectionSuggestion) -> String {
    match suggestion.probability {
        Some(p) => format!(
            "{} · {}%",
            suggestion.collection.name.as_str(),
            (p * 100.).round() as u32
        ),
        None => suggestion.collection.name.as_str().to_owned(),
    }
}
struct PaletteBatchResult {
    updated: Vec<ResourceId>,
    failed: Vec<(ResourceId, String)>,
}

async fn apply_palette_batch(worker: &DbWorker, request: &PaletteRequest) -> PaletteBatchResult {
    let mut result = PaletteBatchResult {
        updated: Vec::new(),
        failed: Vec::new(),
    };
    let mut seen = HashSet::new();
    for id in request
        .resource_ids()
        .into_iter()
        .filter(|id| seen.insert(*id))
    {
        let single = match request {
            PaletteRequest::EditTags(_, added, removed)
            | PaletteRequest::EditTagsBulk(_, added, removed) => {
                PaletteRequest::EditTags(id, added.clone(), removed.clone())
            }
            PaletteRequest::EditCollections(_, added, removed)
            | PaletteRequest::EditCollectionsBulk(_, added, removed) => {
                PaletteRequest::EditCollections(id, added.clone(), removed.clone())
            }
        };
        match apply_palette_request(worker, &single).await {
            Ok(_) => result.updated.push(id),
            Err(error) => result.failed.push((id, error)),
        }
    }
    result
}
async fn apply_palette_request(
    worker: &DbWorker,
    request: &crate::command_palette::PaletteRequest,
) -> Result<Resource, String> {
    use crate::command_palette::PaletteRequest;
    // Read at Apply time, not palette-open time, to preserve intervening changes.
    let record = worker.resource(request.resource_id()).await?;
    let mut new = record.to_new();
    match request {
        PaletteRequest::EditTags(_, ids, removed)
        | PaletteRequest::EditTagsBulk(_, ids, removed) => {
            new.tag_ids.retain(|id| !removed.contains(id));
            for id in ids {
                if !new.tag_ids.contains(id) {
                    new.tag_ids.push(*id);
                }
            }
        }
        PaletteRequest::EditCollections(_, ids, removed)
        | PaletteRequest::EditCollectionsBulk(_, ids, removed) => {
            new.collection_ids.retain(|id| !removed.contains(id));
            for id in ids {
                if !new.collection_ids.contains(id) {
                    new.collection_ids.push(*id);
                }
            }
        }
    }
    worker.update_resource(record.id, new).await
}

#[cfg(test)]
mod palette_persistence_tests {
    use super::{LibraryApp, LibraryDelegate, OpenCommandPalette, Settings, apply_palette_request};
    use crate::command_palette::PaletteRequest;
    use crate::db::DbWorker;
    use gpui_kit::component::{WindowExt as _, table::TableState};
    use gpui_kit::{AppContext as _, Focusable as _, KeyBinding};
    use ibex_core::{NonEmptyStr, ResourceId};

    #[gpui_kit::test]
    fn shift_arrows_extend_shrink_and_clamp_resource_selection(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::test::TestWindowExt as _;
        let dir = tempfile::tempdir().unwrap();
        cx.executor().allow_parking();
        let store = ibex_app::testing::MemoryStore::new();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ibex_app::seed::seed_demo_library(&store, 8))
            .unwrap();
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(store)
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let first = worker
            .list_grid_page_blocking(
                ibex_core::ResourceGridParent::Roots,
                ibex_core::ResourceQuery::newest_first(),
                0,
                50,
            )
            .unwrap();
        let ids: Vec<_> = first.rows.iter().map(|r| r.summary.id).collect();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                let table = cx.new(|cx| {
                    TableState::new(
                        LibraryDelegate::new(worker, first, Settings::default()),
                        window,
                        cx,
                    )
                });
                cx.new(|cx| LibraryApp::new(table, dir.path().to_path_buf(), window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| app.focus_table(window, cx));
            window.render_frame(cx);
            window.press("shift-down", cx);
        })
        .unwrap();
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            assert_eq!(
                app.table.read(cx).delegate().selected_ids,
                [ids[0]]
                    .into_iter()
                    .collect::<std::collections::HashSet<_>>()
            )
        });
        // The range grows from the original row; reversing removes its trailing edge.
        for (key, expected) in [
            ("shift-down", vec![ids[0], ids[1]]),
            ("shift-down", vec![ids[0], ids[1], ids[2]]),
            ("shift-up", vec![ids[0], ids[1]]),
            ("shift-up", vec![ids[0]]),
            ("shift-up", vec![ids[0]]),
            ("down", vec![ids[1]]),
            ("shift-up", vec![ids[0], ids[1]]),
            ("shift-down", vec![ids[1]]),
            ("shift-down", vec![ids[1], ids[2]]),
        ] {
            let count = expected.len();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.press(key, cx);
            })
            .unwrap();
            cx.run_until_parked();
            app.read_with(cx, |app, cx| {
                assert_eq!(
                    app.table.read(cx).delegate().selected_ids,
                    expected
                        .into_iter()
                        .collect::<std::collections::HashSet<_>>(),
                    "after {key}"
                )
            });
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                if count > 1 {
                    assert_eq!(
                        window.find("detail-selection-count").label(),
                        Some(format!("{count} resources selected").as_str())
                    );
                    assert!(window.try_find("edit-resource").is_none());
                } else {
                    assert!(window.try_find("detail-selection-count").is_none());
                }
            })
            .unwrap();
        }
        cx.update_window(handle, |_, _, cx| {
            app.read(cx)
                .table
                .clone()
                .update(cx, |table, cx| table.set_selected_row(ids.len() - 1, cx));
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("shift-down", cx);
        })
        .unwrap();
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            assert_eq!(
                app.table.read(cx).delegate().selected_ids,
                [*ids.last().unwrap()]
                    .into_iter()
                    .collect::<std::collections::HashSet<_>>()
            )
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("shift-up", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("detail-selection-count").is_some());
            window.dispatch_action(Box::new(super::ToggleEditMode), cx);
        })
        .unwrap();
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(
                app.edit.is_none(),
                "multi-selection must not open an invisible editor"
            )
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("tab", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(
                app.read(cx)
                    .table
                    .read(cx)
                    .delegate()
                    .selected_ids
                    .is_empty()
            );
            window.render_frame(cx);
            assert!(
                window.try_find("detail-selection-count").is_none(),
                "column selection must redraw the detail panel"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn resource_modified_clicks_and_select_all(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::Modifiers;
        use gpui_kit::test::TestWindowExt as _;
        let dir = tempfile::tempdir().unwrap();
        cx.executor().allow_parking();
        let store = ibex_app::testing::MemoryStore::new();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ibex_app::seed::seed_demo_library(&store, 8))
            .unwrap();
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(store)
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let first = worker
            .list_grid_page_blocking(
                ibex_core::ResourceGridParent::Roots,
                ibex_core::ResourceQuery::newest_first(),
                0,
                50,
            )
            .unwrap();
        let ids: Vec<_> = first.rows.iter().map(|r| r.summary.id).collect();
        assert!(ids.len() >= 3);
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                let table = cx.new(|cx| {
                    TableState::new(
                        LibraryDelegate::new(worker, first, Settings::default()),
                        window,
                        cx,
                    )
                    .select_on_modifier_click(false)
                });
                cx.new(|cx| LibraryApp::new(table, dir.path().to_path_buf(), window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| app.focus_table(window, cx));
            window.render_frame(cx);
            window.click(format!("resource-row-{}", ids[0]), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click_with_modifiers(
                format!("resource-row-{}", ids[2]),
                Modifiers {
                    platform: true,
                    ..Default::default()
                },
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(app.read(cx).table.read(cx).delegate().selected_ids.len(), 2);
            window.render_frame(cx);
            window.click_with_modifiers(
                format!("resource-row-{}", ids[2]),
                Modifiers {
                    platform: true,
                    ..Default::default()
                },
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(app.read(cx).table.read(cx).delegate().selected_ids.len(), 1);
            window.render_frame(cx);
            window.click_with_modifiers(
                format!("resource-row-{}", ids[2]),
                Modifiers {
                    shift: true,
                    ..Default::default()
                },
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(app.read(cx).table.read(cx).delegate().selected_ids.len(), 3);
            window.render_frame(cx);
            window.press("cmd-a", cx);
        })
        .unwrap();
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            assert_eq!(app.table.read(cx).delegate().selected_ids.len(), ids.len())
        });
        cx.update_window(handle, |_, _, cx| {
            let table = app.read(cx).table.clone();
            table.update(cx, |table, cx| {
                super::restore_grid_selection(table, Some(ids[0]), None, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            assert_eq!(
                app.table.read(cx).delegate().selected_ids.len(),
                ids.len(),
                "structural restoration must preserve bulk selection"
            )
        });
    }
    #[tokio::test]
    async fn bulk_palette_preserves_memberships_and_reports_partial_failure() {
        let store = ibex_app::testing::MemoryStore::new();
        ibex_app::seed::seed_demo_library(&store, 2).await.unwrap();
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(store)
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let ids: Vec<_> = worker
            .list_page(None, 2)
            .await
            .unwrap()
            .rows
            .iter()
            .map(|r| r.id)
            .collect();
        let before = worker.resource(ids[1]).await.unwrap();
        let tag = worker
            .create_tag(NonEmptyStr::try_new("bulk tag").unwrap())
            .await
            .unwrap();
        let missing = ResourceId::generate();
        let request = PaletteRequest::EditTagsBulk(
            vec![ids[0], missing, ids[1], ids[0]],
            vec![tag.id],
            vec![],
        );
        let result = super::apply_palette_batch(&worker, &request).await;
        assert_eq!(result.updated.len(), 2);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].0, missing);
        let after = worker.resource(ids[1]).await.unwrap();
        assert!(after.tag_ids.contains(&tag.id));
        assert!(before.tag_ids.iter().all(|id| after.tag_ids.contains(id)));
        assert_eq!(before.collection_ids, after.collection_ids);
        assert_eq!(before.title, after.title);
    }
    #[tokio::test]
    async fn palette_adds_tags_without_replacing_existing_memberships_or_fields() {
        let store = ibex_app::testing::MemoryStore::new();
        ibex_app::seed::seed_demo_library(&store, 2).await.unwrap();
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(store)
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let id = worker.list_page(None, 1).await.unwrap().rows[0].id;
        let before = worker.resource(id).await.unwrap();
        let tag = worker
            .create_tag(NonEmptyStr::try_new("palette tag").unwrap())
            .await
            .unwrap();
        let request = PaletteRequest::EditTags(id, vec![tag.id, tag.id], vec![]);
        apply_palette_request(&worker, &request).await.unwrap();
        let after = worker.resource(id).await.unwrap();
        assert!(after.tag_ids.contains(&tag.id));
        assert_eq!(after.tag_ids.iter().filter(|&&id| id == tag.id).count(), 1);
        assert!(before.tag_ids.iter().all(|id| after.tag_ids.contains(id)));
        assert_eq!(after.title, before.title);
        assert_eq!(after.collection_ids, before.collection_ids);
    }

    #[tokio::test]
    async fn palette_adds_collections_idempotently_and_preserves_later_edits() {
        let store = ibex_app::testing::MemoryStore::new();
        ibex_app::seed::seed_demo_library(&store, 2).await.unwrap();
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(store)
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let id = worker.list_page(None, 1).await.unwrap().rows[0].id;
        let before = worker.resource(id).await.unwrap();
        let collection = worker
            .create_collection(NonEmptyStr::try_new("Palette collection").unwrap(), None)
            .await
            .unwrap();
        let request =
            PaletteRequest::EditCollections(id, vec![collection.id, collection.id], vec![]);
        let mut edited = before.to_new();
        edited.title = NonEmptyStr::try_new("Changed after palette opened").unwrap();
        worker.update_resource(id, edited).await.unwrap();
        apply_palette_request(&worker, &request).await.unwrap();
        apply_palette_request(&worker, &request).await.unwrap();
        let after = worker.resource(id).await.unwrap();
        assert_eq!(after.title.as_str(), "Changed after palette opened");
        assert_eq!(after.tag_ids, before.tag_ids);
        assert!(
            before
                .collection_ids
                .iter()
                .all(|id| after.collection_ids.contains(id))
        );
        assert_eq!(
            after
                .collection_ids
                .iter()
                .filter(|&&id| id == collection.id)
                .count(),
            1
        );
        let err = apply_palette_request(
            &worker,
            &PaletteRequest::EditTags(ResourceId::generate(), vec![], vec![]),
        )
        .await
        .unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[tokio::test]
    async fn palette_removes_only_unchecked_memberships_and_keeps_intervening_additions() {
        let store = ibex_app::testing::MemoryStore::new();
        ibex_app::seed::seed_demo_library(&store, 2).await.unwrap();
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(store)
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let id = worker.list_page(None, 1).await.unwrap().rows[0].id;
        let mut tags = Vec::new();
        let mut collections = Vec::new();
        for label in ["Remove", "Keep", "Concurrent", "Add"] {
            tags.push(
                worker
                    .create_tag(NonEmptyStr::try_new(format!("palette {label}")).unwrap())
                    .await
                    .unwrap()
                    .id,
            );
            collections.push(
                worker
                    .create_collection(
                        NonEmptyStr::try_new(format!("palette {label}")).unwrap(),
                        None,
                    )
                    .await
                    .unwrap()
                    .id,
            );
        }
        let mut current = worker.resource(id).await.unwrap().to_new();
        current.title = NonEmptyStr::try_new("Intervening title").unwrap();
        current.tag_ids = tags[..3].to_vec();
        current.collection_ids = collections[..3].to_vec();
        worker.update_resource(id, current).await.unwrap();
        let edits = [
            PaletteRequest::EditTags(id, vec![tags[3]], vec![tags[0]]),
            PaletteRequest::EditCollections(id, vec![collections[3]], vec![collections[0]]),
        ];
        for edit in &edits {
            apply_palette_request(&worker, edit).await.unwrap();
            apply_palette_request(&worker, edit).await.unwrap();
        }
        let after = worker.resource(id).await.unwrap();
        assert_eq!(after.title.as_str(), "Intervening title");
        assert_eq!(after.tag_ids, tags[1..]);
        assert_eq!(after.collection_ids, collections[1..]);
        // Unchecking the final memberships must allow clearing the sets entirely.
        apply_palette_request(
            &worker,
            &PaletteRequest::EditTags(id, vec![], after.tag_ids.clone()),
        )
        .await
        .unwrap();
        apply_palette_request(
            &worker,
            &PaletteRequest::EditCollections(id, vec![], after.collection_ids.clone()),
        )
        .await
        .unwrap();
        let cleared = worker.resource(id).await.unwrap();
        assert!(cleared.tag_ids.is_empty());
        assert!(cleared.collection_ids.is_empty());
    }

    #[gpui_kit::test]
    fn command_k_opens_modal_and_escape_restores_library_focus(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::test::TestWindowExt as _;
        let dir = tempfile::tempdir().unwrap();
        cx.executor().allow_parking();
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(ibex_app::testing::MemoryStore::new())
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let first = worker
            .list_grid_page_blocking(
                ibex_core::ResourceGridParent::Roots,
                ibex_core::ResourceQuery {
                    sort: None,
                    filter: ibex_core::ResourceFilter::none(),
                },
                0,
                50,
            )
            .unwrap();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.bind_keys([KeyBinding::new(
                "cmd-k",
                OpenCommandPalette,
                Some("LibraryApp"),
            )]);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                let table = cx.new(|cx| {
                    TableState::new(
                        LibraryDelegate::new(worker, first, Settings::default()),
                        window,
                        cx,
                    )
                });
                cx.new(|cx| LibraryApp::new(table, dir.path().to_path_buf(), window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| app.focus_table(window, cx));
            window.render_frame(cx);
            window.press("cmd-k", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(window.has_active_dialog(cx));
            window.render_frame(cx);
            assert!(window.try_find("palette-back").is_some());
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(!window.has_active_dialog(cx));
            assert!(app.read(cx).table.focus_handle(cx).is_focused(window));
            assert!(!app.read(cx).table.read(cx).delegate().filter.is_active());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn workspace_paints_one_continuous_gradient_across_toolbar_and_body(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::component::ActiveTheme as _;
        use gpui_kit::test::TestWindowExt as _;
        use gpui_kit::{Hsla, linear_color_stop, linear_gradient};
        let dir = tempfile::tempdir().unwrap();
        let (handle, _) = open_theme_fixture(cx, &dir);
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let stops = super::workspace_gradient_stops(false, None);
            let background = linear_gradient(
                super::WORKSPACE_GRADIENT_ANGLE,
                linear_color_stop(Hsla::from(stops.0), 0.),
                linear_color_stop(Hsla::from(stops.1), 1.),
            );
            let glow = super::WorkspaceGlow::new(false, cx.theme().primary).background();
            let quads = window.painted_quads();
            let count = |target| {
                quads
                    .iter()
                    .filter(|quad| quad.background == target)
                    .count()
            };
            assert_eq!(
                count(background),
                1,
                "Only the viewport may paint the workspace gradient; the toolbar must reveal it"
            );
            assert_eq!(count(glow), 1, "One accent glow spans toolbar and body");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn add_menu_opens_identifier_entry(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::test::TestWindowExt as _;
        let dir = tempfile::tempdir().unwrap();
        let (handle, _) = open_theme_fixture(cx, &dir);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("add-resource", cx);
            assert!(window.try_find("popup-menu").is_some());
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.has_active_dialog(cx));
            assert!(window.try_find("palette-identifier").is_some());
            assert!(window.try_find("popup-menu").is_none());
            window.input("9780306406157", cx);
            assert!(window.try_find("palette-identifier").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn add_menu_dismissal_and_busy_state(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::component::ThemeMode;
        use gpui_kit::test::TestWindowExt as _;
        let dir = tempfile::tempdir().unwrap();
        let (handle, app) = open_theme_fixture(cx, &dir);
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            cx.update_window(handle, |_, window, cx| {
                crate::theme::apply(mode, &crate::settings::ThemeSettings::default(), cx);
                window.render_frame(cx);
                window.click("add-resource", cx);
                assert!(window.try_find("popup-menu").is_some());
                window.press("escape", cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find("popup-menu").is_none());
                assert!(!window.has_active_dialog(cx));
                window.click("add-resource", cx);
                assert!(window.try_find("popup-menu").is_some());
                window.click("library-grid-viewport", cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find("popup-menu").is_none());
                for state in [super::ImportState::Choosing, super::ImportState::Importing] {
                    app.update(cx, |app, cx| {
                        app.import_state = state;
                        cx.notify();
                    });
                    window.render_frame(cx);
                    window.click("add-resource", cx);
                    assert!(window.try_find("popup-menu").is_none());
                }
                app.update(cx, |app, cx| {
                    app.import_state = super::ImportState::Idle;
                    cx.notify();
                });
            })
            .unwrap();
        }
        assert!(!cx.did_prompt_for_paths());
    }

    #[gpui_kit::test]
    fn documentation_toolbar_and_shortcut_share_one_window(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::test::TestWindowExt as _;

        let dir = tempfile::tempdir().unwrap();
        let (handle, app) = open_theme_fixture(cx, &dir);
        cx.update(crate::bind_library_keys);
        cx.update_window(handle, |_, window, cx| {
            let focus = app.read(cx).table.focus_handle(cx);
            window.focus(&focus, cx);
            window.press("f1", cx);
        })
        .unwrap();
        assert_eq!(cx.windows().len(), 2);
        let documentation = cx
            .windows()
            .into_iter()
            .find(|other| *other != handle)
            .unwrap();
        cx.update_window(documentation, |_, window, _| window.remove_window())
            .unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find("open-documentation").label(),
                Some("Open documentation")
            );
            window.click("open-documentation", cx);
        })
        .unwrap();
        let windows = cx.windows();
        assert_eq!(windows.len(), 2);
        cx.update_window(handle, |_, window, cx| {
            let focus = app.read(cx).table.focus_handle(cx);
            window.focus(&focus, cx);
            window.press("f1", cx);
        })
        .unwrap();
        assert_eq!(cx.windows(), windows);
    }

    fn open_theme_fixture(
        cx: &mut gpui_kit::TestAppContext,
        dir: &tempfile::TempDir,
    ) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<LibraryApp>) {
        use gpui_kit::component::ThemeMode;
        cx.executor().allow_parking();
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(ibex_app::testing::MemoryStore::new())
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let first = worker
            .list_grid_page_blocking(
                ibex_core::ResourceGridParent::Roots,
                ibex_core::ResourceQuery {
                    sort: None,
                    filter: ibex_core::ResourceFilter::none(),
                },
                0,
                50,
            )
            .unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply(
                ThemeMode::Light,
                &crate::settings::ThemeSettings::default(),
                cx,
            );
            cx.bind_keys([KeyBinding::new(
                "cmd-k",
                OpenCommandPalette,
                Some("LibraryApp"),
            )]);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                let table = cx.new(|cx| {
                    TableState::new(
                        LibraryDelegate::new(worker, first, Settings::default()),
                        window,
                        cx,
                    )
                });
                cx.new(|cx| LibraryApp::new(table, dir.path().to_path_buf(), window, cx))
            })
            .unwrap()
        })
    }

    #[gpui_kit::test]
    fn theme_popover_dismissal_discards_preview_without_writing(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::component::ActiveTheme as _;
        use gpui_kit::test::TestWindowExt as _;
        use gpui_kit::{Hsla, rgb};
        for dismissal in ["escape", "cancel", "outside"] {
            let dir = tempfile::tempdir().unwrap();
            let (handle, app) = open_theme_fixture(cx, &dir);
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click("set-accent-color", cx);
            })
            .unwrap();
            cx.run_until_parked();
            app.read_with(cx, |app, cx| {
                assert_eq!(
                    app.theme_editor.read(cx).draft(),
                    &crate::settings::ThemeSettings::default()
                );
            });
            cx.update_window(handle, |_, window, cx| {
                window.click("theme-swatch-Blue-500", cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                assert_eq!(cx.theme().primary, Hsla::from(rgb(0x3b82f6)));
                match dismissal {
                    "escape" => window.press("escape", cx),
                    "cancel" => window.click("theme-color-cancel", cx),
                    _ => window.click("toggle-theme", cx),
                }
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                assert!(!app.read(cx).theme_editor_open);
                assert_eq!(
                    cx.theme().primary,
                    Hsla::from(rgb(if dismissal == "outside" {
                        0x0a84ff
                    } else {
                        0x0064ff
                    }))
                );
                assert_eq!(
                    app.read(cx).table.read(cx).delegate().settings,
                    Settings::default()
                );
                window.render_frame(cx);
                assert!(window.try_find("theme-color-editor").is_none());
            })
            .unwrap();
            assert!(!dir.path().join("config.toml").exists());
        }
    }

    #[gpui_kit::test]
    fn theme_popover_rejects_invalid_hex_and_keeps_failed_saves_retryable(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::component::ActiveTheme as _;
        use gpui_kit::test::TestWindowExt as _;
        use gpui_kit::{Hsla, rgb};
        let dir = tempfile::tempdir().unwrap();
        let (handle, app) = open_theme_fixture(cx, &dir);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("set-accent-color", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.click("theme-color-hex", cx);
            window.press("cmd-a", cx);
            window.input("invalid", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.click("theme-secondary-tab", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.click("theme-swatch-Cyan-400", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.click("theme-color-save", cx);
            assert!(app.read(cx).theme_editor_open);
            window.click("theme-accent-tab", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!dir.path().join("config.toml").exists());
        let blocked = dir.path().join("not-a-directory");
        std::fs::write(&blocked, "blocked").unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.click("theme-swatch-Orange-500", cx);
            app.update(cx, |app, _| app.settings_dir = blocked);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.click("theme-color-save", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(app.read(cx).theme_editor_open);
            assert_eq!(
                app.read(cx).table.read(cx).delegate().settings,
                Settings::default()
            );
            window.render_frame(cx);
            assert!(window.try_find("theme-color-error").is_some());
            app.update(cx, |app, _| app.settings_dir = dir.path().to_path_buf());
            window.click("theme-color-save", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let saved = Settings::load(dir.path());
        assert_eq!(saved.theme.light.accent.as_deref(), Some("#F97316"));
        assert_eq!(saved.theme.light.secondary.as_deref(), Some("#22D3EE"));
        cx.update_window(handle, |_, _, cx| {
            assert!(!app.read(cx).theme_editor_open);
            assert_eq!(cx.theme().primary, Hsla::from(rgb(0xf97316)));
        })
        .unwrap();
    }
    #[gpui_kit::test]
    fn toolbar_and_palette_edit_only_the_active_mode(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::component::ActiveTheme as _;
        use gpui_kit::test::TestWindowExt as _;
        use gpui_kit::{Hsla, rgb};
        let dir = tempfile::tempdir().unwrap();
        let (handle, app) = open_theme_fixture(cx, &dir);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("set-accent-color", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(!window.has_active_dialog(cx));
            window.render_frame(cx);
            assert!(window.try_find("theme-color-tabs").is_some());
            window.click("theme-swatch-Orange-500", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(cx.theme().primary, Hsla::from(rgb(0xf97316)));
            assert_eq!(Settings::load(dir.path()), Settings::default());
            window.click("theme-secondary-tab", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.click("theme-swatch-Cyan-400", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.click("theme-accent-tab", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let editor = app.read(cx).theme_editor.read(cx);
            assert_eq!(editor.draft().light.accent.as_deref(), Some("#F97316"));
            assert_eq!(editor.draft().light.secondary.as_deref(), Some("#22D3EE"));
            window.click("theme-color-save", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            Settings::load(dir.path()).theme.light.accent.as_deref(),
            Some("#F97316")
        );
        cx.update_window(handle, |_, window, cx| {
            assert!(!window.has_active_dialog(cx));
            assert_eq!(cx.theme().primary, Hsla::from(rgb(0xf97316)));
            window.click("toggle-theme", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(cx.theme().primary, Hsla::from(rgb(0x0a84ff)));
            window.press("cmd-k", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.input("secondary", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.press("cmd-a", cx);
            window.input("#102030", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let saved = Settings::load(dir.path());
        assert_eq!(saved.theme.dark.secondary.as_deref(), Some("#102030"));
        assert_eq!(saved.theme.light.accent.as_deref(), Some("#F97316"));
        assert_eq!(saved.theme.light.secondary.as_deref(), Some("#22D3EE"));
        assert_eq!(saved.theme.dark.accent, None);
        cx.update_window(handle, |_, window, cx| {
            window.click("toggle-theme", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            assert!(!cx.theme().is_dark());
            assert_eq!(cx.theme().primary, Hsla::from(rgb(0xf97316)));
        })
        .unwrap();
        app.read_with(cx, |app, cx| {
            assert_eq!(app.table.read(cx).delegate().settings, saved);
        });
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
            ibex_app::AppState::builder(ibex_app::testing::MemoryStore::new())
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let first = worker
            .list_grid_page_blocking(
                ibex_core::ResourceGridParent::Roots,
                ibex_core::ResourceQuery {
                    sort: None,
                    filter: ibex_core::ResourceFilter::none(),
                },
                0,
                50,
            )
            .unwrap();
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

    #[test]
    fn workspace_gradient_starts_top_right_and_ends_bottom_left() {
        use super::{WORKSPACE_GRADIENT_ANGLE, workspace_gradient_fraction};
        assert_eq!(WORKSPACE_GRADIENT_ANGLE, 225.);
        for (width, height) in [(800., 600.), (600., 800.), (800., 800.)] {
            assert!(workspace_gradient_fraction(width, 0., width, height) < 1e-6);
            assert!((workspace_gradient_fraction(0., height, width, height) - 1.).abs() < 1e-6);
            assert!(
                (workspace_gradient_fraction(width / 2., height / 2., width, height) - 0.5).abs()
                    < 1e-6
            );
        }
        // Match GPUI's aspect-corrected projection so corner masks use the backdrop's color.
        assert!((workspace_gradient_fraction(0., 0., 800., 600.) - 0.675).abs() < 1e-6);
        assert!((workspace_gradient_fraction(800., 600., 800., 600.) - 0.325).abs() < 1e-6);
    }

    #[test]
    fn wash_runs_neutral_to_the_exact_secondary_in_both_modes() {
        for dark in [false, true] {
            let neutral = rgb(if dark { 0x292722 } else { 0xeae8e2 });
            assert_eq!(
                workspace_gradient_stops(dark, Some(0x102030)),
                (neutral, rgb(0x102030))
            );
            let end = rgb(if dark { 0x262c35 } else { 0xd9dde5 });
            assert_eq!(workspace_gradient_stops(dark, None), (neutral, end));
        }
    }
    use super::{GLOW_REACH, WorkspaceGlow, mix_rgb, workspace_gradient_stops};
    use gpui_kit::{Hsla, rgb};

    #[test]
    fn accent_glow_is_vivid_in_the_corner_and_gone_past_its_reach() {
        for dark in [false, true] {
            let stops = workspace_gradient_stops(dark, None);
            let glow = WorkspaceGlow::new(dark, Hsla::from(rgb(0xff3b30)));
            // The corner leans visibly toward the accent without becoming it.
            let corner = glow.over(stops, 0.);
            assert_eq!(corner, mix_rgb(stops.0, glow.accent, glow.alpha));
            let shift = [
                corner.r - stops.0.r,
                corner.g - stops.0.g,
                corner.b - stops.0.b,
            ]
            .map(f32::abs)
            .into_iter()
            .fold(0., f32::max);
            assert!(shift > 0.1, "{corner:?}");
            assert_ne!(corner, rgb(0xff3b30));
            // Beyond its reach the backdrop is the plain wash, ending exactly.
            for t in [GLOW_REACH, 0.5, 1.] {
                assert_eq!(glow.over(stops, t), mix_rgb(stops.0, stops.1, t));
            }
            assert_eq!(glow.over(stops, 1.), stops.1);
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
    use ibex_core::{Name, Person, PersonId, ResourceSort, SortDirection, UtcMillis};

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
    use ibex_core::{NonEmptyStr, ResourceDetail, ResourceId, ResourceKind, UtcMillis};

    /// A detail whose title is the only varying field, enough to prove
    /// which entry a cache read returns.
    pub(super) fn detail(id: ResourceId, title: &str) -> ResourceDetail {
        ResourceDetail {
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
            collections: vec![],
            attachments: vec![],
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
    use ibex_core::{NonEmptyStr, ResourceId, ResourceKind, ResourceSummary, UtcMillis};

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

        // A journal article accepts journal or issue parents; other kinds
        // never appear among the options, whatever page served them.
        let mut issue = summary("Aaa Issue");
        issue.kind = ResourceKind::Issue;
        let options = parent_options(
            &[journal, article, issue, edited],
            edited_id,
            ResourceKind::JournalArticle.parent_rule(),
        );
        let labels: Vec<String> = options
            .iter()
            .map(ParentItem::title)
            .map(String::from)
            .collect();
        assert_eq!(labels, ["Aaa Issue", "Zzz Journal"]);
    }
}

#[cfg(test)]
mod biblatex_gui_tests {
    use super::{LibraryApp, LibraryDelegate, Settings};
    use crate::db::DbWorker;
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, KeyBinding, TestAppContext};
    use ibex_app::testing::MemoryStore;
    use ibex_core::{Resource, Store as _};

    struct Fixture {
        handle: AnyWindowHandle,
        app: Entity<LibraryApp>,
        worker: DbWorker,
        store: MemoryStore,
        dir: tempfile::TempDir,
    }

    fn open(cx: &mut TestAppContext) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        cx.executor().allow_parking();
        let store = MemoryStore::new();
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(store.clone())
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let first = worker
            .list_grid_page_blocking(
                ibex_core::ResourceGridParent::Roots,
                ibex_core::ResourceQuery {
                    sort: None,
                    filter: ibex_core::ResourceFilter::none(),
                },
                0,
                50,
            )
            .unwrap();
        let root = dir.path().to_path_buf();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.bind_keys([KeyBinding::new(
                "n",
                super::BeginNewRow,
                Some("LibraryApp && !Input"),
            )]);
            cx.bind_keys([KeyBinding::new(
                "cmd-shift-i",
                super::ImportBiblatexFile,
                Some("LibraryApp"),
            )]);
            let worker = worker.clone();
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                let table = cx.new(|cx| {
                    gpui_kit::component::table::TableState::new(
                        LibraryDelegate::new(worker, first, Settings::default()),
                        window,
                        cx,
                    )
                });
                cx.new(|cx| LibraryApp::new(table, root, window, cx))
            })
            .unwrap()
        });
        Fixture {
            handle,
            app,
            worker,
            store,
            dir,
        }
    }

    impl Fixture {
        fn write(&self, name: &str, source: &str) -> std::path::PathBuf {
            let path = self.dir.path().join(name);
            std::fs::write(&path, source).unwrap();
            path
        }

        fn row_count(&self) -> usize {
            self.worker.list_page_blocking(None, 50).unwrap().rows.len()
        }

        /// Let worker replies land: fence on the worker queue between parks.
        fn settle(&self, cx: &mut TestAppContext) {
            for _ in 0..3 {
                cx.run_until_parked();
                self.worker.list_page_blocking(None, 50).unwrap();
            }
            cx.run_until_parked();
        }

        /// Pick `path` through the keyboard shortcut and wait for the preview.
        fn choose(&self, cx: &mut TestAppContext, path: &std::path::Path) {
            cx.update_window(self.handle, |_, window, cx| {
                window.render_frame(cx);
                self.app.update(cx, |app, cx| app.focus_table(window, cx));
                window.press("cmd-shift-i", cx);
            })
            .unwrap();
            assert!(cx.did_prompt_for_paths());
            let path = path.to_path_buf();
            cx.simulate_path_prompt_response(move |_| Some(vec![path]));
            self.settle(cx);
        }

        fn window<R>(
            &self,
            cx: &mut TestAppContext,
            f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
        ) -> R {
            cx.update_window(self.handle, |_, window, cx| {
                window.render_frame(cx);
                f(window, cx)
            })
            .unwrap()
        }

        /// Stage a name in a dialog picker using the pointer to open it and
        /// the keyboard to search and pick the first row.
        fn pick(&self, cx: &mut TestAppContext, picker: &'static str, query: &str) {
            self.window(cx, |window, cx| window.click(picker, cx));
            cx.run_until_parked();
            self.window(cx, |window, cx| window.input(query, cx));
            // The picker's search is debounced.
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(200));
            cx.run_until_parked();
            self.window(cx, |window, cx| {
                window.press("down", cx);
                window.press("enter", cx);
            });
            cx.run_until_parked();
        }

        fn resources(&self) -> Vec<Resource> {
            let ids: Vec<_> = self
                .worker
                .list_page_blocking(None, 50)
                .unwrap()
                .rows
                .into_iter()
                .map(|row| row.id)
                .collect();
            let runtime = tokio::runtime::Runtime::new().unwrap();
            ids.into_iter()
                .map(|id| runtime.block_on(self.store.get_resource(id)).unwrap())
                .collect()
        }

        fn tag_texts(&self) -> Vec<String> {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let mut texts: Vec<String> = runtime
                .block_on(self.store.list_tags())
                .unwrap()
                .into_iter()
                .map(|tag| tag.text.as_str().to_owned())
                .collect();
            texts.sort();
            texts
        }
    }

    #[gpui_kit::test]
    fn toolbar_imports_bibliography_refreshes_rows_and_cancel_does_not_write(
        cx: &mut TestAppContext,
    ) {
        let f = open(cx);
        let path = f.write(
            "example.bib",
            "@book{book,title={A GUI imported book}}\n@online{web,title={A GUI imported website}}\n@patent{patent,title={Skipped}}",
        );
        f.window(cx, |window, cx| {
            f.app.update(cx, |app, cx| app.focus_table(window, cx));
            window.press("n", cx);
            window.click("add-resource", cx);
            window.press("down", cx);
            window.press("down", cx);
            window.press("enter", cx);
        });
        assert!(cx.did_prompt_for_paths());
        cx.run_until_parked();
        f.worker.list_page_blocking(None, 50).unwrap();
        cx.run_until_parked();
        f.window(cx, |_, cx| {
            assert!(
                !f.app.read(cx).has_inline(cx),
                "stale new-row reply must not open a draft during import"
            );
        });
        cx.simulate_path_prompt_response(|options| {
            assert!(options.files);
            assert!(!options.directories);
            assert!(!options.multiple);
            Some(vec![path.clone()])
        });
        f.settle(cx);

        // The dialog always opens, and nothing is written before confirming.
        f.window(cx, |window, cx| {
            assert!(window.has_active_dialog(cx));
            assert_eq!(
                window.find("import-bibliography").label(),
                Some("3 entries · 2 new · 1 will be skipped")
            );
            window.click("import-confirm", cx);
        });
        f.settle(cx);
        assert_eq!(f.row_count(), 2);
        f.window(cx, |window, cx| {
            assert!(window.has_active_dialog(cx));
            assert_eq!(
                window.find("biblatex-import-report").label(),
                Some("2 imported · 0 reused · 1 skipped · 0 conflicted · 0 failed · 0 not attempted · 0 warnings")
            );
            assert_eq!(f.app.read(cx).counts.as_ref().unwrap().all_items, 2);
            assert_eq!(f.app.read(cx).table.read(cx).delegate().rows.len(), 2);
            window.press("escape", cx);
        });
        cx.run_until_parked();
        f.window(cx, |window, cx| {
            assert!(!window.has_active_dialog(cx));
            f.app.update(cx, |app, cx| app.focus_table(window, cx));
            window.press("cmd-shift-i", cx);
        });
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        assert_eq!(f.row_count(), 2);
        f.window(cx, |window, cx| assert!(!window.has_active_dialog(cx)));
    }

    #[gpui_kit::test]
    fn escape_cancels_the_import_dialog_without_writing_staged_names(cx: &mut TestAppContext) {
        let f = open(cx);
        let path = f.write("cancel.bib", "@book{a,title={Never imported}}");
        f.choose(cx, &path);
        f.pick(cx, "import-tags-picker", "staged only");
        f.window(cx, |window, cx| {
            assert!(window.has_active_dialog(cx));
            window.press("escape", cx);
        });
        f.settle(cx);
        f.window(cx, |window, cx| assert!(!window.has_active_dialog(cx)));
        assert_eq!(f.row_count(), 0);
        assert!(f.tag_texts().is_empty());
    }

    #[gpui_kit::test]
    fn staged_tags_and_collections_attach_to_every_imported_entry(cx: &mut TestAppContext) {
        let f = open(cx);
        let path = f.write(
            "batch.bib",
            "@book{a,title={First batch book}}\n@book{b,title={Second batch book}}",
        );
        f.choose(cx, &path);
        f.pick(cx, "import-tags-picker", "reading");
        f.pick(cx, "import-collections-picker", "Thesis");
        f.window(cx, |window, cx| {
            assert!(window.try_find("import-remove-tag-new-reading").is_some());
            assert!(
                window
                    .try_find("import-remove-collection-new-Thesis")
                    .is_some()
            );
            window.click("import-confirm", cx);
            // A second activation while the import runs must not resubmit.
            if window.try_find("import-confirm").is_some() {
                window.click("import-confirm", cx);
            }
        });
        f.settle(cx);
        assert_eq!(f.tag_texts(), ["reading"]);
        let resources = f.resources();
        assert_eq!(resources.len(), 2);
        for resource in &resources {
            assert_eq!(resource.tag_ids.len(), 1);
            assert_eq!(resource.collection_ids.len(), 1);
        }
        f.settle(cx);
        f.window(cx, |_, cx| {
            let app = f.app.read(cx);
            assert!(app.tags.iter().any(|tag| tag.text.as_str() == "reading"));
            assert!(
                super::flatten_collections(&app.collection_forest)
                    .iter()
                    .any(|c| c.name.as_str() == "Thesis")
            );
        });
    }

    #[gpui_kit::test]
    fn conflict_choice_keeps_staged_targets(cx: &mut TestAppContext) {
        let f = open(cx);
        let seed = f.write(
            "seed.bib",
            "@book{seed, title={Notes}, author={Ada, Lovelace}, doi={10.1000/notes}}",
        );
        f.choose(cx, &seed);
        f.window(cx, |window, cx| window.click("import-confirm", cx));
        f.settle(cx);
        f.window(cx, |window, cx| window.press("escape", cx));
        cx.run_until_parked();
        assert_eq!(f.row_count(), 1);

        let next = f.write(
            "next.bib",
            "@book{dup, title={Dup}, author={Ada, Lovelace}, doi={10.1000/notes}}\n@book{fresh, title={Fresh book}}",
        );
        f.choose(cx, &next);
        f.pick(cx, "import-tags-picker", "batch");
        f.window(cx, |window, cx| {
            assert!(window.try_find("import-confirm").is_none());
            assert!(window.try_find("import-intentional-dups").is_some());
            window.click("import-skip-conflicts", cx);
        });
        f.settle(cx);
        let resources = f.resources();
        assert_eq!(resources.len(), 2);
        let fresh = resources
            .iter()
            .find(|r| r.title.as_str() == "Fresh book")
            .unwrap();
        let notes = resources
            .iter()
            .find(|r| r.title.as_str() == "Notes")
            .unwrap();
        assert_eq!(fresh.tag_ids.len(), 1);
        assert!(notes.tag_ids.is_empty(), "skipped conflict stays unchanged");
        assert_eq!(f.tag_texts(), ["batch"]);
    }
}
#[cfg(test)]
mod resource_grid_tests {
    use super::{DbWorker, LibraryDelegate, ResourceKind, Settings};

    #[test]
    fn collapsed_grid_does_not_show_children_as_roots() {
        let store = ibex_app::testing::MemoryStore::new();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ibex_app::seed::seed_demo_library(&store, 20))
            .unwrap();
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(store)
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let first = worker
            .list_grid_page_blocking(
                ibex_core::ResourceGridParent::Roots,
                ibex_core::ResourceQuery {
                    sort: None,
                    filter: ibex_core::ResourceFilter::none(),
                },
                0,
                50,
            )
            .unwrap();
        let grid = LibraryDelegate::new(worker, first, Settings::default());
        assert!(
            !grid
                .rows
                .iter()
                .any(|row| row.kind == ResourceKind::BookSection)
        );
        assert!(
            !grid
                .rows
                .iter()
                .any(|row| row.kind == ResourceKind::JournalArticle)
        );
    }
}

#[cfg(test)]
mod nested_grid_interaction_tests {
    use super::{
        DbWorker, LibraryApp, LibraryDelegate, ResourceFilter, ResourceGridParent, ResourceKind,
        ResourceQuery, Settings, TableState,
    };
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext};

    #[gpui_kit::test]
    fn pointer_disclosure_loads_children_and_keyboard_collapses(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.executor().allow_parking();
        let store = ibex_app::testing::MemoryStore::new();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            use ibex_core::Store as _;
            ibex_app::seed::seed_demo_library(&store, 20).await.unwrap();
            let chapter = store
                .list_resources()
                .await
                .unwrap()
                .into_iter()
                .find(|row| row.kind == ResourceKind::BookSection)
                .unwrap();
            let template = store.get_resource(chapter.id).await.unwrap().to_new();
            for _ in 0..60 {
                store.create_resource(template.clone()).await.unwrap();
            }
        });
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(store)
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let query = ResourceQuery {
            sort: None,
            filter: ResourceFilter::none(),
        };
        let first = worker
            .list_grid_page_blocking(ResourceGridParent::Roots, query, 0, 50)
            .unwrap();
        let parent = first
            .rows
            .iter()
            .find(|row| row.summary.kind == ResourceKind::Book && row.has_children)
            .unwrap()
            .summary
            .id;
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            let worker = worker.clone();
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                let table = cx.new(|cx| {
                    TableState::new(
                        LibraryDelegate::new(worker, first, Settings::default()),
                        window,
                        cx,
                    )
                });
                cx.new(|cx| LibraryApp::new(table, dir.path().to_path_buf(), window, cx))
            })
            .unwrap()
        });
        let before = app.read_with(cx, |app, cx| app.table.read(cx).delegate().rows.len());
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| app.focus_table(window, cx));
            window.render_frame(cx);
            window.click(format!("resource-expand-{parent}"), cx);
        })
        .unwrap();
        cx.run_until_parked();
        worker.list_page_blocking(None, 1).unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let table = app.read(cx).table.clone();
            assert_eq!(
                table.read(cx).delegate().rows.len(),
                before + 63,
                "expanding a parent must load every child, not just the first page"
            );
            assert!(
                window
                    .try_find(format!("resource-children-{parent}"))
                    .is_none()
            );
            let ix = table
                .read(cx)
                .delegate()
                .rows
                .iter()
                .position(|row| row.kind == ResourceKind::BookSection)
                .unwrap();
            let selected = table.read(cx).delegate().rows[ix].id;
            table.update(cx, |table, cx| table.set_selected_row(ix, cx));
            app.update(cx, |app, cx| app.focus_table(window, cx));
            window.render_frame(cx);
            window.press("alt-left", cx);
            assert_eq!(table.read(cx).delegate().rows.len(), before);
            assert_eq!(
                table
                    .read(cx)
                    .selected_row()
                    .and_then(|ix| table.read(cx).delegate().row_id(ix)),
                Some(parent)
            );
            assert!(
                !table
                    .read(cx)
                    .delegate()
                    .rows
                    .iter()
                    .any(|row| row.id == selected)
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn palette_toggles_grouping_without_selection_and_restores_grouped_rows(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        cx.executor().allow_parking();
        let store = ibex_app::testing::MemoryStore::new();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ibex_app::seed::seed_demo_library(&store, 20))
            .unwrap();
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(store)
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let total = worker.list_page_blocking(None, 50).unwrap().rows.len();
        let first = worker
            .list_grid_page_blocking(
                ResourceGridParent::Roots,
                ResourceQuery::newest_first(),
                0,
                50,
            )
            .unwrap();
        let grouped_count = first.rows.len();
        assert!(grouped_count < total);
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                let table = cx.new(|cx| {
                    TableState::new(
                        LibraryDelegate::new(worker.clone(), first, Settings::default()),
                        window,
                        cx,
                    )
                });
                cx.new(|cx| LibraryApp::new(table, dir.path().to_path_buf(), window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| app.open_command_palette(window, cx));
            window.render_frame(cx);
            assert!(
                window.try_find(("palette-row", 2usize)).is_some(),
                "grouping is a library command, available without a selected resource"
            );
            window.input("grouping", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.press("enter", cx))
            .unwrap();
        cx.run_until_parked();
        worker.list_page_blocking(None, 1).unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx));
            let table = app.read(cx).table.clone();
            let delegate = table.read(cx).delegate();
            assert_eq!(delegate.rows.len(), total);
            assert!(
                delegate
                    .grid
                    .positions
                    .values()
                    .all(|position| *position == (0, None, false))
            );
            app.update(cx, |app, cx| app.open_command_palette(window, cx));
            window.render_frame(cx);
            window.click(("palette-row", 2usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        worker.list_page_blocking(None, 1).unwrap();
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            assert_eq!(app.table.read(cx).delegate().rows.len(), grouped_count);
        });
    }
}

#[cfg(test)]
mod cached_grid_interaction_tests {
    use super::resource_grid::ChildPage;
    use super::{DbWorker, LibraryApp, LibraryDelegate, Settings, TableState};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext};
    use ibex_core::{
        NonEmptyStr, ResourceGridPage, ResourceGridRow, ResourceId, ResourceKind, ResourceSummary,
        UtcMillis,
    };

    fn row(title: &str, has_children: bool) -> ResourceGridRow {
        ResourceGridRow {
            summary: ResourceSummary {
                id: ResourceId::generate(),
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new(title).unwrap(),
                creators_label: String::new(),
                published_label: None,
                publisher: None,
                container_title: None,
                date_added: UtcMillis::from_unix_ms(0),
                date_edited: UtcMillis::from_unix_ms(0),
            },
            has_children,
        }
    }

    #[gpui_kit::test]
    fn disclosure_preserves_other_selection_and_selects_collapsed_ancestor(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        cx.executor().allow_parking();
        let parent = row("Parent book", true);
        let child = row("Chapter", false);
        let other = row("Other root", false);
        let parent_id = parent.summary.id;
        let child_id = child.summary.id;
        let other_id = other.summary.id;
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(ibex_app::testing::MemoryStore::new())
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                let mut delegate = LibraryDelegate::new(
                    worker,
                    ResourceGridPage {
                        rows: vec![parent, other],
                        next_offset: None,
                    },
                    Settings::default(),
                );
                delegate.grid.children.insert(
                    parent_id,
                    ChildPage {
                        rows: vec![child],
                        loaded: true,
                        ..Default::default()
                    },
                );
                let table = cx.new(|cx| TableState::new(delegate, window, cx));
                cx.new(|cx| LibraryApp::new(table, dir.path().to_path_buf(), window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            let table = app.read(cx).table.clone();
            table.update(cx, |table, cx| table.set_selected_row(1, cx));
            app.update(cx, |app, cx| app.focus_table(window, cx));
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window
                    .try_find(format!("resource-expand-{child_id}"))
                    .is_none()
            );
            window.click(format!("resource-expand-{parent_id}"), cx);
            let table = app.read(cx).table.clone();
            assert_eq!(table.read(cx).delegate().rows.len(), 3);
            assert_eq!(
                table
                    .read(cx)
                    .selected_row()
                    .and_then(|ix| table.read(cx).delegate().row_id(ix)),
                Some(other_id)
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let table = app.read(cx).table.clone();
            table.update(cx, |table, cx| table.set_selected_row(1, cx));
            app.update(cx, |app, cx| app.focus_table(window, cx));
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("alt-left", cx);
            let table = app.read(cx).table.clone();
            assert_eq!(table.read(cx).delegate().rows.len(), 2);
            assert_eq!(
                table
                    .read(cx)
                    .selected_row()
                    .and_then(|ix| table.read(cx).delegate().row_id(ix)),
                Some(parent_id)
            );
            window.press("alt-right", cx);
            assert_eq!(table.read(cx).delegate().rows.len(), 3);
        })
        .unwrap();
    }

    #[test]
    fn arriving_child_page_moves_loading_editor_by_resource_id() {
        let parent = row("Parent", true);
        let other = row("Editing this", false);
        let id = other.summary.id;
        let parent_id = parent.summary.id;
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(ibex_app::testing::MemoryStore::new())
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let mut delegate = LibraryDelegate::new(
            worker,
            ResourceGridPage {
                rows: vec![parent, other],
                next_offset: None,
            },
            Settings::default(),
        );
        delegate.inline = Some(super::InlineSession::Loading { id, row_ix: 1 });
        delegate.grid.expanded.insert(parent_id);
        let (token, _) = delegate.grid.begin(parent_id).unwrap();
        delegate.grid.finish(
            parent_id,
            token,
            Ok(ResourceGridPage {
                rows: vec![row("New child", false)],
                next_offset: None,
            }),
        );
        delegate.rebuild_rows();
        assert_eq!(delegate.inline.as_ref().unwrap().row_ix(), 2);
        assert_eq!(delegate.row_id(2), Some(id));
        assert_ne!(delegate.row_id(1), Some(id));
    }

    #[gpui_kit::test]
    fn child_retry_and_collapse_remain_available_while_request_is_pending(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.executor().allow_parking();
        let parent = row("Retry this book", true);
        let id = parent.summary.id;
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(ibex_app::testing::MemoryStore::new())
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                let mut delegate = LibraryDelegate::new(
                    worker,
                    ResourceGridPage {
                        rows: vec![parent],
                        next_offset: None,
                    },
                    Settings::default(),
                );
                delegate.grid.expanded.insert(id);
                delegate.grid.children.insert(
                    id,
                    ChildPage {
                        error: Some("Connection lost".into()),
                        ..Default::default()
                    },
                );
                let table = cx.new(|cx| TableState::new(delegate, window, cx));
                cx.new(|cx| LibraryApp::new(table, dir.path().to_path_buf(), window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let retry = format!("resource-children-{id}");
            let expand = format!("resource-expand-{id}");
            assert_eq!(
                window.find(retry.clone()).label(),
                Some("Children failed · Retry")
            );
            window.click(retry.clone(), cx);
            assert_eq!(
                window.find(retry.clone()).label(),
                Some("Loading children…")
            );
            let table = app.read(cx).table.clone();
            let first = table.read(cx).delegate().grid.children[&id]
                .request
                .unwrap();
            window.click(expand.clone(), cx);
            assert!(
                table.read(cx).delegate().grid.children[&id]
                    .request
                    .is_none()
            );
            assert!(window.try_find(retry.clone()).is_none());
            window.click(expand.clone(), cx);
            let second = table.read(cx).delegate().grid.children[&id]
                .request
                .unwrap();
            assert_ne!(first, second);
            assert_eq!(window.find(retry).label(), Some("Loading children…"));

            table.update(cx, |table, _| {
                table.delegate_mut().inline = Some(super::InlineSession::Loading { id, row_ix: 0 });
            });
            window.render_frame(cx);
            window.click(expand, cx);
            assert!(
                table.read(cx).delegate().grid.expanded.contains(&id),
                "disclosure must not move an active inline editor"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn background_loading_keeps_the_grid_viewport_height(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.executor().allow_parking();
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(ibex_app::testing::MemoryStore::new())
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                let first = ResourceGridPage {
                    rows: vec![row("Book", false)],
                    next_offset: Some(1),
                };
                let table = cx.new(|cx| {
                    TableState::new(
                        LibraryDelegate::new(worker, first, Settings::default()),
                        window,
                        cx,
                    )
                });
                cx.new(|cx| LibraryApp::new(table, dir.path().to_path_buf(), window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let idle = window.find("library-grid-viewport").bounds();
            let table = app.read(cx).table.clone();
            table.update(cx, |table, cx| {
                table.delegate_mut().loading = true;
                cx.notify();
            });
            window.render_frame(cx);
            assert_eq!(
                window.find("library-grid-viewport").bounds(),
                idle,
                "background page loading must not add a footer or shrink the grid"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn sidebar_sections_toggle_independently_without_clearing_filter(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.executor().allow_parking();
        let store = ibex_app::testing::MemoryStore::new();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ibex_app::seed::seed_demo_library(&store, 8))
            .unwrap();
        let (tags, forest) = tokio::runtime::Runtime::new().unwrap().block_on(async {
            use ibex_core::Store as _;
            (
                store.list_tags().await.unwrap(),
                store.list_collection_forest().await.unwrap(),
            )
        });
        let tag = tags.into_iter().next().unwrap();
        let first_tag = tag.id;
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(store)
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let first = worker
            .list_grid_page_blocking(
                ibex_core::ResourceGridParent::Roots,
                ibex_core::ResourceQuery::newest_first(),
                0,
                50,
            )
            .unwrap();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(
                gpui_kit::WindowOptions {
                    window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                        origin: Default::default(),
                        size: gpui_kit::size(gpui_kit::px(1100.), gpui_kit::px(700.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let table = cx.new(|cx| {
                        TableState::new(
                            LibraryDelegate::new(
                                worker,
                                first,
                                Settings {
                                    left_sidebar_width: Some(240.),
                                    right_sidebar_width: Some(360.),
                                    ..Settings::default()
                                },
                            ),
                            window,
                            cx,
                        )
                    });
                    cx.new(|cx| LibraryApp::new(table, dir.path().to_path_buf(), window, cx))
                },
            )
            .unwrap()
        });
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.counts = Some(ibex_core::ResourceCounts {
                all_items: 1,
                by_kind: [(ResourceKind::Book, 1)].into(),
                by_tag: [(first_tag, 1)].into(),
                ..Default::default()
            });
            app.tags = vec![tag];
            app.collection_forest = forest;
            cx.notify();
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("toggle-section-kinds").is_some());
            let kinds_arrow = window.find("toggle-section-kinds").bounds().center();
            let tags_arrow = window.find("toggle-section-tags").bounds().center();
            let collections_arrow = window.find("toggle-section-collections").bounds().center();
            let new_collection = window.find("new-collection").bounds().center();
            assert!((kinds_arrow.x - tags_arrow.x).abs() < gpui_kit::px(2.));
            assert!((tags_arrow.x - collections_arrow.x).abs() < gpui_kit::px(2.));
            assert!(new_collection.x < collections_arrow.x);
            assert!((new_collection.y - collections_arrow.y).abs() < gpui_kit::px(2.));
            let tags_before = window.find("toggle-section-tags").bounds().top();
            app.update(cx, |app, cx| {
                app.table.update(cx, |table, _| {
                    table.delegate_mut().filter.kind = Some(ResourceKind::Book);
                });
            });
            window.click("toggle-section-kinds", cx);
            let tags_collapsed = window.find("toggle-section-tags").bounds().top();
            assert!(
                tags_collapsed < tags_before,
                "hiding kind rows should move Tags upward"
            );
            assert_eq!(
                app.read(cx).table.read(cx).delegate().filter.kind,
                Some(ResourceKind::Book)
            );
            let collections_before = window.find("toggle-section-collections").bounds().top();
            window.click("toggle-section-tags", cx);
            assert!(window.find("toggle-section-collections").bounds().top() < collections_before);
            window.click("toggle-section-collections", cx);
            assert!(window.try_find("new-collection").is_some());
            assert_eq!(app.read(cx).collapsed_sections.len(), 3);
            window.click("new-collection", cx);
            assert!(
                !app.read(cx)
                    .collapsed_sections
                    .contains(&super::SidebarSection::Collections)
            );
            window.click("toggle-section-kinds", cx);
            assert!(window.find("toggle-section-tags").bounds().top() > tags_collapsed);
            window.click("toggle-section-tags", cx);
            assert!(app.read(cx).collapsed_sections.is_empty());
        })
        .unwrap();
    }
}

#[cfg(test)]
mod sidebar_navigation_tests {
    use super::{DbWorker, LibraryApp, LibraryDelegate, Settings, TableState};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, WindowBounds,
        WindowOptions, px, size,
    };
    use ibex_core::{ResourceGridPage, ResourceKind};

    fn open_sidebar(
        cx: &mut TestAppContext,
    ) -> (AnyWindowHandle, Entity<LibraryApp>, tempfile::TempDir) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(ibex_app::testing::MemoryStore::new())
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(1100.), px(1100.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let table = cx.new(|cx| {
                        TableState::new(
                            LibraryDelegate::new(
                                worker,
                                ResourceGridPage {
                                    rows: Vec::new(),
                                    next_offset: None,
                                },
                                Settings::default(),
                            ),
                            window,
                            cx,
                        )
                    });
                    cx.new(|cx| LibraryApp::new(table, dir.path().to_path_buf(), window, cx))
                },
            )
            .unwrap()
        });
        cx.run_until_parked();
        (handle, app, dir)
    }

    #[gpui_kit::test]
    fn sidebar_kinds_group_containers_with_contents(cx: &mut TestAppContext) {
        let (handle, app, _dir) = open_sidebar(cx);
        app.update(cx, |app, cx| {
            app.counts = None;
            cx.notify();
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // A literal reading order, independent of the core enum's declaration order.
            let expected = [
                ResourceKind::Book,
                ResourceKind::BookSection,
                ResourceKind::Journal,
                ResourceKind::Issue,
                ResourceKind::JournalArticle,
                ResourceKind::ConferencePaper,
                ResourceKind::Preprint,
                ResourceKind::Thesis,
                ResourceKind::Report,
                ResourceKind::Manuscript,
                ResourceKind::MagazineArticle,
                ResourceKind::NewspaperArticle,
                ResourceKind::DictionaryEntry,
                ResourceKind::EncyclopediaArticle,
                ResourceKind::Webpage,
                ResourceKind::Blog,
                ResourceKind::BlogPost,
                ResourceKind::Podcast,
                ResourceKind::PodcastEpisode,
                ResourceKind::Dataset,
                ResourceKind::ComputerProgram,
            ];
            let mut previous_y = px(0.);
            for (index, kind) in expected.into_iter().enumerate() {
                let id = ("sidebar-entry", index + 2);
                let y = window.find(id).bounds().top();
                assert!(y > previous_y);
                previous_y = y;
                window.click(id, cx);
                assert_eq!(
                    app.read(cx).table.read(cx).delegate().filter.kind,
                    Some(kind)
                );
            }
        })
        .unwrap();
    }

    fn collection(
        name: &str,
        children: Vec<ibex_core::CollectionTreeNode>,
    ) -> ibex_core::CollectionTreeNode {
        ibex_core::CollectionTreeNode {
            collection: ibex_core::Collection::try_new(
                ibex_core::CollectionId::generate(),
                ibex_core::NonEmptyStr::try_new(name).unwrap(),
            ),
            children,
        }
    }

    #[gpui_kit::test]
    fn collection_disclosure_does_not_select_and_refresh_keeps_collapsed_branches(
        cx: &mut TestAppContext,
    ) {
        let (handle, app, _dir) = open_sidebar(cx);
        let child = collection("Child", vec![]);
        let parent = collection("Parent", vec![child]);
        let parent_id = parent.collection.id;
        let other = collection("Other", vec![]);
        let other_id = other.collection.id;
        let forest = vec![parent, other];
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.set_collection_forest(forest.clone(), window, cx);
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.try_find(("collection-row", 2_usize)).is_some());
            // Select the other root, then disclose an unrelated parent.
            window.click(("collection-row", 2_usize), cx);
            assert_eq!(
                app.read(cx).table.read(cx).delegate().filter.collection,
                Some(other_id)
            );
            let generation = app.read(cx).table.read(cx).delegate().listing_gen;
            window.click(format!("toggle-collection-{parent_id}"), cx);
            assert!(!app.read(cx).expanded_collections.contains(&parent_id));
            assert_eq!(
                app.read(cx).table.read(cx).delegate().filter.collection,
                Some(other_id)
            );
            assert_eq!(
                app.read(cx).table.read(cx).delegate().listing_gen,
                generation
            );
            assert!(window.try_find(("collection-row", 2_usize)).is_none());
            // A metadata/count refresh must not undo the user's disclosure choice.
            app.update(cx, |app, cx| {
                app.set_collection_forest(forest.clone(), window, cx);
                cx.notify();
            });
            window.render_frame(cx);
            assert!(!app.read(cx).expanded_collections.contains(&parent_id));
            assert!(window.try_find(("collection-row", 2_usize)).is_none());
            // The row body selects but does not expand.
            window.click(("collection-row", 0_usize), cx);
            assert_eq!(
                app.read(cx).table.read(cx).delegate().filter.collection,
                Some(parent_id)
            );
            assert!(!app.read(cx).expanded_collections.contains(&parent_id));
            window.click(format!("toggle-collection-{parent_id}"), cx);
            assert!(app.read(cx).expanded_collections.contains(&parent_id));
            assert!(window.try_find(("collection-row", 2_usize)).is_some());
            assert_eq!(
                app.read(cx).table.read(cx).delegate().filter.collection,
                Some(parent_id)
            );
            window.click(format!("toggle-collection-{parent_id}"), cx);
            assert!(!app.read(cx).expanded_collections.contains(&parent_id));
            assert_eq!(
                app.read(cx).table.read(cx).delegate().filter.collection,
                Some(parent_id)
            );
            let new_branch = collection("New branch", vec![collection("New child", vec![])]);
            let new_id = new_branch.collection.id;
            app.update(cx, |app, cx| {
                let mut refreshed = forest.clone();
                refreshed.push(new_branch);
                app.set_collection_forest(refreshed, window, cx);
                cx.notify();
            });
            window.render_frame(cx);
            assert!(!app.read(cx).expanded_collections.contains(&parent_id));
            assert!(app.read(cx).expanded_collections.contains(&new_id));
            assert!(
                window
                    .try_find(format!("toggle-collection-{new_id}"))
                    .is_some()
            );
            app.update(cx, |app, cx| {
                app.set_collection_forest(forest.clone(), window, cx);
                cx.notify();
            });
            window.render_frame(cx);
            assert!(!app.read(cx).expanded_collections.contains(&new_id));
            assert!(
                window
                    .try_find(format!("toggle-collection-{new_id}"))
                    .is_none()
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn sidebar_kind_counts_hide_empty_kinds_but_keep_the_selected_kind(cx: &mut TestAppContext) {
        let (handle, app, _dir) = open_sidebar(cx);
        app.update(cx, |app, cx| {
            app.counts = Some(ibex_core::ResourceCounts {
                all_items: 5,
                by_kind: [(ResourceKind::Book, 3), (ResourceKind::JournalArticle, 2)].into(),
                ..Default::default()
            });
            app.table.update(cx, |table, _| {
                table.delegate_mut().filter.kind = Some(ResourceKind::Podcast);
            });
            cx.notify();
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find(("sidebar-entry", 2_usize)).is_some()); // Book
            assert!(window.try_find(("sidebar-entry", 6_usize)).is_some()); // Article
            assert!(window.try_find(("sidebar-entry", 19_usize)).is_some()); // Selected empty Podcast
            assert!(window.try_find(("sidebar-entry", 4_usize)).is_none()); // Empty Journal
            assert!(window.try_find(("sidebar-entry", 20_usize)).is_none()); // Empty Episode
            assert!(
                window.find(("sidebar-entry", 2_usize)).bounds().top()
                    < window.find(("sidebar-entry", 6_usize)).bounds().top()
            );
            window.click(("sidebar-entry", 6_usize), cx);
            assert_eq!(
                app.read(cx).table.read(cx).delegate().filter.kind,
                Some(ResourceKind::JournalArticle)
            );
            assert!(window.try_find(("sidebar-entry", 19_usize)).is_none());
            window.click("toggle-section-kinds", cx);
            assert!(window.try_find(("sidebar-entry", 6_usize)).is_none());
            assert_eq!(
                app.read(cx).table.read(cx).delegate().filter.kind,
                Some(ResourceKind::JournalArticle)
            );
            window.click("toggle-section-kinds", cx);
            assert!(window.try_find(("sidebar-entry", 6_usize)).is_some());
            assert_eq!(
                app.read(cx).counts.as_ref().unwrap().by_kind[&ResourceKind::Book],
                3
            );
        })
        .unwrap();
    }
}

#[cfg(test)]
mod favorite_shortcut_tests {
    use super::{DbWorker, DetailFetch, LibraryApp, LibraryDelegate, Settings, TableState};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext};
    use ibex_core::{ResourceGridParent, ResourceQuery};

    fn fixture(
        cx: &mut TestAppContext,
    ) -> (
        gpui_kit::AnyWindowHandle,
        gpui_kit::Entity<LibraryApp>,
        DbWorker,
        tempfile::TempDir,
    ) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let store = ibex_app::testing::MemoryStore::new();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            ibex_app::seed::seed_demo_library(&store, 20).await.unwrap();
        });
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(store)
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let first = worker
            .list_grid_page_blocking(
                ResourceGridParent::Roots,
                ResourceQuery::newest_first(),
                0,
                50,
            )
            .unwrap();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::bind_favorite_keys(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                let table = cx.new(|cx| {
                    TableState::new(
                        LibraryDelegate::new(worker.clone(), first, Settings::default()),
                        window,
                        cx,
                    )
                });
                cx.new(|cx| LibraryApp::new(table, dir.path().to_path_buf(), window, cx))
            })
            .unwrap()
        });
        (handle, app, worker, dir)
    }

    fn settle(cx: &mut TestAppContext, worker: &DbWorker) {
        // Each fence lets the foreground completion enqueue its next worker hop.
        for _ in 0..6 {
            cx.run_until_parked();
            worker.list_page_blocking(None, 1).unwrap();
        }
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn favorite_key_toggles_while_detail_loads_and_keeps_target(cx: &mut TestAppContext) {
        let (handle, app, worker, _dir) = fixture(cx);
        let (target, other) = app.read_with(cx, |app, cx| {
            let rows = &app.table.read(cx).delegate().rows;
            (rows[0].id, rows[1].id)
        });
        let rt = tokio::runtime::Runtime::new().unwrap();
        let initial = rt.block_on(worker.resource(target)).unwrap().favorite;
        let other_initial = rt.block_on(worker.resource(other)).unwrap().favorite;
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.selected = Some(target);
                app.detail = Some((target, DetailFetch::Loading));
                app.focus_table(window, cx);
            });
            window.render_frame(cx);
            window.press("f", cx);
            window.press("f", cx);
            // Completion must never follow a newer selection.
            app.update(cx, |app, _| {
                app.selected = Some(other);
                app.detail = Some((other, DetailFetch::Loading));
            });
        })
        .unwrap();
        settle(cx, &worker);
        assert_eq!(
            rt.block_on(worker.resource(target)).unwrap().favorite,
            !initial
        );
        assert_eq!(
            rt.block_on(worker.resource(other)).unwrap().favorite,
            other_initial
        );
        app.read_with(cx, |app, _| {
            assert_eq!(app.detail.as_ref().map(|(id, _)| *id), Some(other));
        });
    }

    #[gpui_kit::test]
    fn favorite_write_reconciles_after_detail_generation_advances(cx: &mut TestAppContext) {
        let (handle, app, worker, _dir) = fixture(cx);
        let target = app.read_with(cx, |app, cx| app.table.read(cx).delegate().rows[0].id);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(worker.set_resource_favorite(target, true))
            .unwrap();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.select_sidebar_nav(super::SidebarNav::Favorites, window, cx);
                app.refresh_counts(window, cx);
            });
        })
        .unwrap();
        settle(cx, &worker);
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                let row = app
                    .table
                    .read(cx)
                    .delegate()
                    .rows
                    .iter()
                    .position(|row| row.id == target)
                    .unwrap();
                app.table
                    .update(cx, |table, cx| table.set_selected_row(row, cx));
                app.select_row(row, window, cx);
                app.focus_table(window, cx);
            });
        })
        .unwrap();
        settle(cx, &worker);
        let favorites_before = app.read_with(cx, |app, _| app.counts.as_ref().unwrap().favorites);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("f", cx);
            app.update(cx, |app, cx| {
                assert!(app.favorite_pending.contains(&target));
                // A concurrent metadata/identifier completion invalidates detail
                // while the favorite action is still between worker hops.
                *app.detail_gens.entry(target).or_default() += 1;
                app.detail_cache.invalidate(&target);
                app.fetch_detail(target, window, cx);
            });
        })
        .unwrap();
        settle(cx, &worker);
        assert!(!rt.block_on(worker.resource(target)).unwrap().favorite);
        app.read_with(cx, |app, cx| {
            assert!(!app.favorite_pending.contains(&target));
            let Some((id, DetailFetch::Loaded(detail))) = &app.detail else {
                panic!("successful favorite write must refresh detail");
            };
            assert_eq!(*id, target);
            assert!(!detail.favorite, "detail must reflect the persisted write");
            assert_eq!(app.counts.as_ref().unwrap().favorites, favorites_before - 1);
            assert!(
                !app.table
                    .read(cx)
                    .delegate()
                    .rows
                    .iter()
                    .any(|row| row.id == target),
                "the Favorites listing must drop the unstarred resource"
            );
        });
    }

    #[gpui_kit::test]
    fn favorite_button_and_key_share_pending_state_and_filter_moves_to_shift_f(
        cx: &mut TestAppContext,
    ) {
        let (handle, app, worker, _dir) = fixture(cx);
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.table
                    .update(cx, |table, cx| table.set_selected_row(0, cx));
                app.select_row(0, window, cx);
                app.focus_table(window, cx);
            });
        })
        .unwrap();
        settle(cx, &worker);
        let target = app.read_with(cx, |app, _| app.selected.unwrap());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let initial = rt.block_on(worker.resource(target)).unwrap().favorite;
        let favorites_before = app.read_with(cx, |app, _| app.counts.as_ref().unwrap().favorites);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("toggle-favorite", cx);
            assert_eq!(window.find("toggle-favorite").disabled(), Some(true));
            app.update(cx, |app, cx| app.focus_table(window, cx));
            window.press("f", cx);
        })
        .unwrap();
        settle(cx, &worker);
        assert_eq!(
            rt.block_on(worker.resource(target)).unwrap().favorite,
            !initial
        );
        app.read_with(cx, |app, _| {
            let Some((_, DetailFetch::Loaded(detail))) = &app.detail else {
                panic!("detail should reload");
            };
            assert_eq!(detail.favorite, !initial);
            assert_eq!(
                app.counts.as_ref().unwrap().favorites,
                if initial {
                    favorites_before - 1
                } else {
                    favorites_before + 1
                }
            );
        });
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| app.focus_table(window, cx));
            window.render_frame(cx);
            window.press("f", cx);
        })
        .unwrap();
        settle(cx, &worker);
        assert_eq!(
            rt.block_on(worker.resource(target)).unwrap().favorite,
            initial
        );
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.table
                    .update(cx, |table, cx| table.set_selected_col(super::COL_TITLE, cx));
                app.focus_table(window, cx);
            });
            window.render_frame(cx);
            window.press("shift-f", cx);
            let input = app
                .read(cx)
                .table
                .read(cx)
                .delegate()
                .filter_inputs
                .as_ref()
                .unwrap()
                .title
                .entity_id();
            assert_eq!(
                window
                    .within("toolbar-search")
                    .find(("input", input))
                    .focused(),
                Some(true)
            );
            window.input("fF", cx);
            assert_eq!(
                window
                    .within("toolbar-search")
                    .find(("input", input))
                    .value(),
                Some("fF")
            );
        })
        .unwrap();
        settle(cx, &worker);
        assert_eq!(
            rt.block_on(worker.resource(target)).unwrap().favorite,
            initial
        );
    }

    #[gpui_kit::test]
    fn favorite_key_ignores_empty_selection_and_new_draft(cx: &mut TestAppContext) {
        let (handle, app, worker, _dir) = fixture(cx);
        let target = app.read_with(cx, |app, cx| app.table.read(cx).delegate().rows[0].id);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let initial = rt.block_on(worker.resource(target)).unwrap().favorite;
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| app.focus_table(window, cx));
            window.render_frame(cx);
            window.press("f", cx);
            assert!(app.read(cx).selected.is_none());
            window.dispatch_action(Box::new(super::BeginNewRow), cx);
        })
        .unwrap();
        settle(cx, &worker);
        app.read_with(cx, |app, cx| assert!(app.has_inline(cx)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("f", cx);
            window.press("shift-f", cx);
            app.read_with(cx, |app, cx| {
                assert!(app.has_inline(cx));
                assert!(app.selected.is_none());
                assert!(!app.table.read(cx).delegate().filters_visible);
            });
        })
        .unwrap();
        settle(cx, &worker);
        assert_eq!(
            rt.block_on(worker.resource(target)).unwrap().favorite,
            initial
        );
    }
}

#[cfg(test)]
mod classification_tests {
    use super::{
        CollectionSuggestion, DbWorker, DetailFetch, JevState, JevStatus, LibraryApp,
        LibraryDelegate, Settings, TableState, collection_suggestions, flatten_collections,
        suggestion_label,
    };
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext};
    use ibex_core::{
        Collection, CollectionId, NonEmptyStr, ResourceGridParent, ResourceId, ResourceKind,
        ResourceQuery,
    };

    fn collection(name: &str, keywords: &[&str]) -> Collection {
        let mut c = Collection::try_new(
            CollectionId::generate(),
            NonEmptyStr::try_new(name).unwrap(),
        );
        c.keywords = keywords.iter().map(|k| k.to_string()).collect();
        c
    }

    #[test]
    fn suggestions_rank_jev_first_and_skip_memberships_and_unknown_ids() {
        let a = collection("Alpha", &["networks"]);
        let b = collection("Beta", &["networks"]);
        let c = collection("Gamma", &["networks"]);
        let library = vec![a.clone(), b.clone(), c.clone()];
        let mut detail = super::detail_prefetch_tests::detail(ResourceId::generate(), "Networks");
        detail.collections = vec![a.clone()];
        let jev = [(c.id, 0.91), (CollectionId::generate(), 0.99), (a.id, 0.8)];
        let out = collection_suggestions(&detail, &library, Some(&jev));
        assert_eq!(
            out,
            vec![
                CollectionSuggestion {
                    collection: c.clone(),
                    probability: Some(0.91)
                },
                CollectionSuggestion {
                    collection: b.clone(),
                    probability: None
                },
            ]
        );
        assert_eq!(suggestion_label(&out[0]), "Gamma · 91%");
        assert_eq!(suggestion_label(&out[1]), "Beta");
        detail.collections = library.clone();
        assert!(collection_suggestions(&detail, &library, None).is_empty());
    }

    struct Fixture {
        handle: gpui_kit::AnyWindowHandle,
        app: gpui_kit::Entity<LibraryApp>,
        worker: DbWorker,
        resource: ResourceId,
        collections: Vec<CollectionId>,
        _dir: tempfile::TempDir,
    }

    fn fixture(cx: &mut TestAppContext) -> Fixture {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let worker = DbWorker::spawn(
            ibex_app::AppState::builder(ibex_app::testing::MemoryStore::new())
                .surface(ibex_app::Surface::Test)
                .build(),
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (resource, collections) = rt.block_on(async {
            let mut ids = Vec::new();
            for (name, keywords) in [
                ("Networks", vec!["citation networks".to_string()]),
                ("Bibliometrics", vec!["bibliometrics".to_string()]),
                ("Unrelated", vec!["earth".to_string()]),
            ] {
                let c = worker
                    .create_collection(NonEmptyStr::try_new(name).unwrap(), None)
                    .await
                    .unwrap();
                worker
                    .set_collection_classification(c.id, keywords, false)
                    .await
                    .unwrap();
                ids.push(c.id);
            }
            let mut new = crate::metadata::empty_resource(
                "Citation networks in art history",
                ResourceKind::Book,
            )
            .unwrap();
            new.keywords = vec!["Bibliometrics".into()];
            (worker.create_resource(new).await.unwrap().id, ids)
        });
        let first = worker
            .list_grid_page_blocking(
                ResourceGridParent::Roots,
                ResourceQuery::newest_first(),
                0,
                50,
            )
            .unwrap();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::bind_library_keys(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                let table = cx.new(|cx| {
                    TableState::new(
                        LibraryDelegate::new(worker.clone(), first, Settings::default()),
                        window,
                        cx,
                    )
                });
                cx.new(|cx| LibraryApp::new(table, dir.path().to_path_buf(), window, cx))
            })
            .unwrap()
        });
        Fixture {
            handle,
            app,
            worker,
            resource,
            collections,
            _dir: dir,
        }
    }

    fn settle(cx: &mut TestAppContext, worker: &DbWorker) {
        for _ in 0..8 {
            cx.run_until_parked();
            worker.list_page_blocking(None, 1).unwrap();
        }
        cx.run_until_parked();
    }

    fn select_first(f: &Fixture, cx: &mut TestAppContext) {
        cx.update_window(f.handle, |_, window, cx| {
            f.app.update(cx, |app, cx| {
                app.table
                    .update(cx, |table, cx| table.set_selected_row(0, cx));
                app.select_row(0, window, cx);
                app.focus_table(window, cx);
            });
        })
        .unwrap();
        settle(cx, &f.worker);
    }

    #[gpui_kit::test]
    fn details_series_create_select_save_and_cancel(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("edit-resource", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("edit-series", cx);
            window.input("Studies in bibliography", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("save-edit", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let series = rt.block_on(f.worker.list_series()).unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].title.as_str(), "Studies in bibliography");
        assert_eq!(
            rt.block_on(f.worker.resource(f.resource))
                .unwrap()
                .series_id,
            Some(series[0].id)
        );
        cx.update_window(f.handle, |_, window, cx| {
            window.click("edit-resource", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("edit-series", cx);
            window.input("Cancelled series", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("cancel-edit", cx);
        })
        .unwrap();
        assert_eq!(
            rt.block_on(f.worker.resource(f.resource))
                .unwrap()
                .series_id,
            Some(series[0].id)
        );
    }

    #[gpui_kit::test]
    fn details_all_fields_save_and_reopen(cx: &mut TestAppContext) {
        use crate::edit::{BibliographicField as F, prefill_draft};
        let f = fixture(cx);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("edit-resource", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        let values: Vec<_> = F::ALL
            .iter()
            .map(|field| {
                (
                    *field,
                    match field {
                        F::Published | F::Accessed | F::OriginalDate | F::EventDate => "2024-02-29",
                        F::Isbns => "9780306406157",
                        F::Issns => "2049-3630",
                        F::Isrns => "TEST-2024",
                        F::Pages => "12–19",
                        _ => "Edited value",
                    },
                )
            })
            .collect();
        for (field, value) in &values {
            cx.update_window(f.handle, |_, window, cx| {
                window.render_frame(cx);
                let id = gpui_kit::SharedString::from(format!("edit-field-{field:?}"));
                for _ in 0..60 {
                    let viewport = window.find("edit-panel").bounds();
                    if window.try_find(id.clone()).is_some_and(|s| {
                        s.visible()
                            && s.bounds().top() >= viewport.top()
                            && s.bounds().bottom() <= viewport.bottom()
                    }) {
                        break;
                    }
                    window.scroll(
                        "edit-panel",
                        gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                            gpui_kit::px(0.),
                            gpui_kit::px(-100.),
                        )),
                        cx,
                    );
                }
                window.click(id, cx);
                window.press("cmd-a", cx);
                window.input(value, cx);
                assert!(
                    window.find("save-edit").visible(),
                    "Save must stay visible while scrolling"
                );
            })
            .unwrap();
        }
        cx.update_window(f.handle, |_, window, cx| {
            window.click("save-edit", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let saved = rt.block_on(f.worker.resource(f.resource)).unwrap();
        let draft = prefill_draft(&saved);
        for (field, value) in values {
            assert_eq!(draft.fields[&field], value, "{field:?}");
        }
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("edit-resource", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        f.app.read_with(cx, |app, cx| {
            let Some(super::EditFetch::Ready(form)) = &app.edit else {
                panic!("form should reopen")
            };
            assert_eq!(form.draft(cx).fields, draft.fields);
        });
    }

    #[gpui_kit::test]
    fn details_creator_roles_credit_and_order_roundtrip(cx: &mut TestAppContext) {
        use ibex_core::{CreatorRole as R, CreatorSpec, Name};
        let f = fixture(cx);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let person =
            rt.block_on(f.worker.create_person(
                Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
            ))
            .unwrap();
        let mut new = rt.block_on(f.worker.resource(f.resource)).unwrap().to_new();
        new.creators = vec![
            CreatorSpec {
                person_id: person.id,
                role: R::Author,
                credited_as: Some(NonEmptyStr::try_new("A. L.").unwrap()),
            },
            CreatorSpec {
                person_id: person.id,
                role: R::Editor,
                credited_as: Some(NonEmptyStr::try_new("Ada").unwrap()),
            },
        ];
        rt.block_on(f.worker.update_resource(f.resource, new))
            .unwrap();
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("edit-resource", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("creator-credit-1", cx);
            window.press("cmd-a", cx);
            window.input("Augusta Ada", cx);
            window.click("creator-role-1", cx);
            window.input("Translator", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("creator-up-1", cx);
            window.click("save-edit", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        let saved = rt.block_on(f.worker.resource(f.resource)).unwrap().to_new();
        assert_eq!(
            saved.creators,
            vec![
                CreatorSpec {
                    person_id: person.id,
                    role: R::Translator,
                    credited_as: Some(NonEmptyStr::try_new("Augusta Ada").unwrap())
                },
                CreatorSpec {
                    person_id: person.id,
                    role: R::Author,
                    credited_as: Some(NonEmptyStr::try_new("A. L.").unwrap())
                },
            ]
        );
    }

    #[gpui_kit::test]
    fn details_favorite_is_staged_until_save(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        let rt = tokio::runtime::Runtime::new().unwrap();
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("edit-resource", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("edit-favorite", cx);
            window.click("cancel-edit", cx);
        })
        .unwrap();
        assert!(!rt.block_on(f.worker.resource(f.resource)).unwrap().favorite);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("edit-resource", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("edit-favorite", cx);
            window.click("save-edit", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        assert!(rt.block_on(f.worker.resource(f.resource)).unwrap().favorite);
    }

    #[gpui_kit::test]
    fn details_create_person_add_remove_preserves_all_other_roles(cx: &mut TestAppContext) {
        use ibex_core::{CreatorRole::*, CreatorSpec, Name};
        let f = fixture(cx);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let person =
            rt.block_on(f.worker.create_person(
                Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
            ))
            .unwrap();
        let mut new = rt.block_on(f.worker.resource(f.resource)).unwrap().to_new();
        let roles = [Author, Editor, Translator, Contributor, Host, Director];
        new.creators = roles
            .iter()
            .map(|role| CreatorSpec {
                person_id: person.id,
                role: *role,
                credited_as: Some(NonEmptyStr::try_new("A. L.").unwrap()),
            })
            .collect();
        let expected = new.creators[1..].to_vec();
        rt.block_on(f.worker.update_resource(f.resource, new))
            .unwrap();
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("edit-resource", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("creator-remove-0", cx);
            reveal_edit_control("add-creator-person", window, cx);
            window.click("add-creator-person", cx);
            window.input("Grace Hopper", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            reveal_edit_control("add-creator", window, cx);
            window.click("add-creator", cx);
            window.click("save-edit", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        let saved = rt.block_on(f.worker.resource(f.resource)).unwrap().to_new();
        assert_eq!(&saved.creators[..5], expected);
        assert_eq!(saved.creators.len(), 6);
        let people = rt.block_on(f.worker.list_persons()).unwrap();
        let grace = people
            .iter()
            .find(|p| p.name.display() == "Grace Hopper")
            .unwrap();
        assert_eq!(saved.creators[5].person_id, grace.id);
        assert_eq!(saved.creators[5].role, Author);
    }

    fn reveal_edit_control(id: &str, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
        let id = gpui_kit::SharedString::from(id.to_owned());
        window.render_frame(cx);
        for _ in 0..100 {
            let viewport = window.find("edit-panel").bounds();
            let target = window.find(id.clone());
            if target.visible()
                && target.bounds().top() >= viewport.top()
                && target.bounds().bottom() <= viewport.bottom()
            {
                return;
            }
            let delta = if target.bounds().top() < viewport.top() {
                100.
            } else {
                -100.
            };
            window.scroll(
                "edit-panel",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(delta),
                )),
                cx,
            );
        }
        panic!("Could not reveal {id}");
    }

    #[gpui_kit::test]
    fn details_multiline_rich_dates_validate_and_cancel(cx: &mut TestAppContext) {
        use crate::edit::{BibliographicField as F, prefill_draft};
        let f = fixture(cx);
        select_first(&f, cx);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let before = rt.block_on(f.worker.resource(f.resource)).unwrap().to_new();
        for save in [false, true] {
            cx.update_window(f.handle, |_, window, cx| {
                window.render_frame(cx);
                window.click("edit-resource", cx);
            })
            .unwrap();
            settle(cx, &f.worker);
            for (field, text) in [
                (F::AbstractText, "First paragraph\nSecond paragraph"),
                (F::Keywords, "alpha\nbeta, gamma"),
                (F::Published, "2023-02-29"),
            ] {
                cx.update_window(f.handle, |_, window, cx| {
                    let id = format!("edit-field-{field:?}");
                    reveal_edit_control(&id, window, cx);
                    window.click(gpui_kit::SharedString::from(id), cx);
                    window.press("cmd-a", cx);
                    for (ix, line) in text.split('\n').enumerate() {
                        if ix > 0 {
                            window.press("shift-enter", cx);
                        }
                        window.input(line, cx);
                    }
                })
                .unwrap();
            }
            cx.update_window(f.handle, |_, window, cx| {
                window.click("save-edit", cx);
            })
            .unwrap();
            settle(cx, &f.worker);
            f.app.read_with(cx, |app, _| {
                let Some(super::EditFetch::Ready(form)) = &app.edit else {
                    panic!("invalid date must keep the form open")
                };
                assert!(
                    form.error
                        .as_ref()
                        .is_some_and(|error| error.contains("Published"))
                );
            });
            assert_eq!(
                rt.block_on(f.worker.resource(f.resource)).unwrap().to_new(),
                before
            );
            cx.update_window(f.handle, |_, window, cx| {
                reveal_edit_control("edit-field-Published", window, cx);
                window.click("edit-field-Published", cx);
                window.press("cmd-a", cx);
                window.input("2024-02/2024-03~", cx);
                window.click(if save { "save-edit" } else { "cancel-edit" }, cx);
            })
            .unwrap();
            settle(cx, &f.worker);
            let saved = rt.block_on(f.worker.resource(f.resource)).unwrap();
            if save {
                assert_eq!(
                    saved.abstract_text.as_deref(),
                    Some("First paragraph\nSecond paragraph")
                );
                assert_eq!(saved.keywords, vec!["alpha", "beta, gamma"]);
                assert_eq!(
                    prefill_draft(&saved).fields[&F::Published],
                    "2024-02/2024-03~"
                );
            } else {
                assert_eq!(saved.to_new(), before);
            }
        }
    }

    #[gpui_kit::test]
    fn details_multiline_cmd_enter_saves_without_inserting_newline(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("edit-resource", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            reveal_edit_control("edit-field-AbstractText", window, cx);
            window.click("edit-field-AbstractText", cx);
            window.input("First", cx);
            window.press("enter", cx);
            window.input("Second", cx);
            window.press("cmd-enter", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(
            rt.block_on(f.worker.resource(f.resource))
                .unwrap()
                .abstract_text
                .as_deref(),
            Some("First\nSecond")
        );
        f.app.read_with(cx, |app, _| assert!(app.edit.is_none()));
    }

    #[gpui_kit::test]
    fn keyword_suggestions_accept_one_then_add_all(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        let rt = tokio::runtime::Runtime::new().unwrap();
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find(("suggestion", 0usize)).is_some());
            assert!(window.try_find(("suggestion", 1usize)).is_some());
            assert!(
                window.try_find(("suggestion", 2usize)).is_none(),
                "earth ≠ art"
            );
            assert!(window.try_find("detail-collections").is_none());
            window.click(("accept-suggestion", 0usize), cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        assert_eq!(
            rt.block_on(f.worker.resource(f.resource))
                .unwrap()
                .collection_ids,
            // Library order is by name: Bibliometrics, then Networks.
            vec![f.collections[1]]
        );
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("detail-collections").is_some());
            assert!(window.try_find(("suggestion", 0usize)).is_some());
            assert!(window.try_find(("suggestion", 1usize)).is_none());
            assert!(window.try_find("accept-all-suggestions").is_none());
            window.click(("accept-suggestion", 0usize), cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        let after = rt.block_on(f.worker.resource(f.resource)).unwrap();
        assert_eq!(after.collection_ids, [f.collections[1], f.collections[0]]);
        f.app.read_with(cx, |app, _| {
            let Some((_, DetailFetch::Loaded(detail))) = &app.detail else {
                panic!("detail should reload after accepting");
            };
            assert_eq!(detail.collections.len(), 2);
        });
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("detail-suggestions").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn accepting_suggestions_gates_inline_drafts_until_refresh_finishes(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(("accept-suggestion", 0usize), cx);
            f.app.read_with(cx, |app, _| {
                assert!(app.suggestion_pending.contains(&f.resource))
            });
            window.press("e", cx);
            f.app.read_with(cx, |app, cx| {
                assert!(
                    !app.has_inline(cx),
                    "acceptance must not open an editor it will discard"
                )
            });
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| window.press("e", cx))
            .unwrap();
        settle(cx, &f.worker);
        f.app.read_with(cx, |app, cx| assert!(app.has_inline(cx)));
        cx.update_window(f.handle, |_, window, cx| {
            window.press("cmd-a", cx);
            window.input("An edit after acceptance", cx);
            window.press("cmd-enter", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let saved = rt.block_on(f.worker.resource(f.resource)).unwrap();
        assert_eq!(saved.title.as_str(), "An edit after acceptance");
        assert_eq!(saved.collection_ids, vec![f.collections[1]]);
    }

    #[gpui_kit::test]
    fn attachment_detail_shows_ordered_files_and_named_open_buttons(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut ids = Vec::new();
        for name in ["first.pdf", "second.txt"] {
            let path = dir.path().join(name);
            std::fs::write(&path, b"file").unwrap();
            ids.push(
                rt.block_on(f.worker.add_attachment(f.resource, path))
                    .unwrap()
                    .id,
            );
        }
        cx.update_window(f.handle, |_, window, cx| {
            f.app.update(cx, |app, cx| {
                app.refresh_after_write(f.resource, window, cx)
            });
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find(format!("attachment-{}", ids[0])).label(),
                Some("first (pdf)")
            );
            assert_eq!(
                window.find(format!("attachment-{}", ids[1])).label(),
                Some("second (txt)")
            );
            assert_eq!(
                window.find(format!("open-attachment-{}", ids[0])).label(),
                Some("Open first")
            );
            assert_eq!(
                window.find(format!("open-attachment-{}", ids[1])).label(),
                Some("Open second")
            );
        })
        .unwrap();
        f.app.read_with(cx, |app, _| {
            let Some((_, DetailFetch::Loaded(detail))) = &app.detail else {
                panic!("detail should reload")
            };
            assert_eq!(
                detail
                    .attachments
                    .iter()
                    .map(|a| a.title.as_str())
                    .collect::<Vec<_>>(),
                ["first", "second"]
            );
        });
    }

    #[gpui_kit::test]
    fn renamed_membership_uses_fresh_forest_despite_cached_detail(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(("accept-suggestion", 0usize), cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            f.app.update(cx, |app, cx| {
                app.begin_collection_rename(f.collections[1], window, cx);
            });
            window.press("cmd-a", cx);
            window.input("Renamed collection", cx);
            window.press("enter", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find("detail-collections").label(),
                Some("Renamed collection")
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn deleted_membership_disappears_from_cached_detail(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(("accept-suggestion", 0usize), cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("detail-collections").is_some());
            f.app.update(cx, |app, cx| {
                app.delete_sidebar_collection(f.collections[1], window, cx)
            });
        })
        .unwrap();
        settle(cx, &f.worker);
        // Reselect through the detail cache, not just the currently displayed projection.
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("detail-collections").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn add_all_accepts_every_suggestion_at_once(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("accept-all-suggestions", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(
            rt.block_on(f.worker.resource(f.resource))
                .unwrap()
                .collection_ids,
            [f.collections[1], f.collections[0]]
        );
    }

    #[gpui_kit::test]
    fn classification_screen_saves_rules_and_refreshes_suggestions(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        let unrelated = f.collections[2];
        cx.update_window(f.handle, |_, window, cx| {
            f.app.update(cx, |app, cx| {
                app.open_collection_classification(unrelated, window, cx)
            });
            window.render_frame(cx);
            assert!(window.try_find("palette-classification").is_some());
            window.click(("palette-row", 0usize), cx);
            window.input(", art history", cx);
            window.press("enter", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let forest = rt.block_on(f.worker.list_collection_forest()).unwrap();
        let saved = forest
            .iter()
            .find(|node| node.collection.id == unrelated)
            .unwrap();
        assert_eq!(saved.collection.keywords, vec!["earth", "art history"]);
        assert!(saved.collection.jev_enabled);
        cx.update_window(f.handle, |_, window, cx| {
            assert!(!window.has_active_dialog(cx));
            window.render_frame(cx);
            assert!(window.try_find(("suggestion", 2usize)).is_some());
        })
        .unwrap();
    }

    /// Opt collections into Jev (keeping their keyword rules) and refresh
    /// the sidebar forest the panel reads candidates from.
    fn enable_jev(f: &Fixture, cx: &mut TestAppContext, picks: &[(usize, &str)]) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let forest = rt.block_on(async {
            for (ix, keyword) in picks {
                f.worker
                    .set_collection_classification(
                        f.collections[*ix],
                        vec![(*keyword).to_owned()],
                        true,
                    )
                    .await
                    .unwrap();
            }
            f.worker.list_collection_forest().await.unwrap()
        });
        cx.update_window(f.handle, |_, window, cx| {
            f.app
                .update(cx, |app, cx| app.set_collection_forest(forest, window, cx));
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn set_key(f: &Fixture, cx: &mut TestAppContext) {
        let dir = f._dir.path().to_path_buf();
        f.app.update(cx, |app, cx| {
            app.table.update(cx, |table, _| {
                table
                    .delegate_mut()
                    .settings
                    .set_openrouter_api_key("sk-or-test", &dir)
                    .unwrap()
            })
        });
    }

    fn wait_for_jev(f: &Fixture, cx: &mut TestAppContext) {
        for _ in 0..300 {
            settle(cx, &f.worker);
            let landed = f.app.read_with(cx, |app, _| {
                app.jev
                    .as_ref()
                    .is_some_and(|jev| jev.status != JevStatus::Pending)
            });
            if landed {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("Jev reply never landed");
    }

    #[gpui_kit::test]
    fn unrelated_writes_preserve_jev_predictions(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        let expected = JevStatus::Ready(vec![(f.collections[2], 0.92)]);
        f.app.update(cx, |app, _| {
            app.jev_gen = 1;
            app.jev = Some(JevState {
                resource_id: f.resource,
                status: expected.clone(),
            });
        });
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            // Accept the Jev result, then toggle favorite and save a no-op edit.
            window.click(("accept-suggestion", 0usize), cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            f.app.update(cx, |app, cx| app.toggle_favorite(window, cx));
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| window.press("e", cx))
            .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| window.press("cmd-enter", cx))
            .unwrap();
        settle(cx, &f.worker);
        f.app.read_with(cx, |app, _| {
            assert_eq!(app.jev.as_ref().map(|jev| &jev.status), Some(&expected));
            assert_eq!(app.jev_gen, 1);
        });
    }

    #[gpui_kit::test]
    fn sidebar_content_edit_cancels_pending_jev(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        f.app.update(cx, |app, _| {
            app.jev_gen = 1;
            app.jev = Some(JevState {
                resource_id: f.resource,
                status: JevStatus::Pending,
            });
        });
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("edit-resource", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.press("cmd-a", cx);
            window.input("New sidebar title", cx);
            window.press("cmd-enter", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(
            rt.block_on(f.worker.resource(f.resource))
                .unwrap()
                .title
                .as_str(),
            "New sidebar title"
        );
        f.app.update(cx, |app, cx| {
            assert!(app.jev.is_none());
            app.finish_jev(1, f.resource, Ok(vec![(f.collections[2], 0.92)]), cx);
            assert!(app.jev.is_none());
        });
    }

    #[gpui_kit::test]
    fn all_jev_candidates_already_joined_explains_without_a_request(cx: &mut TestAppContext) {
        let f = fixture(cx);
        enable_jev(&f, cx, &[(1, "bibliometrics")]);
        set_key(&f, cx);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(("accept-suggestion", 0usize), cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| window.press("cmd-j", cx))
            .unwrap();
        f.app.read_with(cx, |app, _| {
            assert!(matches!(&app.jev, Some(JevState { status: JevStatus::Failed(error), .. }) if error.contains("already belongs to every collection")));
        });
    }

    #[gpui_kit::test]
    fn inline_content_edit_invalidates_jev_and_rejects_old_reply(cx: &mut TestAppContext) {
        let f = fixture(cx);
        enable_jev(&f, cx, &[(2, "earth")]);
        set_key(&f, cx);
        let body = serde_json::json!({"answers": {f.collections[2].to_string(): {"type": "noul", "noul": 0.92}}}).to_string();
        let (url, server) = crate::jev::tests::serve(body, "200 OK");
        f.app.update(cx, |app, _| app.jev_endpoint = url);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| window.press("cmd-j", cx))
            .unwrap();
        wait_for_jev(&f, cx);
        server.join().unwrap();
        let generation = f.app.read_with(cx, |app, _| app.jev_gen);
        cx.update_window(f.handle, |_, window, cx| window.press("e", cx))
            .unwrap();
        settle(cx, &f.worker);
        cx.update_window(f.handle, |_, window, cx| {
            window.press("cmd-a", cx);
            window.input("A completely different subject", cx);
            window.press("cmd-enter", cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(
            rt.block_on(f.worker.resource(f.resource))
                .unwrap()
                .title
                .as_str(),
            "A completely different subject"
        );
        f.app.update(cx, |app, cx| {
            assert!(
                app.jev.is_none(),
                "old-content predictions must be invalidated"
            );
            app.finish_jev(
                generation,
                f.resource,
                Ok(vec![(f.collections[2], 0.92)]),
                cx,
            );
            assert!(
                app.jev.is_none(),
                "an old-content reply must not resurrect predictions"
            );
        });
    }

    #[gpui_kit::test]
    fn jev_ranks_new_candidates_before_applying_top_five(cx: &mut TestAppContext) {
        let mut f = fixture(cx);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            for name in ["Fourth", "Fifth", "Sixth"] {
                f.collections.push(
                    f.worker
                        .create_collection(NonEmptyStr::try_new(name).unwrap(), None)
                        .await
                        .unwrap()
                        .id,
                );
            }
            let record = f.worker.resource(f.resource).await.unwrap();
            let mut new = record.to_new();
            new.collection_ids = f.collections[..5].to_vec();
            f.worker.update_resource(f.resource, new).await.unwrap();
        });
        enable_jev(
            &f,
            cx,
            &[(0, "a"), (1, "b"), (2, "c"), (3, "d"), (4, "e"), (5, "f")],
        );
        set_key(&f, cx);
        let answers: serde_json::Map<_, _> = f
            .collections
            .iter()
            .enumerate()
            .map(|(ix, id)| {
                (
                    id.to_string(),
                    serde_json::json!({"type": "noul", "noul": 0.99 - ix as f32 * 0.05}),
                )
            })
            .collect();
        let (url, server) = crate::jev::tests::serve(
            serde_json::json!({"answers": answers}).to_string(),
            "200 OK",
        );
        f.app.update(cx, |app, _| app.jev_endpoint = url);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| window.press("cmd-j", cx))
            .unwrap();
        wait_for_jev(&f, cx);
        let request = server.join().unwrap();
        f.app.read_with(cx, |app, _| {
            let JevStatus::Ready(picks) = &app.jev.as_ref().unwrap().status else {
                panic!("expected picks")
            };
            assert_eq!(
                picks.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
                vec![f.collections[5]]
            );
        });
        for member in &f.collections[..5] {
            assert!(!request.contains(&member.to_string()));
        }
    }

    #[gpui_kit::test]
    fn cmd_j_ranks_jev_picks_first_and_accepts_them(cx: &mut TestAppContext) {
        let f = fixture(cx);
        enable_jev(&f, cx, &[(0, "citation networks"), (2, "earth")]);
        set_key(&f, cx);
        let unrelated = f.collections[2];
        let body = serde_json::json!({"answers": {
            unrelated.to_string(): {"type": "noul", "noul": 0.92},
            f.collections[0].to_string(): {"type": "noul", "noul": 0.3},
        }})
        .to_string();
        let (url, server) = crate::jev::tests::serve(body, "200 OK");
        f.app.update(cx, |app, _| app.jev_endpoint = url);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            assert_ne!(window.find("classify-jev").disabled(), Some(true));
            window.press("cmd-j", cx);
            window.render_frame(cx);
            assert!(window.try_find("jev-pending").is_some());
            assert_eq!(window.find("classify-jev").disabled(), Some(true));
        })
        .unwrap();
        // The request leaves only once the foreground task runs.
        wait_for_jev(&f, cx);
        let request = server.join().unwrap();
        assert!(request.contains("sk-or-test"));
        assert!(request.contains("Citation networks in art history"));
        f.app.read_with(cx, |app, _| {
            assert_eq!(
                app.jev.as_ref().map(|jev| &jev.status),
                Some(&JevStatus::Ready(vec![(unrelated, 0.92)]))
            );
        });
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            // Jev's pick first, then the two keyword matches.
            assert!(window.try_find(("suggestion", 2usize)).is_some());
            assert!(window.try_find("jev-error").is_none());
            window.click(("accept-suggestion", 0usize), cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(
            rt.block_on(f.worker.resource(f.resource))
                .unwrap()
                .collection_ids,
            vec![unrelated]
        );
    }

    #[gpui_kit::test]
    fn cmd_j_without_a_key_explains_instead_of_calling_jev(cx: &mut TestAppContext) {
        let f = fixture(cx);
        enable_jev(&f, cx, &[(0, "citation networks")]);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find("classify-jev").disabled(), Some(true));
            window.press("cmd-j", cx);
            window.render_frame(cx);
            assert!(window.try_find("jev-error").is_some());
        })
        .unwrap();
        f.app.read_with(cx, |app, _| {
            let Some(JevState {
                status: JevStatus::Failed(error),
                ..
            }) = &app.jev
            else {
                panic!("expected a failure");
            };
            assert!(error.contains("OpenRouter API key"));
        });
    }

    #[gpui_kit::test]
    fn cmd_j_without_jev_collections_reports_it(cx: &mut TestAppContext) {
        let f = fixture(cx);
        set_key(&f, cx);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.press("cmd-j", cx);
        })
        .unwrap();
        f.app.read_with(cx, |app, _| {
            assert!(matches!(
                &app.jev,
                Some(JevState { status: JevStatus::Failed(error), .. })
                    if error.contains("No collections are included")
            ));
        });
    }

    #[gpui_kit::test]
    fn off_selection_jev_reply_finishes_before_returning(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        f.app.update(cx, |app, cx| {
            app.jev_gen = 1;
            app.jev = Some(JevState {
                resource_id: f.resource,
                status: JevStatus::Pending,
            });
            app.selected = Some(ResourceId::generate());
            app.finish_jev(1, f.resource, Ok(vec![(f.collections[0], 0.8)]), cx);
            app.selected = Some(f.resource);
        });
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("jev-pending").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn stale_jev_replies_are_dropped(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        let pick = vec![(f.collections[0], 0.8)];
        f.app.update(cx, |app, cx| {
            app.jev_gen = 2;
            app.jev = Some(JevState {
                resource_id: f.resource,
                status: JevStatus::Pending,
            });
            app.finish_jev(1, f.resource, Ok(pick.clone()), cx);
            assert_eq!(app.jev.as_ref().unwrap().status, JevStatus::Pending);
            app.finish_jev(2, ResourceId::generate(), Ok(pick.clone()), cx);
            assert_eq!(app.jev.as_ref().unwrap().status, JevStatus::Pending);
            app.finish_jev(2, f.resource, Ok(pick.clone()), cx);
            assert_eq!(app.jev.as_ref().unwrap().status, JevStatus::Ready(pick));
        });
    }

    #[gpui_kit::test]
    fn palette_saves_the_api_key_and_model_to_config(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        for (row, value) in [(8usize, "sk-or-v1-saved"), (9, "typesafe/jev-2")] {
            cx.update_window(f.handle, |_, window, cx| {
                let palette = f
                    .app
                    .update(cx, |app, cx| app.open_palette(window, cx))
                    .unwrap();
                window.render_frame(cx);
                // Rows past the list's max height are scrolled out of view.
                palette.update(cx, |palette, cx| palette.confirm(row, window, cx));
                window.render_frame(cx);
                window.input(value, cx);
                window.press("enter", cx);
            })
            .unwrap();
            settle(cx, &f.worker);
            cx.update_window(f.handle, |_, window, cx| {
                assert!(!window.has_active_dialog(cx));
            })
            .unwrap();
        }
        let saved = Settings::load(f._dir.path());
        assert_eq!(
            saved.openrouter_api_key.as_ref().map(|k| k.expose()),
            Some("sk-or-v1-saved")
        );
        assert_eq!(saved.jev_model(), "typesafe/jev-2");
        f.app.read_with(cx, |app, cx| {
            assert_eq!(app.table.read(cx).delegate().settings, saved);
        });
    }

    #[gpui_kit::test]
    fn rules_window_saves_each_row_and_keeps_the_library_in_sync(cx: &mut TestAppContext) {
        let f = fixture(cx);
        select_first(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            f.app
                .update(cx, |app, cx| app.open_classification_rules(window, cx));
        })
        .unwrap();
        let (rules_handle, rules) = f
            .app
            .read_with(cx, |app, _| app.rules_window.clone().unwrap());
        cx.update_window(f.handle, |_, window, cx| {
            f.app
                .update(cx, |app, cx| app.open_classification_rules(window, cx));
        })
        .unwrap();
        f.app.read_with(cx, |app, _| {
            assert_eq!(app.rules_window.as_ref().unwrap().0, rules_handle, "reused");
        });
        let unrelated = f.collections[2];
        // Rows are ordered by name: Bibliometrics, Networks, Unrelated.
        let edit = |cx: &mut TestAppContext, value: String| {
            cx.update_window(rules_handle, |_, window, cx| {
                let input = rules.read(cx).row_state(2).unwrap().1.clone();
                assert_eq!(rules.read(cx).row_state(2).unwrap().0.id, unrelated);
                input.update(cx, |state, cx| state.set_value(value, window, cx));
                gpui_kit::Focusable::focus_handle(&input, cx).focus(window, cx);
                window.press("enter", cx);
            })
            .unwrap();
            settle(cx, &f.worker);
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let stored = |rt: &tokio::runtime::Runtime| {
            rt.block_on(f.worker.list_collection_forest())
                .unwrap()
                .into_iter()
                .find(|node| node.collection.id == unrelated)
                .unwrap()
                .collection
        };

        edit(cx, "x".repeat(200));
        assert!(rules.read_with(cx, |rules, _| rules.row_state(2).unwrap().2.is_some()));
        assert_eq!(stored(&rt).keywords, vec!["earth"]);

        edit(cx, " earth ,  art history".into());
        let saved = stored(&rt);
        assert_eq!(saved.keywords, vec!["earth", "art history"]);
        assert!(!saved.jev_enabled);
        cx.update_window(rules_handle, |_, _, cx| {
            let rules = rules.read(cx);
            let (_, input, error) = rules.row_state(2).unwrap();
            assert!(error.is_none());
            assert_eq!(input.read(cx).value().as_ref(), "earth, art history");
        })
        .unwrap();
        f.app.read_with(cx, |app, _| {
            let library = flatten_collections(&app.collection_forest);
            let synced = library.iter().find(|c| c.id == unrelated).unwrap();
            assert_eq!(synced.keywords, saved.keywords);
        });
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find(("suggestion", 2usize)).is_some());
        })
        .unwrap();

        cx.update_window(rules_handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(("rule-auto", 2usize), cx);
        })
        .unwrap();
        settle(cx, &f.worker);
        let saved = stored(&rt);
        assert!(saved.jev_enabled);
        assert_eq!(saved.keywords, vec!["earth", "art history"]);

        let added = rt
            .block_on(
                f.worker
                    .create_collection(NonEmptyStr::try_new("Added").unwrap(), None),
            )
            .unwrap();
        let forest = rt.block_on(f.worker.list_collection_forest()).unwrap();
        cx.update_window(f.handle, |_, window, cx| {
            f.app
                .update(cx, |app, cx| app.set_collection_forest(forest, window, cx));
        })
        .unwrap();
        rules.read_with(cx, |rules, _| {
            assert_eq!(rules.row_count(), 4);
            assert_eq!(rules.row_state(0).unwrap().0.id, added.id);
        });
    }
}
