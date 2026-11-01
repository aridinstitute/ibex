//! Data source for the edit form's tag Combobox: library tags not yet
//! staged on the form, plus a "Create" row for a typed new name.

use gpui_kit::component::searchable_list::{SearchableListDelegate, SearchableListItem};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, IndexPath, Sizable as _, h_flex};
use gpui_kit::*;

use ibex_core::{Tag, TagId};

/// What a picked row stands for: a stored tag, or a new one to create
/// from the typed name.
#[derive(Clone, Debug, PartialEq)]
pub enum TagChoice {
    Existing(TagId),
    Create(String),
}

#[derive(Clone)]
pub struct TagOption {
    label: SharedString,
    value: TagChoice,
}

impl SearchableListItem for TagOption {
    type Value = TagChoice;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &TagChoice {
        &self.value
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        match &self.value {
            TagChoice::Existing(_) => div().child(self.label.clone()).into_any_element(),
            TagChoice::Create(_) => h_flex()
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

/// Library tags still available to attach, filtered by the search query,
/// followed by a create row while the query names no existing tag and no
/// tag the resource already carries.
pub struct TagDelegate {
    options: Vec<TagOption>,
    /// Lowercased names of tags the resource already has: typing one of
    /// these must not offer a create row for a duplicate attach.
    blocked: Vec<String>,
    matched: Vec<usize>,
    create: Option<TagOption>,
}

impl TagDelegate {
    /// `available` is the library tags the resource does not yet carry.
    /// `blocked` is the lowercased text of tags already on the resource.
    pub fn new(available: &[Tag], blocked: &[String]) -> Self {
        let options: Vec<TagOption> = available
            .iter()
            .map(|tag| TagOption {
                label: tag.text.as_str().to_owned().into(),
                value: TagChoice::Existing(tag.id),
            })
            .collect();
        Self {
            matched: (0..options.len()).collect(),
            options,
            blocked: blocked.to_vec(),
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
            .filter(|(_, option)| option.label.to_lowercase().contains(&needle))
            .map(|(ix, _)| ix)
            .collect();
        let exact = self
            .options
            .iter()
            .any(|option| option.label.to_lowercase() == needle)
            || self.blocked.iter().any(|name| name == &needle);
        self.create = (!query.is_empty() && !exact).then(|| TagOption {
            label: format!("Create \u{201c}{query}\u{201d}").into(),
            value: TagChoice::Create(query.to_owned()),
        });
    }

    fn rows(&self) -> impl Iterator<Item = &TagOption> {
        self.matched
            .iter()
            .filter_map(|&ix| self.options.get(ix))
            .chain(self.create.iter())
    }
}

/// Row of `value` among `rows`; generic so the comparison runs on the
/// delegate's `V` rather than the concrete `TagChoice`.
fn row_of<'a, I, V>(rows: impl Iterator<Item = &'a I>, value: &V) -> Option<IndexPath>
where
    I: SearchableListItem<Value = V> + 'a,
    V: PartialEq,
{
    rows.into_iter()
        .position(|item| item.value() == value)
        .map(|row| IndexPath::default().row(row))
}

impl SearchableListDelegate for TagDelegate {
    type Item = TagOption;

    fn items_count(&self, _: usize) -> usize {
        self.matched.len() + usize::from(self.create.is_some())
    }

    fn item(&self, ix: IndexPath) -> Option<&TagOption> {
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
    use super::{TagChoice, TagDelegate};
    use gpui_kit::component::IndexPath;
    use gpui_kit::component::searchable_list::{
        SearchableListDelegate as _, SearchableListItem as _,
    };
    use ibex_core::{NonEmptyStr, Tag, TagId};

    fn tag(text: &str) -> Tag {
        Tag::try_new(TagId::generate(), NonEmptyStr::try_new(text).unwrap())
    }

    fn titles(delegate: &TagDelegate) -> Vec<String> {
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
    fn search_filters_tags_and_offers_create_for_a_new_name() {
        let tags = vec![
            tag("epistemology"),
            tag("methods"),
            tag("Philosophy of Mind"),
        ];
        let mut delegate = TagDelegate::new(&tags, &[]);
        assert_eq!(
            titles(&delegate),
            ["epistemology", "methods", "Philosophy of Mind"]
        );

        delegate.filter("  mind ");
        assert_eq!(
            titles(&delegate),
            ["Philosophy of Mind", "Create \u{201c}mind\u{201d}"]
        );

        // An exact (case-insensitive) match needs no create row.
        delegate.filter("Methods");
        assert_eq!(titles(&delegate), ["methods"]);

        delegate.filter("ontology");
        assert_eq!(titles(&delegate), ["Create \u{201c}ontology\u{201d}"]);
        let last = delegate.item(IndexPath::default().row(0)).unwrap();
        assert_eq!(last.value(), &TagChoice::Create("ontology".into()));
    }

    #[test]
    fn create_is_blocked_for_tags_already_on_the_resource() {
        let available = vec![tag("epistemology")];
        let blocked = vec!["methods".into()];
        let mut delegate = TagDelegate::new(&available, &blocked);
        delegate.filter("Methods");
        assert_eq!(titles(&delegate), Vec::<String>::new());
        delegate.filter("method");
        assert_eq!(titles(&delegate), ["Create \u{201c}method\u{201d}"]);
    }

    #[test]
    fn position_resolves_existing_tags_among_visible_rows() {
        let tags = vec![tag("a"), tag("b")];
        let mut delegate = TagDelegate::new(&tags, &[]);
        let b = TagChoice::Existing(tags[1].id);
        assert_eq!(delegate.position(&b), Some(IndexPath::default().row(1)));
        delegate.filter("a");
        assert_eq!(delegate.position(&b), None);
    }
}
