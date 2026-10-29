//! Nested membership editors. Checked rows are the draft; only Apply persists its diff.
use std::collections::HashSet;

use crate::settings::{ThemeColorTarget, parse_hex_color};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::Cancel;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use oeuvre_core::{Collection, CollectionId, Resource, ResourceId, Tag, TagId};

#[derive(Clone, Debug)]
pub struct SetThemeColor {
    pub target: ThemeColorTarget,
    pub hex: String,
}

#[derive(Clone, Debug)]
pub enum PaletteRequest {
    /// Resource, additions, removals (relative to the opening snapshot).
    EditTags(ResourceId, Vec<TagId>, Vec<TagId>),
    EditCollections(ResourceId, Vec<CollectionId>, Vec<CollectionId>),
    /// Targets, memberships to add to all, memberships to remove from all.
    EditTagsBulk(Vec<ResourceId>, Vec<TagId>, Vec<TagId>),
    EditCollectionsBulk(Vec<ResourceId>, Vec<CollectionId>, Vec<CollectionId>),
}

impl PaletteRequest {
    pub fn resource_id(&self) -> ResourceId {
        match self {
            Self::EditTags(id, _, _) | Self::EditCollections(id, _, _) => *id,
            Self::EditTagsBulk(ids, _, _) | Self::EditCollectionsBulk(ids, _, _) => {
                *ids.first().expect("bulk requests require a target")
            }
        }
    }

    pub fn resource_ids(&self) -> Vec<ResourceId> {
        match self {
            Self::EditTags(id, _, _) | Self::EditCollections(id, _, _) => vec![*id],
            Self::EditTagsBulk(ids, _, _) | Self::EditCollectionsBulk(ids, _, _) => ids.clone(),
        }
    }
}

/// Library-wide view preference; independent of resource membership edits.
pub struct ToggleResourceGrouping;

#[derive(Clone, Debug)]
pub struct IdentifierRequest {
    pub identifier: String,
    pub target: Option<ResourceId>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Commands,
    Tags,
    Collections,
    Identifier,
    Color(ThemeColorTarget),
}

pub struct CommandPalette {
    state: Entity<InputState>,
    highlighted: Option<usize>,
    scroll: ScrollHandle,
    targets: Vec<ResourceId>,
    title: String,
    screen: Screen,
    tags: Vec<Tag>,
    collections: Vec<Collection>,
    existing_tags: HashSet<TagId>,
    existing_collections: HashSet<CollectionId>,
    selected_tags: HashSet<TagId>,
    selected_collections: HashSet<CollectionId>,
    // Existing/selected contain the union; unresolved mixed rows are not edits.
    original_mixed_tags: HashSet<TagId>,
    original_mixed_collections: HashSet<CollectionId>,
    mixed_tags: HashSet<TagId>,
    mixed_collections: HashSet<CollectionId>,
    ready: bool,
    grouped: bool,
    saving: bool,
    // A partially persisted draft may only be retried unchanged or dismissed.
    submitted: bool,
    error: Option<String>,
    identifier_target: Option<ResourceId>,
    suggested_identifier: Option<String>,
    colors: [String; 2],
}

impl EventEmitter<PaletteRequest> for CommandPalette {}
impl EventEmitter<ToggleResourceGrouping> for CommandPalette {}
impl EventEmitter<IdentifierRequest> for CommandPalette {}
impl EventEmitter<SetThemeColor> for CommandPalette {}

impl CommandPalette {
    pub fn set_colors(&mut self, accent: String, secondary: String) {
        self.colors = [accent, secondary];
    }

    pub fn open_color(
        &mut self,
        target: ThemeColorTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate(Screen::Color(target), window, cx);
        let value = self.colors[if target == ThemeColorTarget::Accent {
            0
        } else {
            1
        }]
        .clone();
        self.state
            .update(cx, |state, cx| state.set_value(value, window, cx));
    }

    fn apply_color(&mut self, cx: &mut Context<Self>) {
        let Screen::Color(target) = self.screen else {
            return;
        };
        if self.saving {
            return;
        }
        let hex = self.state.read(cx).value().trim().to_owned();
        if let Err(error) = parse_hex_color(&hex) {
            self.error = Some(error.into());
            cx.notify();
            return;
        }
        self.saving = true;
        self.error = None;
        cx.emit(SetThemeColor { target, hex });
        cx.notify();
    }
    pub fn new(
        target: Option<ResourceId>,
        grouped: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_for_targets(target.into_iter().collect(), grouped, window, cx)
    }

