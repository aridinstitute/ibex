//! Spreadsheet-style inline row editing for the library DataTable.
//!
//! Window-free draft assembly lives in [`crate::edit`]; this module owns the
//! GPUI entities and the session state the table delegate renders into cells.

use gpui_kit::component::combobox::ComboboxState;
use gpui_kit::component::input::InputState;
use gpui_kit::component::searchable_list::{
    SearchableListDelegate, SearchableListItem, SearchableVec,
};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, IndexPath, Sizable as _, h_flex};
use gpui_kit::*;

use oeuvre_core::{
    CreatorSpec, Name, NameError, Person, PersonId, Resource, ResourceId, ResourceKind,
};

use crate::edit::{
    EditDraft, KindItem, creators_from_person_ids, empty_create_draft, kind_items, prefill_draft,
};
use crate::library::LibraryApp;

/// Editable columns in left-to-right focus order (Added/Edited stay labels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineField {
    Kind,
    Title,
    Creators,
    Published,
    Publisher,
}

impl InlineField {
    pub const ALL: [Self; 5] = [
        Self::Kind,
        Self::Title,
        Self::Creators,
        Self::Published,
        Self::Publisher,
    ];

    pub fn from_col(col_ix: usize) -> Option<Self> {
        match col_ix {
            0 => Some(Self::Kind),
            1 => Some(Self::Title),
            2 => Some(Self::Creators),
            3 => Some(Self::Published),
            4 => Some(Self::Publisher),
            _ => None,
        }
    }
}

/// Inline edit/create session mirrored onto the table delegate for `render_td`.
pub enum InlineSession {
    /// Fetching the raw record before editors appear.
    Loading { id: ResourceId, row_ix: usize },
    /// Editing an existing row at `form.row_ix`.
    Ready(Box<InlineRowEdit>),
    /// Creating a new resource; occupies table row 0 until saved or cancelled.
    Create(Box<InlineRowEdit>),
}

impl InlineSession {
    pub fn row_ix(&self) -> usize {
        match self {
            Self::Loading { row_ix, .. } => *row_ix,
            Self::Ready(form) | Self::Create(form) => form.row_ix,
        }
    }

    pub fn is_create(&self) -> bool {
        matches!(self, Self::Create(_))
    }

    pub fn ready_form(&self) -> Option<&InlineRowEdit> {
        match self {
            Self::Ready(form) | Self::Create(form) => Some(form),
            Self::Loading { .. } => None,
        }
    }

    pub fn ready_form_mut(&mut self) -> Option<&mut InlineRowEdit> {
        match self {
            Self::Ready(form) | Self::Create(form) => Some(form),
            Self::Loading { .. } => None,
        }
    }
}

/// Editors for one inline row. `record` is `Some` when updating, `None` when creating.
pub struct InlineRowEdit {
    pub record: Option<Resource>,
    pub row_ix: usize,
    pub title: Entity<InputState>,
    pub publisher: Entity<InputState>,
    pub published_year: Entity<InputState>,
    pub kind: Entity<ComboboxState<SearchableVec<KindItem>>>,
    pub creators: Entity<ComboboxState<PersonDelegate>>,
    /// Creator specs at open time, so person re-picks keep role/credited-as.
    pub previous_creators: Vec<CreatorSpec>,
    /// Which editable column Tab/arrows advance from. Tracked explicitly
    /// because Input's inner focus handle often fails `is_focused` probes.
    pub active_field: InlineField,
    pub error: Option<String>,
    pub saving: bool,
    pub author_creating: bool,
}

/// A stored person or the typed name to create from the search box.
#[derive(Clone, Debug, PartialEq)]
pub enum PersonChoice {
    Existing(PersonId),
    Create(String),
}

#[derive(Clone)]
pub struct PersonItem {
    pub label: SharedString,
    pub choice: PersonChoice,
}

impl SearchableListItem for PersonItem {
    type Value = PersonChoice;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.choice
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        match &self.choice {
            PersonChoice::Existing(_) => div().child(self.label.clone()).into_any_element(),
            PersonChoice::Create(_) => h_flex()
                .gap_1()
                .items_center()
                .child(
                    Icon::new(IconName::Plus)
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(self.label.clone())
                .into_any_element(),
        }
    }
}

/// Search the library's people and offer creation only when no display name matches.
pub struct PersonDelegate {
    options: Vec<PersonItem>,
    matched: Vec<usize>,
    create: Option<PersonItem>,
}

impl PersonDelegate {
    pub fn new(people: &[Person]) -> Self {
        let options = person_items(people);
        Self {
            matched: (0..options.len()).collect(),
            options,
            create: None,
        }
    }

