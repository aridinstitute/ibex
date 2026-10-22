//! Right-side detail panel: identity and actions above a labelled metadata list.

use gpui_kit::component::description_list::{DescriptionItem, DescriptionList};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use oeuvre_core::{CreatorRole, ResourceDetail, format_published_label, format_recorded_date};

use crate::kind::{kind_color, kind_label, kind_symbol};

/// Panel width on the right side of the library grid.
pub const PANEL_WIDTH: Pixels = px(320.);
/// Label column width inside the panel; values take the remaining width.
pub const FIELD_LABEL_WIDTH: Pixels = px(88.);
/// Matches the grid's cell text so the two panes read as one view.
pub const FIELD_TEXT_SIZE: Pixels = px(12.);
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

/// The byline prefers authors, then the first credited creator role.
fn primary_creator_index(detail: &ResourceDetail) -> Option<usize> {
    detail
        .creators
        .iter()
        .position(|group| group.role == CreatorRole::Author)
        .or_else(|| (!detail.creators.is_empty()).then_some(0))
}

fn byline(detail: &ResourceDetail) -> Option<String> {
    primary_creator_index(detail).map(|index| detail.creators[index].names.join(", "))
}

/// Metadata below the identity header; the kind, title and primary creator
/// group are already visible above it. Other creator roles remain here.
pub fn field_rows(detail: &ResourceDetail) -> Vec<(String, Vec<String>)> {
    let mut rows = Vec::new();
    let primary = primary_creator_index(detail);
    rows.extend(
        detail
            .creators
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != primary)
            .map(|(_, group)| (role_label(group.role), group.names.clone())),
    );
    if let Some(publisher) = &detail.publisher {
        rows.push(("Publisher".to_string(), vec![publisher.clone()]));
    }
    if let Some(published) = &detail.published {
        rows.push((
            "Published".to_string(),
            vec![format_published_label(published).as_str().to_owned()],
        ));
    }
    if let Some(accessed) = &detail.accessed {
        rows.push((
            "Accessed".to_string(),
            vec![format_published_label(accessed).as_str().to_owned()],
        ));
    }
    if let Some(container) = &detail.container_title {
        rows.push(("In".to_string(), vec![container.as_str().to_owned()]));
    }
    if let Some(series) = &detail.series_title {
        rows.push(("Series".to_string(), vec![series.as_str().to_owned()]));
    }
    if let Some(number) = &detail.series_number {
        rows.push((
            "Series number".to_string(),
            vec![number.as_str().to_owned()],
        ));
    }
    // Record-keeping stamps close the panel: they describe the library
    // record, not the work, and are always present on a stored record.
    rows.push((
        "Added".to_string(),
        vec![format_recorded_date(detail.date_added)],
    ));
    rows.push((
        "Edited".to_string(),
        vec![format_recorded_date(detail.date_edited)],
    ));
    rows
}

/// One value cell: a single line renders as text, several as a vertical
/// list (one creator name per line).
fn value_element(lines: &[String]) -> AnyElement {
    match lines {
        [line] => div().child(line.clone()).into_any_element(),
        _ => v_flex().gap_1().children(lines.to_vec()).into_any_element(),
    }
}