    pub fn new_for_targets(
        mut targets: Vec<ResourceId>,
        grouped: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut seen = HashSet::new();
        targets.retain(|id| seen.insert(*id));
        ensure_palette_bindings(cx);
        let state = cx.new(|cx| InputState::new(window, cx).placeholder("Type a command…"));
        cx.subscribe_in(&state, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.highlighted = this.visible_options(cx).first().map(|option| option.0);
                this.scroll.set_offset(point(px(0.), px(0.)));
                cx.notify();
            }
        })
        .detach();
        Self {
            state,
            highlighted: Some(0),
            scroll: ScrollHandle::new(),
            title: if !targets.is_empty() {
                "Loading resource…"
            } else {
                "Select a resource to edit tags or collections"
            }
            .into(),
            screen: Screen::Commands,
            targets,
            tags: Vec::new(),
            collections: Vec::new(),
            existing_tags: HashSet::new(),
            existing_collections: HashSet::new(),
            selected_tags: HashSet::new(),
            selected_collections: HashSet::new(),
            original_mixed_tags: HashSet::new(),
            original_mixed_collections: HashSet::new(),
            mixed_tags: HashSet::new(),
            mixed_collections: HashSet::new(),
            ready: false,
            grouped,
            saving: false,
            submitted: false,
            error: None,
            identifier_target: None,
            suggested_identifier: None,
            colors: [String::new(), String::new()],
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.state.focus_handle(cx).focus(window, cx);
    }

    pub fn loaded(
        &mut self,
        record: Resource,
        tags: Vec<Tag>,
        collections: Vec<Collection>,
        cx: &mut Context<Self>,
    ) {
        self.loaded_resources(vec![record], tags, collections, cx);
    }

    /// Load the complete opening snapshot; partial target loads cannot be edited.
    pub fn loaded_resources(
        &mut self,
        resources: Vec<Resource>,
        tags: Vec<Tag>,
        collections: Vec<Collection>,
        cx: &mut Context<Self>,
    ) {
        let records: Vec<_> = self
            .targets
            .iter()
            .filter_map(|id| resources.iter().find(|resource| resource.id == *id))
            .collect();
        if records.is_empty() || records.len() != self.targets.len() {
            self.ready = false;
            self.failed("Unable to load all selected resources".into(), cx);
            return;
        }
        self.title = if records.len() == 1 {
            records[0].title.as_str().to_owned()
        } else {
            format!("{} resources selected", records.len())
        };
        self.suggested_identifier = if records.len() == 1 {
            records[0].doi.clone().or_else(|| {
                records[0]
                    .isbns
                    .first()
                    .map(|isbn| isbn.as_str().to_owned())
            })
        } else {
            None
        };
        self.existing_tags = records
            .iter()
            .flat_map(|record| record.tag_ids.iter().copied())
            .collect();
        self.existing_collections = records
            .iter()
            .flat_map(|record| record.collection_ids.iter().copied())
            .collect();
        self.original_mixed_tags = self
            .existing_tags
            .iter()
            .copied()
            .filter(|id| !records.iter().all(|record| record.tag_ids.contains(id)))
            .collect();
        self.original_mixed_collections = self
            .existing_collections
            .iter()
            .copied()
            .filter(|id| {
                !records
                    .iter()
                    .all(|record| record.collection_ids.contains(id))
            })
            .collect();
        self.mixed_tags = self.original_mixed_tags.clone();
        self.mixed_collections = self.original_mixed_collections.clone();
        self.error = None;
        self.saving = false;
        self.selected_tags = self.existing_tags.clone();
        self.selected_collections = self.existing_collections.clone();
        self.tags = tags;
        self.collections = collections;
        self.ready = true;
        cx.notify();
    }

    pub fn failed(&mut self, error: String, cx: &mut Context<Self>) {
        if !self.ready {
            self.title = "Unable to load resource".into();
        }
        self.saving = false;
        self.error = Some(error);
        cx.notify();
    }

    fn navigate(&mut self, screen: Screen, window: &mut Window, cx: &mut Context<Self>) {
        self.screen = screen;
        self.error = None;
        self.state.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.set_placeholder(
                match screen {
                    Screen::Commands => "Type a command…",
                    Screen::Tags => "Search tags…",
                    Screen::Collections => "Search collections…",
                    Screen::Identifier => "Enter ISBN or DOI…",
                    Screen::Color(_) => "Enter #RRGGBB…",
                },
                window,
                cx,
            );
        });
        self.highlighted = Some(0);
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.focus(window, cx);
        cx.notify();
    }

    fn option_disabled(&self, row: usize) -> bool {
        self.saving
            || self.submitted
            || match row {
                2 | 3 | 5 | 6 if self.screen == Screen::Commands => false,
                4 if self.screen == Screen::Commands => !self.ready || self.targets.len() != 1,
                _ => !self.ready || self.targets.is_empty(),
            }
    }

    fn visible_options(&self, cx: &App) -> Vec<(usize, SharedString, bool)> {
        let query = self.state.read(cx).value().trim().to_lowercase();
        let options: Vec<_> = match self.screen {
            Screen::Commands => vec![
                (0, "Edit tags…".into(), false),
                (1, "Edit collections…".into(), false),
                (2, "Group children under parents".into(), self.grouped),
                (3, "Add by ISBN or DOI…".into(), false),
                (4, "Update metadata by ISBN or DOI…".into(), false),
                (5, "Set accent color…".into(), false),
                (6, "Set secondary color…".into(), false),
            ],
            Screen::Tags => self
                .tags
                .iter()
                .enumerate()
                .map(|(row, tag)| {
                    (
                        row,
                        SharedString::from(if self.mixed_tags.contains(&tag.id) {
                            format!("{} (some)", tag.text.as_str())
                        } else {
                            tag.text.as_str().to_owned()
                        }),
                        self.selected_tags.contains(&tag.id) && !self.mixed_tags.contains(&tag.id),
                    )
                })
                .collect(),
            Screen::Collections => self
                .collections
                .iter()
                .enumerate()
                .map(|(row, collection)| {
                    (
                        row,
                        SharedString::from(if self.mixed_collections.contains(&collection.id) {
                            format!("{} (some)", collection.name.as_str())
                        } else {
                            collection.name.as_str().to_owned()
                        }),
                        self.selected_collections.contains(&collection.id)
                            && !self.mixed_collections.contains(&collection.id),
                    )
                })
                .collect(),
            Screen::Identifier | Screen::Color(_) => vec![],
        };
        options
            .into_iter()
            .filter(|(row, label, _)| {
                label.to_lowercase().contains(&query)
                    || (self.screen == Screen::Commands
                        && *row == 2
                        && "toggle grouping flat nested tree".contains(&query))
            })
            .collect()
    }

    fn move_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let options = self.visible_options(cx);
        if options.is_empty() {
            return;
        }
        let current = options
            .iter()
            .position(|option| Some(option.0) == self.highlighted);
        let next = current.map_or(if delta < 0 { options.len() - 1 } else { 0 }, |current| {
            (current as isize + delta).rem_euclid(options.len() as isize) as usize
        });
        self.highlighted = Some(options[next].0);
        self.scroll.scroll_to_item(next);
        cx.notify();
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if self.screen == Screen::Commands || self.submitted {
            window.close_dialog(cx);
        } else {
            self.navigate(Screen::Commands, window, cx);
        }
    }

    fn confirm(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.option_disabled(row) {
            return;
        }
        match self.screen {
            Screen::Commands => match row {
                0 => self.navigate(Screen::Tags, window, cx),
                1 => self.navigate(Screen::Collections, window, cx),
                3 | 4 => {
                    self.identifier_target = if row == 4 {
                        self.targets.first().copied()
                    } else {
                        None
                    };
                    self.navigate(Screen::Identifier, window, cx);
                    if row == 4 {
                        if let Some(identifier) = &self.suggested_identifier {
                            self.state.update(cx, |state, cx| {
                                state.set_value(identifier.clone(), window, cx)
                            });
                        }
                    }
                }
                2 => cx.emit(ToggleResourceGrouping),
                5 => self.open_color(ThemeColorTarget::Accent, window, cx),
                6 => self.open_color(ThemeColorTarget::Secondary, window, cx),
                _ => {}
            },
            Screen::Tags => {
                if let Some(tag) = self.tags.get(row)
                    && !self.mixed_tags.remove(&tag.id)
                    && !self.selected_tags.insert(tag.id)
                {
                    self.selected_tags.remove(&tag.id);
                }
            }
            Screen::Collections => {
                if let Some(collection) = self.collections.get(row)
                    && !self.mixed_collections.remove(&collection.id)
                    && !self.selected_collections.insert(collection.id)
                {
                    self.selected_collections.remove(&collection.id);
                }
            }
            Screen::Identifier => self.apply_identifier(cx),
            Screen::Color(_) => self.apply_color(cx),
        }
        cx.notify();
    }

    fn change_count(&self) -> usize {
        match self.screen {
            Screen::Commands => 0,
            Screen::Identifier | Screen::Color(_) => 0,
            Screen::Tags => {
                self.selected_tags
                    .symmetric_difference(&self.existing_tags)
                    .count()
                    + self
                        .original_mixed_tags
                        .difference(&self.mixed_tags)
                        .filter(|id| self.selected_tags.contains(id))
                        .count()
            }
            Screen::Collections => {
                self.selected_collections
                    .symmetric_difference(&self.existing_collections)
                    .count()
                    + self
                        .original_mixed_collections
                        .difference(&self.mixed_collections)
                        .filter(|id| self.selected_collections.contains(id))
                        .count()
            }
        }
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        let Some(&id) = self.targets.first() else {
            return;
        };
        if !self.ready || self.saving || self.change_count() == 0 {
            return;
        }
        let request = match self.screen {
            Screen::Tags => PaletteRequest::EditTags(
                id,
                self.tags
                    .iter()
                    .filter(|tag| {
                        self.selected_tags.contains(&tag.id)
                            && (!self.existing_tags.contains(&tag.id)
                                || (self.original_mixed_tags.contains(&tag.id)
                                    && !self.mixed_tags.contains(&tag.id)))
                    })
                    .map(|tag| tag.id)
                    .collect(),
                self.existing_tags
                    .difference(&self.selected_tags)
                    .copied()
                    .collect(),
            ),
            Screen::Collections => PaletteRequest::EditCollections(
                id,
                self.collections
                    .iter()
                    .filter(|collection| {
                        self.selected_collections.contains(&collection.id)
                            && (!self.existing_collections.contains(&collection.id)
                                || (self.original_mixed_collections.contains(&collection.id)
                                    && !self.mixed_collections.contains(&collection.id)))
                    })
                    .map(|collection| collection.id)
                    .collect(),
                self.existing_collections
                    .difference(&self.selected_collections)
                    .copied()
                    .collect(),
            ),
            Screen::Commands | Screen::Identifier | Screen::Color(_) => return,
        };
        let request = if self.targets.len() > 1 {
            match request {
                PaletteRequest::EditTags(_, added, removed) => {
                    PaletteRequest::EditTagsBulk(self.targets.clone(), added, removed)
                }
                PaletteRequest::EditCollections(_, added, removed) => {
                    PaletteRequest::EditCollectionsBulk(self.targets.clone(), added, removed)
                }
                request => request,
            }
        } else {
            request
        };
        self.saving = true;
        self.submitted = true;
        self.error = None;
        cx.emit(request);
        cx.notify();
    }
}

