//! Data source for collection Comboboxes: library collections not yet
//! staged, plus (for the import dialog) a "Create" row for a typed new name.

use gpui_kit::component::searchable_list::{SearchableListDelegate, SearchableListItem};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, IndexPath, Sizable as _, h_flex};
use gpui_kit::*;

use ibex_core::{Collection, CollectionId, CollectionTreeNode};

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

/// What a picked row stands for: a stored collection, or a new top-level one
/// to create from the typed name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollectionChoice {
    Existing(CollectionId),
    Create(String),
}

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

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        match &self.value {
            CollectionChoice::Existing(_) => div().child(self.label.clone()).into_any_element(),
            CollectionChoice::Create(_) => h_flex()
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

pub struct CollectionDelegate {
    options: Vec<CollectionOption>,
    matched: Vec<usize>,
    /// `Some` when typed names may become new collections: the lowercased
    /// names already staged, which must not be offered again.
    creatable: Option<Vec<String>>,
    create: Option<CollectionOption>,
}

impl CollectionDelegate {
    /// Existing collections only (the edit form attaches, never creates).
    pub fn new(available: &[Collection]) -> Self {
        let options: Vec<CollectionOption> = available
            .iter()
            .map(|collection| CollectionOption {
                label: collection.name.as_str().to_owned().into(),
                value: CollectionChoice::Existing(collection.id),
            })
            .collect();
        Self {
            matched: (0..options.len()).collect(),
            options,
            creatable: None,
            create: None,
        }
    }

    /// Existing collections plus a create row while the query names no
    /// available or `blocked` (already staged, lowercased) collection.
    pub fn creatable(available: &[Collection], blocked: &[String]) -> Self {
        Self {
            creatable: Some(blocked.to_vec()),
            ..Self::new(available)
        }
    }

    fn filter(&mut self, query: &str) {
        let query = query.trim();
        let needle = query.to_lowercase();
        self.matched = self
            .options
            .iter()
            .enumerate()
            .filter(|(_, option)| option.label.to_lowercase().contains(&needle))
            .map(|(ix, _)| ix)
            .collect();
        self.create = self.creatable.as_ref().and_then(|blocked| {
            let exact = self
                .options
                .iter()
                .any(|option| option.label.to_lowercase() == needle)
                || blocked.iter().any(|name| name == &needle);
            (!query.is_empty() && !exact).then(|| CollectionOption {
                label: format!("Create \u{201c}{query}\u{201d}").into(),
                value: CollectionChoice::Create(query.to_owned()),
            })
        });
    }

    fn rows(&self) -> impl Iterator<Item = &CollectionOption> {
        self.matched
            .iter()
            .filter_map(|&ix| self.options.get(ix))
            .chain(self.create.iter())
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
        self.matched.len() + usize::from(self.create.is_some())
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

#[cfg(test)]
mod tests {
    // No `use super::*`: the glob would re-import gpui's `test` proc macro.
    use super::{CollectionChoice, CollectionDelegate};
    use gpui_kit::component::IndexPath;
    use gpui_kit::component::searchable_list::{
        SearchableListDelegate as _, SearchableListItem as _,
    };
    use ibex_core::{Collection, CollectionId, NonEmptyStr};

    fn collection(name: &str) -> Collection {
        Collection::try_new(
            CollectionId::generate(),
            NonEmptyStr::try_new(name).unwrap(),
        )
    }

    fn titles(delegate: &CollectionDelegate) -> Vec<String> {
        (0..delegate.items_count(0))
            .map(|row| {
                delegate
                    .item(IndexPath::default().row(row))
                    .unwrap()
                    .title()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn plain_delegate_never_offers_create() {
        let mut delegate = CollectionDelegate::new(&[collection("Methods")]);
        delegate.filter("Research");
        assert!(titles(&delegate).is_empty());
    }

    #[test]
    fn creatable_delegate_offers_create_unless_named_or_staged() {
        let available = vec![collection("Methods")];
        let mut delegate = CollectionDelegate::creatable(&available, &["staged".into()]);
        delegate.filter(" Research ");
        assert_eq!(titles(&delegate), ["Create \u{201c}Research\u{201d}"]);
        assert_eq!(
            delegate.item(IndexPath::default().row(0)).unwrap().value(),
            &CollectionChoice::Create("Research".into())
        );
        delegate.filter("methods");
        assert_eq!(titles(&delegate), ["Methods"]);
        delegate.filter("Staged");
        assert!(titles(&delegate).is_empty());
    }
}