/// The right-side sidebar for the current [`DetailView`]: a column beside
/// the grid, sharing the row with it rather than overlapping it, so the
/// grid's rows are never behind the sidebar's empty stretches. Opaque,
/// preferred 320px width (clamped by the library to the viewport) — the
/// grid gives way when the window is too narrow to hold both — and scrollable
/// when the field list outgrows the window.
/// The loaded resource's identity, actions and metadata share one scroll area.
pub fn render_panel(view: DetailView<'_>, actions: Option<AnyElement>, cx: &App) -> Div {
    let muted = cx.theme().muted_foreground;
    let danger = cx.theme().danger;

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
        DetailView::Fields(detail) => {
            let header = v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .text_size(px(10.))
                        .child(
                            kind_symbol(detail.kind)
                                .with_size(px(14.))
                                .text_color(kind_color(detail.kind, cx.theme().is_dark())),
                        )
                        .child(div().text_color(muted).child(kind_label(detail.kind))),
                )
                .child(
                    v_flex()
                        .mt(px(3.))
                        .gap(px(1.))
                        .child(
                            div()
                                .text_size(px(18.))
                                .line_height(relative(1.15))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(detail.title.as_str().to_owned()),
                        )
                        .when_some(byline(detail), |identity, names| {
                            identity.child(
                                div()
                                    .line_height(relative(1.25))
                                    .text_color(muted)
                                    .child(names),
                            )
                        }),
                )
                .when_some(actions, |header, actions| {
                    header.child(h_flex().pt_2().child(actions))
                });
            v_flex()
                .gap_3()
                .child(header)
                .child(div().border_b_1().border_color(cx.theme().border))
                .child(
                    DescriptionList::new()
                        .bordered(false)
                        .columns(1)
                        .label_width(FIELD_LABEL_WIDTH)
                        .children(field_rows(detail).into_iter().map(|(label, lines)| {
                            DescriptionItem::new(label).value(value_element(&lines))
                        })),
                )
                .into_any_element()
        }
    };

    v_flex()
        .h_full()
        .flex_shrink_0()
        .w_full()
        .bg(cx.theme().background)
        .text_size(FIELD_TEXT_SIZE)
        .child(
            div()
                .id("detail-panel")
                .flex_1()
                .min_h_0()
                .p_3()
                .overflow_y_scroll()
                .child(content),
        )
}

#[cfg(test)]
mod tests {
    // No `use super::*`: the glob would re-import gpui's `test` proc
    // macro (via the module's `gpui_kit::*`), hijacking `#[test]`.
    use super::{byline, field_rows, role_label};
    use oeuvre_core::{
        BibliographicDate, CreatorRole, DetailCreators, NonEmptyStr, ResourceDetail, ResourceId,
        ResourceKind, UtcMillis,
    };

    /// A journal article carrying every optional field the panel can show.
    fn journal_article() -> ResourceDetail {
        ResourceDetail {
            id: ResourceId::generate(),
            kind: ResourceKind::JournalArticle,
            title: NonEmptyStr::try_new("Quantum bibliometrics").unwrap(),
            creators: vec![DetailCreators {
                role: CreatorRole::Author,
                names: vec!["Ada Lovelace".into(), "G. Hopper".into()],
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
            date_added: UtcMillis::from_unix_ms(1_609_459_200_000),
            date_edited: UtcMillis::from_unix_ms(1_609_459_200_000),
        }
    }

    #[test]
    fn field_rows_keep_field_order_and_skip_absent_fields() {
        let detail = journal_article();

        let rows = field_rows(&detail);
        let labels: Vec<&str> = rows.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, ["Published", "In", "Added", "Edited"]);
        assert_eq!(rows[0].1, vec!["Summer 2021"]);
        assert_eq!(rows[1].1, vec!["Journal of Citation Studies"]);
        assert_eq!(rows[2].1, vec!["Jan 1, 2021"]);
        assert_eq!(rows[3].1, vec!["Jan 1, 2021"]);
        assert_eq!(byline(&detail), Some("Ada Lovelace, G. Hopper".to_string()));
    }

    #[test]
    fn byline_falls_back_to_first_creator_and_preserves_other_roles() {
        let mut detail = journal_article();
        detail.creators = vec![
            DetailCreators {
                role: CreatorRole::Editor,
                names: vec!["Grace Hopper".into()],
            },
            DetailCreators {
                role: CreatorRole::Translator,
                names: vec!["Alan Turing".into()],
            },
        ];
        assert_eq!(byline(&detail).as_deref(), Some("Grace Hopper"));
        assert_eq!(
            field_rows(&detail)[0],
            ("Translator".into(), vec!["Alan Turing".into()])
        );
        detail.creators.push(DetailCreators {
            role: CreatorRole::Author,
            names: vec!["Ada Lovelace".into()],
        });
        assert_eq!(byline(&detail).as_deref(), Some("Ada Lovelace"));
        assert_eq!(field_rows(&detail)[0].0, "Editor");
        assert_eq!(field_rows(&detail)[1].0, "Translator");
        detail.creators.clear();
        assert_eq!(byline(&detail), None);
        assert_eq!(field_rows(&detail)[0].0, "Published");
    }

    #[test]
    fn role_labels_are_sentence_case() {
        assert_eq!(role_label(CreatorRole::Author), "Author");
        assert_eq!(role_label(CreatorRole::Contributor), "Contributor");
        assert_eq!(role_label(CreatorRole::Translator), "Translator");
    }
}