impl CommandPalette {
    fn apply_identifier(&mut self, cx: &mut Context<Self>) {
        let identifier = self.state.read(cx).value().trim().to_owned();
        if identifier.is_empty() || self.saving {
            return;
        }
        self.saving = true;
        self.error = None;
        cx.emit(IdentifierRequest {
            identifier,
            target: self.identifier_target,
        });
        cx.notify();
    }

    pub fn identifier_failed(&mut self, message: String, cx: &mut Context<Self>) {
        self.saving = false;
        self.error = Some(message);
        cx.notify();
    }
}

gpui_kit::actions!([
    PaletteNext,
    PalettePrevious,
    PaletteConfirm,
    PaletteApply,
    PaletteBack
]);

struct PaletteBindings;
impl Global for PaletteBindings {}

fn ensure_palette_bindings(cx: &mut App) {
    if cx.has_global::<PaletteBindings>() {
        return;
    }
    cx.set_global(PaletteBindings);
    for context in ["ResourcePalette", "ResourcePalette > Input"] {
        cx.bind_keys([
            KeyBinding::new("down", PaletteNext, Some(context)),
            KeyBinding::new("up", PalettePrevious, Some(context)),
            KeyBinding::new("cmd-enter", PaletteApply, Some(context)),
            KeyBinding::new("escape", PaletteBack, Some(context)),
        ]);
    }
    cx.bind_keys([KeyBinding::new(
        "enter",
        PaletteConfirm,
        Some("ResourcePalette > Input"),
    )]);
}

