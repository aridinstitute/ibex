//! Searchable single-selection series choices, including creation.
use gpui_kit::component::searchable_list::{SearchableListDelegate, SearchableListItem};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, IndexPath, Sizable as _, h_flex};
use gpui_kit::*;
use ibex_core::{Series, SeriesId};
/// A stored series or the typed name to create from the search box.
#[derive(Clone, Debug, PartialEq)]
pub enum SeriesChoice {
    Existing(SeriesId),
    Create(String),
}

#[derive(Clone)]
pub struct SeriesItem {
    pub label: SharedString,
    pub choice: SeriesChoice,
}

impl SearchableListItem for SeriesItem {
    type Value = SeriesChoice;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.choice
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        match &self.choice {
            SeriesChoice::Existing(_) => div().child(self.label.clone()).into_any_element(),
            SeriesChoice::Create(_) => h_flex()
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

/// Search series titles and offer creation for a new title.
pub struct SeriesDelegate {
    options: Vec<SeriesItem>,
    matched: Vec<usize>,
    create: Option<SeriesItem>,
}

impl SeriesDelegate {
    pub fn new(series: &[Series]) -> Self {
        let options = series_items(series);
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
        self.create = (!query.is_empty() && !exact).then(|| SeriesItem {
            label: format!("Create \u{201c}{query}\u{201d}").into(),
            choice: SeriesChoice::Create(query.to_owned()),
        });
    }

    fn rows(&self) -> impl Iterator<Item = &SeriesItem> {
        self.matched
            .iter()
            .filter_map(|&ix| self.options.get(ix))
            .chain(self.create.iter())
    }
}

impl SearchableListDelegate for SeriesDelegate {
    type Item = SeriesItem;

    fn items_count(&self, _: usize) -> usize {
        self.matched.len() + usize::from(self.create.is_some())
    }

    fn item(&self, ix: IndexPath) -> Option<&SeriesItem> {
        self.rows().nth(ix.row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        SeriesItem: SearchableListItem<Value = V>,
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
fn series_items(series: &[Series]) -> Vec<SeriesItem> {
    let mut items: Vec<_> = series
        .iter()
        .map(|s| SeriesItem {
            label: s.title.as_str().to_owned().into(),
            choice: SeriesChoice::Existing(s.id),
        })
        .collect();
    items.sort_by_key(|s| s.label.to_lowercase());
    items
}
