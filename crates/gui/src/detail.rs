//! Right-side detail panel: one field per row, label left and value right,
//! the way Zotero's item pane aligns them.

use gpui_kit::component::description_list::{DescriptionItem, DescriptionList};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use oeuvre_core::{format_published_label, CreatorRole, ResourceDetail};

use crate::kind::{kind_color, kind_label};

/// Panel width on the right side of the library grid.
pub const PANEL_WIDTH: Pixels = px(320.);
/// Label column width inside the panel; values take the remaining width.
pub const FIELD_LABEL_WIDTH: Pixels = px(88.);
/// Matches the grid's cell text so the two panes read as one view.
pub const FIELD_TEXT_SIZE: Pixels = px(13.);
/// Opacity of the kind tint at the panel's top edge. A hint of the kind's
/// color rather than a wash: the field labels stay above 13:1 contrast on
/// it in both palettes.
const TINT_ALPHA: f32 = 0.16;
/// Fraction of the panel's height the tint fades across. It reaches fully
/// transparent before the bottom, so a short field list still fades out
/// rather than ending on a hard band.
const TINT_FADE: f32 = 0.2;

// Compile-time guard on the two knobs above: the tint must stay short of
// opaque so the field text keeps its contrast, and finish fading at or
// before the bottom edge.
const _: () = {
    assert!(TINT_ALPHA > 0. && TINT_ALPHA <= 0.6);
    assert!(TINT_FADE > 0. && TINT_FADE <= 1.);
};

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