    fn filter(&mut self, query: &str) {
        let query = query.trim();
        let needle = query.to_lowercase();
        self.matched = self
            .options
            .iter()
            .enumerate()
            .filter(|(_, item)| item.label.to_lowercase().contains(&needle))
            .map(|(ix, _)| ix)
            .collect();
        let exact = self
            .options
            .iter()
            .any(|item| item.label.to_lowercase() == needle);
        self.create = (!query.is_empty() && !exact).then(|| PersonItem {
            label: format!("Create \u{201c}{query}\u{201d}").into(),
            choice: PersonChoice::Create(query.to_owned()),
        });
    }

    fn rows(&self) -> impl Iterator<Item = &PersonItem> {
        self.matched
            .iter()
            .filter_map(|&ix| self.options.get(ix))
            .chain(self.create.iter())
    }
}

impl SearchableListDelegate for PersonDelegate {
    type Item = PersonItem;

    fn items_count(&self, _: usize) -> usize {
        self.matched.len() + usize::from(self.create.is_some())
    }

    fn item(&self, ix: IndexPath) -> Option<&PersonItem> {
        self.rows().nth(ix.row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        PersonItem: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.rows()
            .position(|item| item.value() == value)
            .map(IndexPath::new)
    }

    fn perform_search(&mut self, query: &str, _: &mut Window, _: &mut App) -> Task<()> {
        self.filter(query);
        Task::ready(())
    }
}

/// The last word is the bibliographic surname; a mononym belongs in `last`.
pub fn person_name_from_query(query: &str) -> Result<Name, NameError> {
    let words: Vec<_> = query.split_whitespace().collect();
    let last = words.last().copied();
    let first = (words.len() > 1).then(|| words[..words.len().saturating_sub(1)].join(" "));
    Name::try_new(first.as_deref(), last, None::<&str>)
}

pub fn selected_person_ids(choices: &[PersonChoice]) -> Vec<PersonId> {
    choices
        .iter()
        .filter_map(|choice| match choice {
            PersonChoice::Existing(id) => Some(*id),
            PersonChoice::Create(_) => None,
        })
        .collect()
}

impl InlineRowEdit {
    /// Prefill editors from a stored record for update.
    pub fn for_edit(
        record: Resource,
        people: &[Person],
        row_ix: usize,
        window: &mut Window,
        cx: &mut Context<LibraryApp>,
    ) -> Self {
        let draft = prefill_draft(&record);
        Self::from_draft(Some(record), draft, people, row_ix, window, cx)
    }

    /// Empty Book draft for create at row 0.
    pub fn for_create(
        people: &[Person],
        window: &mut Window,
        cx: &mut Context<LibraryApp>,
    ) -> Self {
        Self::from_draft(None, empty_create_draft(), people, 0, window, cx)
    }

    fn from_draft(
        record: Option<Resource>,
        draft: EditDraft,
        people: &[Person],
        row_ix: usize,
        window: &mut Window,
        cx: &mut Context<LibraryApp>,
    ) -> Self {
        let previous_creators = draft.creators.clone();
        let items = kind_items();
        let kind_row = items
            .iter()
            .position(|item| item.value() == &draft.kind)
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

        let person_items = person_items(people);
        let selected_indices: Vec<IndexPath> = previous_creators
            .iter()
            .filter_map(|spec| {
                person_items
                    .iter()
                    .position(|item| item.choice == PersonChoice::Existing(spec.person_id))
                    .map(IndexPath::new)
            })
            .collect();
        let creators = cx.new(|cx| {
            ComboboxState::new(PersonDelegate::new(people), selected_indices, window, cx)
                .multiple(true)
                .searchable(true)
        });

        Self {
            title: prefill_input(draft.title, "Title", window, cx),
            publisher: prefill_input(draft.publisher, "Publisher", window, cx),
            published_year: prefill_input(draft.published_year, "Year", window, cx),
            kind,
            creators,
            previous_creators,
            record,
            row_ix,
            active_field: InlineField::Title,
            error: None,
            saving: false,
            author_creating: false,
        }
    }

    /// Selected kind for the icon cell (falls back to Book on create).
    pub fn selected_kind(&self, cx: &App) -> ResourceKind {
        self.kind
            .read(cx)
            .selected_values()
            .first()
            .copied()
            .unwrap_or_else(|| {
                self.record
                    .as_ref()
                    .map(|r| r.kind)
                    .unwrap_or(ResourceKind::Book)
            })
    }

