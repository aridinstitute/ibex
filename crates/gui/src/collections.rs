//! Data source for the edit form's collection Combobox: library collections
//! not yet staged on the form.

use gpui_kit::component::IndexPath;
use gpui_kit::component::searchable_list::{SearchableListDelegate, SearchableListItem};
use gpui_kit::*;

use oeuvre_core::{Collection, CollectionId, CollectionTreeNode};

/// Flatten the forest into rows for the picker (name only, no nesting UI).
pub fn flatten_collections(forest: &[CollectionTreeNode]) -> Vec<Collection> {
    let mut out = Vec::new();
    fn walk(nodes: &[CollectionTreeNode], out: &mut Vec<Collection>) {
        for node in nodes {
            out.push(node.collection.clone());
            walk(&node.children, out);
        }
    }
    walk(forest, &mut out);
    // Deduplicate multi-parent appearances while keeping first-seen order.
    let mut seen = std::collections::HashSet::new();
    out.retain(|c| seen.insert(c.id));
    out.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionChoice(pub CollectionId);

#[derive(Clone)]
pub struct CollectionOption {
    label: SharedString,
    value: CollectionChoice,
}

impl SearchableListItem for CollectionOption {
    type Value = CollectionChoice;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &CollectionChoice {
        &self.value
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div().child(self.label.clone())
    }
}

pub struct CollectionDelegate {
    options: Vec<CollectionOption>,
    matched: Vec<usize>,
}

impl CollectionDelegate {
    pub fn new(available: &[Collection]) -> Self {
        let options: Vec<CollectionOption> = available
            .iter()
            .map(|collection| CollectionOption {
                label: collection.name.as_str().to_owned().into(),
                value: CollectionChoice(collection.id),
            })
            .collect();
        Self {
            matched: (0..options.len()).collect(),
            options,
        }
    }

    fn filter(&mut self, query: &str) {
        let needle = query.trim().to_lowercase();
        self.matched = self
            .options
            .iter()
            .enumerate()
            .filter(|(_, option)| option.label.to_lowercase().contains(&needle))
            .map(|(ix, _)| ix)
            .collect();
    }

    fn rows(&self) -> impl Iterator<Item = &CollectionOption> {
        self.matched.iter().filter_map(|&ix| self.options.get(ix))
    }
}

fn row_of<'a, I, V>(rows: impl Iterator<Item = &'a I>, value: &V) -> Option<IndexPath>
where
    I: SearchableListItem<Value = V> + 'a,
    V: PartialEq,
{
    rows.into_iter()
        .position(|item| item.value() == value)
        .map(|row| IndexPath::default().row(row))
}

impl SearchableListDelegate for CollectionDelegate {
    type Item = CollectionOption;

    fn items_count(&self, _: usize) -> usize {
        self.matched.len()
    }

    fn item(&self, ix: IndexPath) -> Option<&CollectionOption> {
        self.rows().nth(ix.row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        row_of(self.rows(), value)
    }

    fn perform_search(&mut self, query: &str, _: &mut Window, _: &mut App) -> Task<()> {
        self.filter(query);
        Task::ready(())
    }
}
