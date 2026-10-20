//! Right-side detail panel: one field per row, label left and value right,
//! the way Zotero's item pane aligns them.

use gpui_kit::component::description_list::{DescriptionItem, DescriptionList};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

use oeuvre_core::{format_published_label, CreatorRole, ResourceDetail};

use crate::kind::kind_label;

/// Panel width on the right side of the library grid.
pub const PANEL_WIDTH: Pixels = px(320.);
/// Label column width inside the panel; values take the remaining width.
const FIELD_LABEL_WIDTH: Pixels = px(88.);
/// Matches the grid's cell text so the two panes read as one view.
const FIELD_TEXT_SIZE: Pixels = px(13.);

/// Detail fetch state for the selected resource. The loaded detail is boxed
/// to keep the enum small; only one exists at a time.
pub enum DetailFetch {
    Loading,
    Loaded(Box<ResourceDetail>),
    Failed(String),
}

/// What the panel shows right now, derived from the selection and fetch.
pub enum DetailView<'a> {
    /// No row is selected.
    Empty,
    Loading,
    Failed(&'a str),
    Fields(&'a ResourceDetail),
}

/// Sentence-case label for a creator role, matching `kind_label`'s style.
pub fn role_label(role: CreatorRole) -> String {
    match role {
        CreatorRole::Author => "Author",
        CreatorRole::Editor => "Editor",
        CreatorRole::Translator => "Translator",
        CreatorRole::Contributor => "Contributor",
        CreatorRole::Host => "Host",
        CreatorRole::Director => "Director",
    }
    .to_owned()
}

/// (field name, field value) rows for a detail, absent fields omitted the
/// way the CLI summary row omits them. Title and Kind always show.
pub fn field_rows(detail: &ResourceDetail) -> Vec<(String, String)> {
    let mut rows = vec![
        ("Title".to_string(), detail.title.as_str().to_owned()),
        ("Kind".to_string(), kind_label(detail.kind)),
    ];
    rows.extend(
        detail
            .creators
            .iter()
            .map(|group| (role_label(group.role), group.names.clone())),
    );
    if let Some(publisher) = &detail.publisher {
        rows.push(("Publisher".to_string(), publisher.clone()));
    }
    if let Some(published) = &detail.published {
        rows.push((
            "Published".to_string(),
            format_published_label(published).as_str().to_owned(),
        ));
    }
    if let Some(accessed) = &detail.accessed {
        rows.push((
            "Accessed".to_string(),
            format_published_label(accessed).as_str().to_owned(),
        ));
    }
    if let Some(container) = &detail.container_title {
        rows.push(("In".to_string(), container.as_str().to_owned()));
    }
    if let Some(series) = &detail.series_title {
        rows.push(("Series".to_string(), series.as_str().to_owned()));
    }
    if let Some(number) = &detail.series_number {
        rows.push(("Series number".to_string(), number.as_str().to_owned()));
    }
    rows
}

/// The right-side panel for the current [`DetailView`].
pub fn render_panel(view: DetailView<'_>, cx: &App) -> Div {
    let muted = cx.theme().muted_foreground;
    let danger = cx.theme().danger;
    let border = cx.theme().border;

    let content: AnyElement = match view {
        DetailView::Empty => div()
            .text_color(muted)
            .child("Select a resource to see its details.")
            .into_any_element(),
        DetailView::Loading => div()
            .text_color(muted)
            .child("Loading details…")
            .into_any_element(),
        DetailView::Failed(error) => div()
            .text_color(danger)
            .child(error.to_string())
            .into_any_element(),
        DetailView::Fields(detail) => DescriptionList::new()
            .bordered(false)
            .columns(1)
            .label_width(FIELD_LABEL_WIDTH)
            .children(
                field_rows(detail)
                    .into_iter()
                    .map(|(label, value)| DescriptionItem::new(label).value(value)),
            )
            .into_any_element(),
    };

    div()
        .w(PANEL_WIDTH)
        .flex_shrink_0()
        .h_full()
        .p_3()
        .text_size(FIELD_TEXT_SIZE)
        .border_l_1()
        .border_color(border)
        .overflow_hidden()
        .child(content)
}

#[cfg(test)]
mod tests {
    // No `use super::*`: the glob would re-import gpui's `test` proc
    // macro (via the module's `gpui_kit::*`), hijacking `#[test]`.
    use super::{field_rows, role_label};
    use oeuvre_core::{
        BibliographicDate, CreatorRole, DetailCreators, NonEmptyStr, ResourceDetail, ResourceId,
        ResourceKind, UtcMillis,
    };

    #[test]
    fn field_rows_keep_field_order_and_skip_absent_fields() {
        let detail = ResourceDetail {
            id: ResourceId::generate(),
            kind: ResourceKind::JournalArticle,
            title: NonEmptyStr::try_new("Quantum bibliometrics").unwrap(),
            creators: vec![DetailCreators {
                role: CreatorRole::Author,
                names: "Ada Lovelace; G. Hopper".into(),
            }],
            publisher: None,
            published: Some(
                BibliographicDate::try_new(
                    UtcMillis::from_unix_ms(0),
                    Some(NonEmptyStr::try_new("Summer 2021").unwrap()),
                )
                .unwrap(),
            ),
            accessed: None,
            container_title: Some(NonEmptyStr::try_new("Journal of Citation Studies").unwrap()),
            series_title: None,
            series_number: None,
        };

        let rows = field_rows(&detail);
        let labels: Vec<&str> = rows.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, ["Title", "Kind", "Author", "Published", "In"]);
        assert_eq!(rows[0].1, "Quantum bibliometrics");
        assert_eq!(rows[1].1, "Journal article");
        assert_eq!(rows[2].1, "Ada Lovelace; G. Hopper");
        assert_eq!(rows[3].1, "Summer 2021");
        assert_eq!(rows[4].1, "Journal of Citation Studies");
    }

    #[test]
    fn role_labels_are_sentence_case() {
        assert_eq!(role_label(CreatorRole::Author), "Author");
        assert_eq!(role_label(CreatorRole::Contributor), "Contributor");
        assert_eq!(role_label(CreatorRole::Translator), "Translator");
    }
}