impl CommandPalette {
    fn render_option(
        &self,
        row: usize,
        label: SharedString,
        checked: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let disabled = self.option_disabled(row);
        let highlighted = self.highlighted == Some(row) && !disabled;
        let mixed = match self.screen {
            Screen::Tags => self
                .tags
                .get(row)
                .is_some_and(|tag| self.mixed_tags.contains(&tag.id)),
            Screen::Collections => self
                .collections
                .get(row)
                .is_some_and(|collection| self.mixed_collections.contains(&collection.id)),
            Screen::Commands => false,
            Screen::Identifier | Screen::Color(_) => false,
        };
        h_flex()
            .id(("palette-row", row))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(label.clone())
            .aria_selected(highlighted)
            // Row geometry is independent of hover, focus, and membership.
            .w_full()
            .h_9()
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .when(highlighted, |row| {
                row.bg(cx.theme().accent)
                    .text_color(cx.theme().accent_foreground)
            })
            .when(disabled, |row| row.text_color(cx.theme().muted_foreground))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && !this.option_disabled(row) && this.highlighted != Some(row) {
                    this.highlighted = Some(row);
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.confirm(row, window, cx)))
            .child(
                div()
                    .id(("palette-label", row))
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .text_lg()
                    .truncate()
                    .child(label),
            )
            .child(
                div()
                    .size_4()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(checked, |slot| {
                        slot.child(Icon::new(IconName::Check).size_3())
                    })
                    .when(mixed, |slot| slot.child("—")),
            )
            .into_any_element()
    }
}