    /// Read widgets into an [`EditDraft`] for save. Parent/series/accessed
    /// ride through from the stored record on update; create leaves them empty.
    pub fn draft(&self, cx: &App) -> EditDraft {
        let value = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let kind = self
            .kind
            .read(cx)
            .selected_values()
            .first()
            .copied()
            .unwrap_or_else(|| {
                self.record
                    .as_ref()
                    .map(|r| r.kind)
                    .unwrap_or(ResourceKind::Book)
            });
        let person_ids = selected_person_ids(&self.creators.read(cx).selected_values());
        let creators = creators_from_person_ids(&person_ids, &self.previous_creators);
        let ridden = self.record.as_ref().map(prefill_draft);
        EditDraft {
            title: value(&self.title),
            kind,
            parent: self.record.as_ref().and_then(|r| r.parent_id),
            publisher: value(&self.publisher),
            published_year: value(&self.published_year),
            accessed_year: ridden
                .as_ref()
                .map(|d| d.accessed_year.clone())
                .unwrap_or_default(),
            series: ridden
                .as_ref()
                .map(|d| d.series.clone())
                .unwrap_or_default(),
            series_number: ridden
                .as_ref()
                .map(|d| d.series_number.clone())
                .unwrap_or_default(),
            creators,
            // Inline row edit does not touch tags/collections; keep the stored set.
            tag_ids: ridden
                .as_ref()
                .map(|d| d.tag_ids.clone())
                .unwrap_or_default(),
            collection_ids: ridden
                .as_ref()
                .map(|d| d.collection_ids.clone())
                .unwrap_or_default(),
        }
    }
}

pub(crate) fn person_items(people: &[Person]) -> Vec<PersonItem> {
    let mut items: Vec<PersonItem> = people
        .iter()
        .map(|person| PersonItem {
            label: person.name.display().into(),
            choice: PersonChoice::Existing(person.id),
        })
        .collect();
    items.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    items
}

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

/// Trigger body: selected people as compact secondary tags (badge chips).
pub fn creators_badge_trigger(
    selection: &[(IndexPath, PersonItem)],
    placeholder: Option<&SharedString>,
    cx: &App,
) -> AnyElement {
    if selection.is_empty() {
        return div()
            .text_color(cx.theme().muted_foreground)
            .child(
                placeholder
                    .cloned()
                    .unwrap_or_else(|| SharedString::from("Creators")),
            )
            .into_any_element();
    }
    h_flex()
        .gap_1()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .children(
            selection
                .iter()
                .filter(|(_, item)| matches!(item.choice, PersonChoice::Existing(_)))
                .map(|(_, item)| {
                    Tag::secondary()
                        .small()
                        .child(item.label.clone())
                        .into_any_element()
                }),
        )
        .into_any_element()
}
#[cfg(test)]
mod tests {
    use super::{PersonChoice, PersonDelegate, person_name_from_query, selected_person_ids};
    use gpui_kit::component::IndexPath;
    use gpui_kit::component::searchable_list::{
        SearchableListDelegate as _, SearchableListItem as _,
    };
    use oeuvre_core::{Name, Person, PersonId};

    #[test]
    fn author_search_offers_create_only_for_a_new_nonblank_name() {
        let ada = Person::try_new(
            PersonId::generate(),
            Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap(),
            vec![],
        )
        .unwrap();
        let mut delegate = PersonDelegate::new(&[ada]);
        delegate.filter(" Grace Hopper ");
        assert_eq!(delegate.items_count(0), 1);
        assert_eq!(
            delegate.item(IndexPath::new(0)).unwrap().value(),
            &PersonChoice::Create("Grace Hopper".into())
        );
        delegate.filter("ADA LOVELACE");
        assert_eq!(delegate.items_count(0), 1);
        assert_eq!(
            delegate.item(IndexPath::new(0)).unwrap().title().as_ref(),
            "Ada Lovelace"
        );
        delegate.filter("   ");
        assert_eq!(delegate.items_count(0), 1);
    }

    #[test]
    fn author_creation_only_enters_draft_after_it_has_a_stored_id() {
        let ada = PersonId::generate();
        assert_eq!(
            selected_person_ids(&[
                PersonChoice::Existing(ada),
                PersonChoice::Create("Grace Hopper".into())
            ]),
            vec![ada]
        );
    }

    #[test]
    fn author_name_splits_last_word_and_keeps_mononyms_as_last() {
        let name = person_name_from_query("  Ada  Byron Lovelace  ").unwrap();
        assert_eq!(name.first(), Some("Ada Byron"));
        assert_eq!(name.last(), Some("Lovelace"));
        let name = person_name_from_query("Augustine").unwrap();
        assert_eq!(name.first(), None);
        assert_eq!(name.last(), Some("Augustine"));
        assert!(person_name_from_query("  ").is_err());
    }
}