/// (field name, value lines) rows for a detail, absent fields omitted the
/// way the CLI summary row omits them. Title and Kind always show; a creator
/// group carries one line per name.
pub fn field_rows(detail: &ResourceDetail) -> Vec<(String, Vec<String>)> {
    let mut rows: Vec<(String, Vec<String>)> = vec![
        ("Title".to_string(), vec![detail.title.as_str().to_owned()]),
        ("Kind".to_string(), vec![kind_label(detail.kind)]),
    ];
    rows.extend(
        detail
            .creators
            .iter()
            .map(|group| (role_label(group.role), group.names.clone())),
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
    rows
}

/// One value cell: a single line renders as text, several as a vertical
/// list (one creator name per line).
fn value_element(lines: &[String]) -> AnyElement {
    match lines {
        [line] => div().child(line.clone()).into_any_element(),
        _ => v_flex()
            .gap_1()
            .children(lines.to_vec())
            .into_any_element(),
    }
}

/// Kind color the panel tints toward, or `None` while no resource is
/// loaded. Pure so the tint's source is unit-testable without a window.
fn tint_color(view: &DetailView<'_>, dark: bool) -> Option<Hsla> {
    match view {
        DetailView::Fields(detail) => Some(kind_color(detail.kind, dark)),
        DetailView::Empty | DetailView::Loading | DetailView::Failed(_) => None,
    }
}

/// The tint's two gradient stops: the kind color at [`TINT_ALPHA`] at the
/// panel's top edge, fading to fully transparent at [`TINT_FADE`] of the way
/// down. Positions are fractions of the panel's height, not percentages out
/// of 100 — `LinearColorStop::percentage` is documented as 0.0 to 1.0, and a
/// value past 1 pushes the second stop off the end of the gradient, which
/// collapses the ramp into one flat color.
fn tint_stops(color: Hsla) -> [LinearColorStop; 2] {
    [
        linear_color_stop(color.opacity(TINT_ALPHA), 0.),
        linear_color_stop(color.opacity(0.), TINT_FADE),
    ]
}

/// The kind-colored wash over the panel's top edge, fading top to bottom.
/// It ends fully transparent, so the panel's own opaque background still
/// keeps the covered grid columns from showing through.
fn kind_tint(color: Hsla) -> Div {
    let [from, to] = tint_stops(color);
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        // Angle 180 runs stop 0 to stop 1 top to bottom, so the color sits
        // at the panel's top edge.
        .bg(linear_gradient(180., from, to))
}

/// The right-side sidebar for the current [`DetailView`]: a column beside
/// the grid, sharing the row with it rather than overlapping it, so the
/// grid's rows are never behind the sidebar's empty stretches. Opaque,
/// fixed-width — the grid gives way when the window is too narrow to hold
/// both — and scrollable when the field list outgrows the window.
/// `actions`, when present, rides the sidebar's top-right corner over the
/// kind tint.
pub fn render_panel(view: DetailView<'_>, actions: Option<AnyElement>, cx: &App) -> Div {
    let muted = cx.theme().muted_foreground;
    let danger = cx.theme().danger;
    let border = cx.theme().border;
    let tint = tint_color(&view, cx.theme().is_dark());

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
            .children(field_rows(detail).into_iter().map(|(label, lines)| {
                DescriptionItem::new(label).value(value_element(&lines))
            }))
            .into_any_element(),
    };

    v_flex()
        // The tint is absolutely positioned, so the sidebar must be its
        // containing block: without `relative` the wash would escape into
        // the nearest positioned ancestor and spill over the grid.
        .relative()
        .h_full()
        .flex_shrink_0()
        .w(PANEL_WIDTH)
        .bg(cx.theme().background)
        .border_l_1()
        .border_color(border)
        .text_size(FIELD_TEXT_SIZE)
        // The tint is a sibling of the scroller, not a child, so it stays
        // pinned to the sidebar's top edge as the fields scroll under it.
        .when_some(tint, |el, tint| el.child(kind_tint(tint)))
        .when_some(actions, |el, actions| {
            el.child(
                h_flex()
                    .justify_end()
                    .px_2()
                    .pt_1()
                    .flex_shrink_0()
                    .child(actions),
            )
        })
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
    use super::{
        DetailView, TINT_ALPHA, TINT_FADE, field_rows, role_label, tint_color, tint_stops,
    };
    use crate::kind::kind_color;
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
        }
    }

    #[test]
    fn field_rows_keep_field_order_and_skip_absent_fields() {
        let detail = journal_article();

        let rows = field_rows(&detail);
        let labels: Vec<&str> = rows.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, ["Title", "Kind", "Author", "Published", "In"]);
        assert_eq!(rows[0].1, vec!["Quantum bibliometrics"]);
        assert_eq!(rows[1].1, vec!["Journal article"]);
        // Creator groups carry one line per name; the panel stacks them.
        assert_eq!(rows[2].1, vec!["Ada Lovelace", "G. Hopper"]);
        assert_eq!(rows[3].1, vec!["Summer 2021"]);
        assert_eq!(rows[4].1, vec!["Journal of Citation Studies"]);
    }

    #[test]
    fn role_labels_are_sentence_case() {
        assert_eq!(role_label(CreatorRole::Author), "Author");
        assert_eq!(role_label(CreatorRole::Contributor), "Contributor");
        assert_eq!(role_label(CreatorRole::Translator), "Translator");
    }

    #[test]
    fn tint_follows_the_loaded_kind_and_vanishes_without_one() {
        let mut detail = journal_article();
        for kind in [ResourceKind::JournalArticle, ResourceKind::Dataset] {
            detail.kind = kind;
            for dark in [false, true] {
                assert_eq!(
                    tint_color(&DetailView::Fields(&detail), dark),
                    Some(kind_color(kind, dark)),
                    "{kind:?} (dark: {dark})"
                );
            }
        }
        // No loaded resource means no kind to tint with, so the panel stays
        // plain rather than guessing.
        assert_eq!(tint_color(&DetailView::Empty, false), None);
        assert_eq!(tint_color(&DetailView::Loading, false), None);
        assert_eq!(tint_color(&DetailView::Failed("boom"), false), None);
    }

    #[test]
    fn tint_stops_fade_within_the_panel() {
        let color = kind_color(ResourceKind::Book, false);
        let [from, to] = tint_stops(color);

        assert_eq!(from.percentage, 0., "the tint starts at the top edge");
        assert_eq!(to.percentage, TINT_FADE);

        // `LinearColorStop::percentage` is a fraction of the gradient's
        // length, not a percentage out of 100. A stop past 1 runs off the
        // end and the shader clamps the whole panel to the first stop's
        // color, which is how this shipped as a solid wash once.
        for stop in [from, to] {
            assert!(
                (0. ..=1.).contains(&stop.percentage),
                "stop {} is outside the gradient",
                stop.percentage
            );
        }

        // Strong at the top, gone at the fade line, and the same hue
        // throughout rather than a blend toward another color.
        assert_eq!(from.color.a, TINT_ALPHA);
        assert_eq!(to.color.a, 0.);
        assert_eq!(
            (from.color.h, from.color.s, from.color.l),
            (color.h, color.s, color.l)
        );
    }
}