impl Render for CommandPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let options = self.visible_options(cx);
        let unavailable = !self.ready || self.saving || self.targets.is_empty();
        let heading = match self.screen {
            Screen::Commands => "Commands",
            Screen::Color(ThemeColorTarget::Accent) => "Commands / Set accent color",
            Screen::Color(ThemeColorTarget::Secondary) => "Commands / Set secondary color",
            Screen::Tags => "Commands / Edit tags",
            Screen::Collections => "Commands / Edit collections",
            Screen::Identifier => {
                if self.identifier_target.is_some() {
                    "Commands / Update metadata"
                } else {
                    "Commands / Add resource"
                }
            }
        };
        let list_height = (options.len().max(1) as f32 * 2.25).min(18.75);
        v_flex()
            .id("resource-palette")
            .test_support()
            .w_full()
            .text_lg()
            .key_context("ResourcePalette")
            .capture_action(cx.listener(|this, _: &Cancel, window, cx| this.back(window, cx)))
            .on_action(cx.listener(|this, _: &PaletteBack, window, cx| this.back(window, cx)))
            .on_action(cx.listener(|this, _: &PaletteNext, _, cx| this.move_highlight(1, cx)))
            .on_action(cx.listener(|this, _: &PalettePrevious, _, cx| this.move_highlight(-1, cx)))
            .on_action(cx.listener(|this, _: &PaletteApply, _, cx| {
                if matches!(this.screen, Screen::Color(_)) {
                    this.apply_color(cx)
                } else if this.screen == Screen::Identifier {
                    this.apply_identifier(cx)
                } else {
                    this.apply(cx)
                }
            }))
            .on_action(cx.listener(|this, _: &PaletteConfirm, window, cx| {
                if matches!(this.screen, Screen::Color(_)) {
                    this.apply_color(cx)
                } else if this.screen == Screen::Identifier {
                    this.apply_identifier(cx);
                } else if let Some(row) = this.highlighted {
                    this.confirm(row, window, cx);
                }
            }))
            .child(
                v_flex()
                    .px_3()
                    .py_2()
                    .gap_1()
                    .child(div().text_xl().child(heading))
                    .child(
                        div()
                            .text_base()
                            .text_color(cx.theme().muted_foreground)
                            .child(if matches!(self.screen, Screen::Color(_)) {
                                "Enter a hex color; changes are saved to config.toml".to_owned()
                            } else {
                                self.title.clone()
                            }),
                    ),
            )
            .child(
                div()
                    .px_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Input::new(&self.state)
                            .prefix(
                                Icon::new(IconName::Search).text_color(cx.theme().muted_foreground),
                            )
                            .appearance(false)
                            .p_0(),
                    ),
            )
            .child(
                v_flex()
                    .id(match self.screen {
                        Screen::Commands => "palette-commands",
                        Screen::Tags => "palette-tags",
                        Screen::Collections => "palette-collections",
                        Screen::Identifier => "palette-identifier",
                        Screen::Color(_) => "palette-color",
                    })
                    .test_support()
                    .role(Role::ListBox)
                    .w_full()
                    .h(rems(list_height))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .when(options.is_empty(), |list| {
                        list.child(
                            div()
                                .h_9()
                                .px_3()
                                .text_color(cx.theme().muted_foreground)
                                .child(if matches!(self.screen, Screen::Color(_)) {
                                    "Enter #RRGGBB, then press Enter to save."
                                } else if self.screen == Screen::Identifier {
                                    if self.identifier_target.is_some() {
                                        "Enter an ISBN or DOI. Only missing fields will be filled."
                                    } else {
                                        "Enter a valid ISBN or DOI, then press Enter."
                                    }
                                } else {
                                    "No matching options"
                                }),
                        )
                    })
                    .children(
                        options.into_iter().map(|(row, label, checked)| {
                            self.render_option(row, label, checked, cx)
                        }),
                    ),
            )
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_base()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .justify_between()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("palette-back")
                            .ghost()
                            .text_base()
                            .label(if self.screen == Screen::Commands || self.submitted {
                                "Cancel"
                            } else {
                                "Back"
                            })
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.screen == Screen::Commands || this.submitted {
                                    window.close_dialog(cx);
                                } else {
                                    this.navigate(Screen::Commands, window, cx);
                                }
                            })),
                    )
                    .when(self.screen != Screen::Commands, |el| {
                        el.child(
                            div()
                                .text_base()
                                .text_color(cx.theme().muted_foreground)
                                .child(if self.screen == Screen::Identifier {
                                    "↵ Look up"
                                } else if matches!(self.screen, Screen::Color(_)) {
                                    "↵ Save · Esc Back"
                                } else if self.submitted {
                                    "⌘↵ Retry · Esc Cancel"
                                } else {
                                    "↵ Toggle · ⌘↵ Apply"
                                }),
                        )
                        .child(
                            Button::new("palette-apply")
                                .primary()
                                .text_base()
                                .label(if self.saving {
                                    "Saving…".to_owned()
                                } else if matches!(self.screen, Screen::Color(_)) {
                                    "Save".to_owned()
                                } else if self.screen == Screen::Identifier {
                                    "Look up".to_owned()
                                } else if self.submitted {
                                    "Retry".to_owned()
                                } else {
                                    format!("Apply ({})", self.change_count())
                                })
                                .disabled(
                                    if self.screen == Screen::Identifier
                                        || matches!(self.screen, Screen::Color(_))
                                    {
                                        self.saving || self.state.read(cx).value().trim().is_empty()
                                    } else {
                                        unavailable || self.change_count() == 0
                                    },
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if matches!(this.screen, Screen::Color(_)) {
                                        this.apply_color(cx)
                                    } else if this.screen == Screen::Identifier {
                                        this.apply_identifier(cx)
                                    } else {
                                        this.apply(cx)
                                    }
                                })),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandPalette, IdentifierRequest, PaletteRequest, Screen};
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, px, size};
    use gpui_kit::{Context, IntoElement, ParentElement as _, Render, Styled, Window, div};
    use oeuvre_core::{
        Collection, CollectionId, NewResource, NonEmptyStr, Resource, ResourceId, ResourceKind,
        Tag, TagId,
    };
    use std::{cell::RefCell, rc::Rc};

    struct Fixture {
        window: AnyWindowHandle,
        palette: Entity<CommandPalette>,
        tags: Vec<TagId>,
        collections: Vec<CollectionId>,
        requests: Rc<RefCell<Vec<PaletteRequest>>>,
    }

    struct PaletteHost(Entity<CommandPalette>);
    impl Render for PaletteHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full()
        }
    }

    fn open(cx: &mut TestAppContext, selected: bool, loaded: bool) -> Fixture {
        open_with_bulk(cx, selected, loaded, false)
    }

    fn open_with_bulk(
        cx: &mut TestAppContext,
        selected: bool,
        loaded: bool,
        bulk: bool,
    ) -> Fixture {
        let tags: Vec<_> = ["Existing", "Methods", "Research"]
            .into_iter()
            .map(|text| Tag::try_new(TagId::generate(), NonEmptyStr::try_new(text).unwrap()))
            .collect();
        let collections: Vec<_> = ["Existing", "Reading", "Writing"]
            .into_iter()
            .map(|name| {
                Collection::try_new(
                    CollectionId::generate(),
                    NonEmptyStr::try_new(name).unwrap(),
                )
            })
            .collect();
        let tag_ids = tags.iter().map(|tag| tag.id).collect();
        let collection_ids = collections.iter().map(|collection| collection.id).collect();
        let id = ResourceId::generate();
        let mut record = Resource::try_new(
            id,
            NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new("Selected book").unwrap(),
                creators: vec![],
                publisher: None,
                published: None,
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
                tag_ids: vec![tags[0].id],
                collection_ids: vec![collections[0].id],
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
            },
        )
        .unwrap();
        if bulk {
            record.tag_ids.push(tags[2].id);
            record.collection_ids.push(collections[2].id);
        }
        let mut second = record.clone();
        second.id = ResourceId::generate();
        second.tag_ids = vec![tags[2].id];
        second.collection_ids = vec![collections[2].id];
        let requests = Rc::new(RefCell::new(Vec::new()));
        let sink = requests.clone();
        let (window, host) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(
                gpui_kit::WindowOptions {
                    window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                        origin: Default::default(),
                        size: size(px(640.), px(480.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let palette = cx.new(|cx| {
                        let mut palette = if bulk {
                            CommandPalette::new_for_targets(vec![id, second.id], true, window, cx)
                        } else {
                            CommandPalette::new(selected.then_some(id), true, window, cx)
                        };
                        if loaded {
                            if bulk {
                                palette.loaded_resources(
                                    vec![record, second],
                                    tags,
                                    collections,
                                    cx,
                                );
                            } else {
                                palette.loaded(record, tags, collections, cx);
                            }
                        }
                        palette
                    });
                    cx.new(|_| PaletteHost(palette))
                },
            )
            .unwrap()
        });
        let palette = cx.update(|cx| host.read(cx).0.clone());
        cx.update_window(window, |_, window, cx| {
            palette.update(cx, |_, cx| {
                cx.subscribe(&palette, move |_, _, request: &PaletteRequest, _| {
                    sink.borrow_mut().push(request.clone())
                })
                .detach();
            });
            let content = palette.clone();
            let initial_focus = std::cell::Cell::new(true);
            window.open_dialog(cx, move |dialog, window, cx| {
                if initial_focus.replace(false) {
                    content.update(cx, |palette, cx| palette.focus(window, cx));
                }
                dialog
                    .close_button(false)
                    .overlay_closable(false)
                    .keyboard(false)
                    .entrance(gpui_kit::component::dialog::DialogEntrance::None)
                    .width(px(480.))
                    .p_0()
                    .child(content.clone())
            });
            window.render_frame(cx);
        })
        .unwrap();
        Fixture {
            window,
            palette,
            tags: tag_ids,
            collections: collection_ids,
            requests,
        }
    }

    #[gpui_kit::test]
    fn identifier_commands_are_available_in_the_right_context(cx: &mut TestAppContext) {
        let f = open(cx, false, false);
        cx.update_window(f.window, |_, window, cx| {
            assert!(!f.palette.read(cx).option_disabled(3));
            assert!(f.palette.read(cx).option_disabled(4));
            window.click(("palette-row", 3usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            f.palette.read_with(cx, |palette, _| palette.screen),
            Screen::Identifier
        );

        let f = open(cx, true, true);
        cx.update_window(f.window, |_, window, cx| {
            assert!(!f.palette.read(cx).option_disabled(4));
            window.click(("palette-row", 4usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(
            f.palette
                .read_with(cx, |palette, _| palette.identifier_target.is_some())
        );
    }

    #[gpui_kit::test]
    fn theme_color_commands_validate_input_without_a_resource(cx: &mut TestAppContext) {
        use super::SetThemeColor;
        use crate::settings::ThemeColorTarget;
        for (row, target) in [
            (5, ThemeColorTarget::Accent),
            (6, ThemeColorTarget::Secondary),
        ] {
            let f = open(cx, false, false);
            let requests = Rc::new(RefCell::new(Vec::<SetThemeColor>::new()));
            let sink = requests.clone();
            cx.update_window(f.window, |_, window, cx| {
                f.palette.update(cx, |_, cx| {
                    cx.subscribe(&f.palette, move |_, _, request: &SetThemeColor, _| {
                        sink.borrow_mut().push(request.clone());
                    })
                    .detach();
                });
                assert!(!f.palette.read(cx).option_disabled(row));
                window.click(("palette-row", row), cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(f.window, |_, window, cx| {
                assert_eq!(f.palette.read(cx).screen, Screen::Color(target));
                let state = f.palette.read(cx).state.clone();
                state.update(cx, |state, cx| state.set_value("not a color", window, cx));
                window.press("enter", cx);
            })
            .unwrap();
            cx.run_until_parked();
            assert!(requests.borrow().is_empty());
            assert!(
                f.palette
                    .read_with(cx, |palette, _| palette.error.is_some())
            );
            cx.update_window(f.window, |_, window, cx| {
                let state = f.palette.read(cx).state.clone();
                state.update(cx, |state, cx| state.set_value("#AB12EF", window, cx));
                window.click("palette-apply", cx);
            })
            .unwrap();
            cx.run_until_parked();
            let requests = requests.borrow();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].target, target);
            assert_eq!(requests[0].hex, "#AB12EF");
        }
    }

    #[gpui_kit::test]
    fn adding_identifier_emits_the_lookup_request(cx: &mut TestAppContext) {
        let f = open(cx, false, false);
        let requests = Rc::new(RefCell::new(Vec::<IdentifierRequest>::new()));
        let sink = requests.clone();
        cx.update_window(f.window, |_, window, cx| {
            f.palette.update(cx, |_, cx| {
                cx.subscribe(&f.palette, move |_, _, request: &IdentifierRequest, _| {
                    sink.borrow_mut().push(request.clone());
                })
                .detach();
            });
            window.click(("palette-row", 3usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| {
            let state = f.palette.read(cx).state.clone();
            state.update(cx, |state, cx| state.set_value("9780306406157", window, cx));
            window.click("palette-apply", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let requests = requests.borrow();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].identifier, "9780306406157");
        assert_eq!(requests[0].target, None);
    }

    #[gpui_kit::test]
    fn bulk_editors_preserve_untouched_mixed_membership(cx: &mut TestAppContext) {
        for collections in [false, true] {
            let f = open_with_bulk(cx, true, true, true);
            cx.update_window(f.window, |_, window, cx| {
                if collections {
                    window.press("down", cx);
                }
                window.press("enter", cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(f.window, |_, window, cx| {
                assert_eq!(f.palette.read(cx).change_count(), 0);
                assert!(
                    f.palette.read(cx).visible_options(cx)[0]
                        .1
                        .contains("(some)")
                );
                window.click("palette-apply", cx);
                window.press("cmd-enter", cx);
            })
            .unwrap();
            cx.run_until_parked();
            assert!(f.requests.borrow().is_empty());
            cx.update_window(f.window, |_, window, cx| {
                window.click(("palette-row", 1usize), cx)
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(f.window, |_, window, cx| window.click("palette-apply", cx))
                .unwrap();
            cx.run_until_parked();
            let requests = f.requests.borrow();
            if collections {
                assert!(
                    matches!(&requests[..], [PaletteRequest::EditCollectionsBulk(ids, added, removed)]
                if ids.len() == 2 && added == &vec![f.collections[1]] && removed.is_empty())
                );
            } else {
                assert!(
                    matches!(&requests[..], [PaletteRequest::EditTagsBulk(ids, added, removed)]
                if ids.len() == 2 && added == &vec![f.tags[1]] && removed.is_empty())
                );
            }
            assert_eq!(requests[0].resource_ids().len(), 2);
            assert_eq!(requests[0].resource_id(), requests[0].resource_ids()[0]);
        }
    }

    #[gpui_kit::test]
    fn bulk_mixed_rows_resolve_to_all_then_none(cx: &mut TestAppContext) {
        for collections in [false, true] {
            for remove in [false, true] {
                let f = open_with_bulk(cx, true, true, true);
                cx.update_window(f.window, |_, window, cx| {
                    if collections {
                        window.press("down", cx);
                    }
                    window.press("enter", cx);
                })
                .unwrap();
                cx.run_until_parked();
                cx.update_window(f.window, |_, window, cx| {
                    let options = f.palette.read(cx).visible_options(cx);
                    assert!(options[0].1.contains("(some)"));
                    assert!(!options[1].2);
                    assert!(options[2].2);
                    assert_eq!(f.palette.read(cx).change_count(), 0);
                    window.click(("palette-row", 0usize), cx);
                })
                .unwrap();
                cx.run_until_parked();
                cx.update_window(f.window, |_, window, cx| {
                    let options = f.palette.read(cx).visible_options(cx);
                    assert!(!options[0].1.contains("(some)"));
                    assert!(options[0].2);
                    assert_eq!(f.palette.read(cx).change_count(), 1);
                    if remove {
                        window.press("enter", cx);
                    }
                })
                .unwrap();
                cx.run_until_parked();
                // None/all rows still revert to no change after two toggles.
                for row in [1usize, 2] {
                    for _ in 0..2 {
                        cx.update_window(f.window, |_, window, cx| {
                            window.click(("palette-row", row), cx);
                        })
                        .unwrap();
                        cx.run_until_parked();
                    }
                }
                cx.update_window(f.window, |_, window, cx| {
                    assert_eq!(f.palette.read(cx).change_count(), 1);
                    assert_eq!(f.palette.read(cx).visible_options(cx)[0].2, !remove);
                    window.press("cmd-enter", cx);
                })
                .unwrap();
                cx.run_until_parked();
                let requests = f.requests.borrow();
                assert_eq!(requests.len(), 1);
                match &requests[0] {
                    PaletteRequest::EditTagsBulk(ids, added, removed) if !collections => {
                        assert_eq!(ids.len(), 2);
                        assert_eq!(added, &if remove { vec![] } else { vec![f.tags[0]] });
                        assert_eq!(removed, &if remove { vec![f.tags[0]] } else { vec![] });
                    }
                    PaletteRequest::EditCollectionsBulk(ids, added, removed) if collections => {
                        assert_eq!(ids.len(), 2);
                        assert_eq!(
                            added,
                            &if remove {
                                vec![]
                            } else {
                                vec![f.collections[0]]
                            }
                        );
                        assert_eq!(
                            removed,
                            &if remove {
                                vec![f.collections[0]]
                            } else {
                                vec![]
                            }
                        );
                    }
                    request => panic!("unexpected request: {request:?}"),
                }
            }
        }
    }

    #[gpui_kit::test]
    fn grouping_remains_available_without_loaded_targets(cx: &mut TestAppContext) {
        let f = open(cx, false, false);
        let toggles = Rc::new(RefCell::new(0));
        let sink = toggles.clone();
        cx.update_window(f.window, |_, window, cx| {
            f.palette.update(cx, |_, cx| {
                cx.subscribe(
                    &f.palette,
                    move |_, _, _: &super::ToggleResourceGrouping, _| {
                        *sink.borrow_mut() += 1;
                    },
                )
                .detach();
            });
            window.input("group", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| window.press("enter", cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(*toggles.borrow(), 1);
        assert!(f.requests.borrow().is_empty());
    }

    #[gpui_kit::test]
    fn incomplete_bulk_snapshot_disables_editing(cx: &mut TestAppContext) {
        let f = open_with_bulk(cx, true, true, true);
        cx.update_window(f.window, |_, window, cx| {
            f.palette.update(cx, |palette, cx| {
                palette.loaded_resources(vec![], vec![], vec![], cx);
            });
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, _, cx| {
            assert!(!f.palette.read(cx).ready);
            assert!(f.palette.read(cx).screen == Screen::Commands);
        })
        .unwrap();
        assert!(f.requests.borrow().is_empty());
    }
    #[gpui_kit::test]
    fn nested_tags_search_toggle_and_apply(cx: &mut TestAppContext) {
        let f = open(cx, true, true);
        cx.update_window(f.window, |_, window, cx| {
            window.input("edit tags", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| {
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| {
            assert!(f.palette.read(cx).screen == Screen::Tags);
            assert!(f.requests.borrow().is_empty());
            window.input("research", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| {
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| {
            // Filtered row zero must still identify Research, not Existing.
            assert!(f.palette.read(cx).selected_tags.contains(&f.tags[2]));
            assert!(f.requests.borrow().is_empty());
            window.click("palette-apply", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(
            matches!(&f.requests.borrow()[..], [PaletteRequest::EditTags(_, added, removed)] if added == &vec![f.tags[2]] && removed.is_empty())
        );
    }

    #[gpui_kit::test]
    fn nested_collections_multi_select_and_keyboard_apply(cx: &mut TestAppContext) {
        let f = open(cx, true, true);
        cx.update_window(f.window, |_, window, cx| {
            window.input("collection", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| {
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| {
            assert!(f.palette.read(cx).screen == Screen::Collections);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| {
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| {
            assert_eq!(f.palette.read(cx).change_count(), 2);
            assert!(f.requests.borrow().is_empty());
            window.press("cmd-enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(
            matches!(&f.requests.borrow()[..], [PaletteRequest::EditCollections(_, added, removed)] if added == &vec![f.collections[1]] && removed == &vec![f.collections[0]])
        );
    }

    #[gpui_kit::test]
    fn escape_returns_from_search_without_applying(cx: &mut TestAppContext) {
        let f = open(cx, true, true);
        cx.update_window(f.window, |_, window, cx| window.press("enter", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| {
            window.input("research", cx);
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, _, cx| {
            assert!(f.palette.read(cx).screen == Screen::Commands);
            assert!(f.requests.borrow().is_empty());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn missing_selection_and_loading_disable_resource_commands(cx: &mut TestAppContext) {
        for selected in [false, true] {
            let f = open(cx, selected, false);
            cx.update_window(f.window, |_, window, cx| window.press("enter", cx))
                .unwrap();
            cx.run_until_parked();
            cx.update_window(f.window, |_, _, cx| {
                assert!(f.palette.read(cx).screen == Screen::Commands)
            })
            .unwrap();
            assert!(f.requests.borrow().is_empty());
        }
    }

    #[gpui_kit::test]
    fn loading_failure_keeps_commands_disabled(cx: &mut TestAppContext) {
        let f = open(cx, true, false);
        cx.update_window(f.window, |_, window, cx| {
            f.palette
                .update(cx, |palette, cx| palette.failed("load failed".into(), cx));
            window.render_frame(cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, _, cx| {
            assert!(f.palette.read(cx).screen == Screen::Commands)
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn save_failure_preserves_selection_for_retry_and_blocks_double_apply(cx: &mut TestAppContext) {
        let f = open(cx, true, true);
        cx.update_window(f.window, |_, window, cx| window.press("enter", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| window.press("enter", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| window.click("palette-apply", cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(f.requests.borrow().len(), 1);
        cx.update_window(f.window, |_, window, cx| {
            window.press("cmd-enter", cx);
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(f.requests.borrow().len(), 1);
        cx.update_window(f.window, |_, window, cx| {
            assert!(f.palette.read(cx).screen == Screen::Tags);
            f.palette.update(cx, |palette, cx| {
                palette.failed("Save failed; try again".into(), cx)
            });
            window.render_frame(cx);
            window.press("enter", cx);
            assert_eq!(
                f.palette.read(cx).change_count(),
                1,
                "failed saves must only retry the unchanged draft"
            );
            assert_eq!(f.palette.read(cx).change_count(), 1);
            window.click("palette-apply", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(f.requests.borrow().len(), 2);
        assert!(
            matches!(&f.requests.borrow()[1], PaletteRequest::EditTags(_, added, removed) if added.is_empty() && removed == &vec![f.tags[0]])
        );
    }

    #[gpui_kit::test]
    fn editing_tags_can_uncheck_an_existing_membership(cx: &mut TestAppContext) {
        let f = open(cx, true, true);
        cx.update_window(f.window, |_, window, cx| window.press("enter", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, _, cx| {
            assert!(f.palette.read(cx).selected_tags.contains(&f.tags[0]));
            assert_eq!(f.palette.read(cx).change_count(), 0);
        })
        .unwrap();
        cx.update_window(f.window, |_, window, cx| window.input("Existing", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| window.press("enter", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| {
            assert!(f.palette.read(cx).selected_tags.is_empty());
            assert_eq!(f.palette.read(cx).change_count(), 1);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| {
            assert_eq!(f.palette.read(cx).change_count(), 0);
            window.click("palette-apply", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(
            f.requests.borrow().is_empty(),
            "reverting a toggle must not save"
        );
        cx.update_window(f.window, |_, window, cx| window.press("enter", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(f.window, |_, window, cx| window.click("palette-apply", cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            f.requests.borrow().len(),
            1,
            "unchecking an existing tag must be an applicable edit"
        );
        assert!(
            matches!(&f.requests.borrow()[..], [PaletteRequest::EditTags(_, added, removed)] if added.is_empty() && removed == &vec![f.tags[0]])
        );
    }

    #[gpui_kit::test]
    fn membership_row_geometry_is_stable_when_toggled(cx: &mut TestAppContext) {
        for collections in [false, true] {
            let f = open(cx, true, true);
            cx.update_window(f.window, |_, window, cx| {
                if collections {
                    window.press("down", cx);
                }
                window.press("enter", cx);
            })
            .unwrap();
            cx.run_until_parked();
            let before = cx
                .update_window(f.window, |_, window, cx| {
                    window.render_frame(cx);
                    let label = window.find(("palette-label", 0usize)).bounds();
                    let fill = window.find(("palette-row", 0usize)).bounds();
                    let palette = window.find("resource-palette").bounds();
                    assert_eq!(fill.origin.x, palette.origin.x);
                    assert_eq!(
                        fill.size.width, palette.size.width,
                        "the gray fill must cover the entire palette width"
                    );
                    window.press("enter", cx);
                    (label, fill)
                })
                .unwrap();
            cx.run_until_parked();
            cx.update_window(f.window, |_, window, cx| {
                window.render_frame(cx);
                assert_eq!(
                    window.find(("palette-label", 0usize)).bounds(),
                    before.0,
                    "toggling membership must not move or resize the label"
                );
                assert_eq!(window.find(("palette-row", 0usize)).bounds(), before.1);
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn command_rows_stay_put_when_hover_and_keyboard_selection_change(cx: &mut TestAppContext) {
        let f = open(cx, true, true);
        let labels = [("palette-label", 0usize), ("palette-label", 1usize)];
        let before = cx
            .update_window(f.window, |_, window, cx| {
                window.render_frame(cx);
                labels.map(|label| window.find(label).bounds())
            })
            .unwrap();
        for label in labels {
            cx.update_window(f.window, |_, window, cx| window.hover(label, cx))
                .unwrap();
            cx.run_until_parked();
            cx.update_window(f.window, |_, window, cx| {
                window.render_frame(cx);
                assert_eq!(labels.map(|label| window.find(label).bounds()), before);
                window.press("down", cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(f.window, |_, window, cx| {
                window.render_frame(cx);
                assert_eq!(labels.map(|label| window.find(label).bounds()), before);
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn modal_rows_fill_the_palette_and_hover_does_not_reflow_it(cx: &mut TestAppContext) {
        let f = open(cx, true, true);
        cx.run_until_parked();
        let before = cx
            .update_window(f.window, |_, window, cx| {
                window.render_frame(cx);
                let palette = window.find("resource-palette").bounds();
                let row = window.find(("palette-row", 1usize)).bounds();
                assert_eq!(
                    row.origin.x, palette.origin.x,
                    "highlight must reach the left edge of the palette"
                );
                assert_eq!(
                    row.size.width, palette.size.width,
                    "highlight must span the full palette width, not an inset virtual row"
                );
                (palette, row)
            })
            .unwrap();
        for _ in 0..4 {
            for label in [("palette-label", 0usize), ("palette-label", 1usize)] {
                cx.update_window(f.window, |_, window, cx| window.hover(label, cx))
                    .unwrap();
                cx.run_until_parked();
                cx.update_window(f.window, |_, window, cx| {
                    window.render_frame(cx);
                    assert_eq!(window.find("resource-palette").bounds(), before.0);
                    assert_eq!(window.find(("palette-row", 1usize)).bounds(), before.1);
                })
                .unwrap();
            }
        }
    }

    #[gpui_kit::test]
    fn same_named_collections_have_independent_hover_and_membership(cx: &mut TestAppContext) {
        let f = open(cx, true, true);
        cx.update_window(f.window, |_, window, cx| {
            f.palette.update(cx, |palette, cx| {
                palette.collections[2].name = palette.collections[1].name.clone();
                cx.notify();
            });
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        for row in [1usize, 2] {
            cx.update_window(f.window, |_, window, cx| {
                window.hover(("palette-row", row), cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(f.window, |_, window, cx| {
                assert_eq!(f.palette.read(cx).highlighted, Some(row));
                window.click(("palette-row", row), cx);
            })
            .unwrap();
            cx.run_until_parked();
        }
        cx.update_window(f.window, |_, window, cx| window.click("palette-apply", cx))
            .unwrap();
        cx.run_until_parked();
        assert!(
            matches!(&f.requests.borrow()[..], [PaletteRequest::EditCollections(_, added, removed)] if added == &f.collections[1..] && removed.is_empty())
        );
    }
}
