//! Right-side detail panel: identity and actions above a labelled metadata list.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::description_list::{DescriptionItem, DescriptionList};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use ibex_core::{CreatorRole, ResourceDetail, format_published_label, format_recorded_date};

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
    /// Multi-selection shows its scope rather than one resource's fields.
    Multiple(usize),
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
    for (label, value) in [
        ("Subtitle", detail.subtitle.as_ref().map(|s| s.as_str())),
        (
            "Title add-on",
            detail.title_addon.as_ref().map(|s| s.as_str()),
        ),
        (
            "Short title",
            detail.short_title.as_ref().map(|s| s.as_str()),
        ),
        ("Place", detail.place.as_deref()),
        ("Volume", detail.volume.as_deref()),
        ("Issue", detail.issue.as_deref()),
        ("Pages", detail.pages.as_ref().map(|p| p.as_str())),
        ("Total pages", detail.total_pages.as_deref()),
        ("Article number", detail.article_number.as_deref()),
        ("Edition", detail.edition.as_deref()),
        ("DOI", detail.doi.as_deref()),
        ("URL", detail.url.as_deref()),
        ("Eprint", detail.eprint.as_deref()),
        ("Language", detail.language.as_deref()),
        ("Document type", detail.document_type.as_deref()),
        ("Publication method", detail.publication_method.as_deref()),
        ("Institution", detail.institution.as_deref()),
        ("Organization", detail.organization.as_deref()),
        ("School", detail.school.as_deref()),
        ("Abstract", detail.abstract_text.as_deref()),
    ] {
        if let Some(value) = value {
            rows.push((label.into(), vec![value.to_owned()]));
        }
    }
    for (label, values) in [
        (
            "ISBN",
            detail
                .isbns
                .iter()
                .map(|id| id.as_str().to_owned())
                .collect::<Vec<_>>(),
        ),
        (
            "ISSN",
            detail
                .issns
                .iter()
                .map(|id| id.as_str().to_owned())
                .collect::<Vec<_>>(),
        ),
        (
            "ISRN",
            detail
                .isrns
                .iter()
                .map(|id| id.as_str().to_owned())
                .collect::<Vec<_>>(),
        ),
        ("Keywords", detail.keywords.clone()),
    ] {
        if !values.is_empty() {
            rows.push((label.into(), values));
        }
    }
    for (label, date) in [
        ("Original date", &detail.original_date),
        ("Event date", &detail.event_date),
    ] {
        if let Some(date) = date {
            rows.push((
                label.into(),
                vec![format_published_label(date).as_str().to_owned()],
            ));
        }
    }
    // Record-keeping stamps close the bibliographic block; tags are not a
    // text row here — the caller supplies a hashtag value the panel appends.
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

/// Reject parser repairs of relative, hostless, or malformed stored text.
/// Trimming affects only the navigation target, never the displayed/stored value.
fn browser_url_target(value: &str) -> Option<&str> {
    let value = value.trim();
    let (scheme, rest) = value.split_once("://")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https")
        || rest.starts_with(['/', '?', '#'])
        || value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control() || ch == '\\')
    {
        return None;
    }
    let parsed = reqwest::Url::parse(value).ok()?;
    parsed.has_host().then_some(value)
}

fn field_value_element(label: &str, lines: &[String], cx: &App) -> AnyElement {
    if let ("URL", [value]) = (label, lines)
        && let Some(target) = browser_url_target(value)
    {
        let muted = cx.theme().muted_foreground;
        let link = cx.theme().link;
        let ring = cx.theme().ring;
        return h_flex()
            .gap_2()
            .items_start()
            .child(div().flex_1().min_w_0().child(value.clone()))
            .child(
                gpui_kit::base::Link::new("detail-open-url")
                    .debug_selector(|| "detail-open-url".into())
                    .key_context("BrowserLink")
                    .href(target.to_owned())
                    .accessibility_label("Open URL in browser")
                    .open_with(|href, _, _, cx| cx.open_url(href))
                    // Icon-only affordance: muted until hover, with the
                    // invisible 1px border doubling as the keyboard focus ring.
                    .flex_shrink_0()
                    .px_1()
                    .border_1()
                    .border_color(cx.theme().background)
                    .cursor_pointer()
                    .text_color(muted)
                    .hover(|style| style.text_color(link))
                    .active(|style| style.text_color(link.opacity(0.6)))
                    .focus_visible(|style| style.border_color(ring))
                    .child(Icon::new(IconName::ExternalLink).with_size(FIELD_TEXT_SIZE)),
            )
            .into_any_element();
    }
    value_element(lines)
}

/// The right-side sidebar for the current [`DetailView`]: a column beside
/// the grid, sharing the row with it rather than overlapping it, so the
/// grid's rows are never behind the sidebar's empty stretches. Opaque,
/// preferred 320px width (clamped by the library to the viewport) — the
/// grid gives way when the window is too narrow to hold both — and scrollable
/// when the field list outgrows the window.
/// The loaded resource's identity, actions and metadata share one scroll area.
/// Notes are a card stacked under this one, not a footer of this scroll.
/// `extra_rows` are caller-built trailing fields (tags, collections,
/// suggestions) appended after the metadata, in order. Ignored unless the
/// panel is showing a loaded resource.
/// `max_height` caps the card so a long field list scrolls. `None` fills the
/// parent; the card still ends at its content when the list is shorter.
pub fn render_panel(
    view: DetailView<'_>,
    actions: Option<AnyElement>,
    extra_rows: Vec<(SharedString, AnyElement)>,
    max_height: Option<Pixels>,
    cx: &App,
) -> Div {
    let muted = cx.theme().muted_foreground;
    let danger = cx.theme().danger;

    let content: AnyElement = match view {
        DetailView::Multiple(count) => {
            let label = format!("{count} resources selected");
            div()
                .id("detail-selection-count")
                .test_support()
                .aria_label(label.clone())
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(label)
                .into_any_element()
        }
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
                .child(
                    DescriptionList::new()
                        .bordered(false)
                        .columns(1)
                        .label_width(FIELD_LABEL_WIDTH)
                        .children(field_rows(detail).into_iter().map(|(label, lines)| {
                            let value = field_value_element(&label, &lines, cx);
                            DescriptionItem::new(label).value(value)
                        }))
                        .children(
                            extra_rows
                                .into_iter()
                                .map(|(label, value)| DescriptionItem::new(label).value(value)),
                        ),
                )
                .into_any_element()
        }
    };

    v_flex()
        .flex_shrink_0()
        .w_full()
        // Stay clear of the workspace wash — a solid/glass fill reads as a
        // boxed island against the seamless sheet.
        .bg(rgba(0x00000000))
        .text_size(FIELD_TEXT_SIZE)
        .when_some(max_height, |panel, max| panel.max_h(max))
        .when(max_height.is_none(), |panel| panel.h_full())
        .child(
            div()
                .id("detail-panel")
                .test_support()
                .p_3()
                .overflow_y_scroll()
                .when_some(max_height, |scroll, max| scroll.max_h(max))
                .when(max_height.is_none(), |scroll| scroll.flex_1().min_h_0())
                .child(content),
        )
}

#[cfg(test)]
mod tests {
    // No `use super::*`: the glob would re-import gpui's `test` proc
    // macro (via the module's `gpui_kit::*`), hijacking `#[test]`.
    use super::{byline, field_rows, role_label};
    use ibex_core::{
        BibliographicDate, CreatorRole, DetailCreators, NonEmptyStr, ResourceDetail, ResourceId,
        ResourceKind, UtcMillis,
    };

    struct DetailPanel(ResourceDetail, gpui_kit::FocusHandle, Vec<&'static str>);

    fn press_key(cx: &mut gpui_kit::VisualTestContext, key: &'static str) {
        let keystroke = gpui_kit::Keystroke::parse(key).unwrap();
        cx.simulate_event(gpui_kit::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui_kit::KeyUpEvent { keystroke });
    }

    const NO_ACTION_URL: &str = "https://no-action.invalid/";

    fn arm_url_probe(cx: &mut gpui_kit::VisualTestContext) {
        // opened_url() clones the last request rather than consuming it.
        // A distinct sentinel makes every assertion detect a fresh activation.
        cx.update(|_, cx| cx.open_url(NO_ACTION_URL));
    }

    #[test]
    fn browser_targets_require_absolute_http_urls_with_hosts() {
        for value in [
            "https://example.org/path?q=one#section",
            "http://localhost:8080/",
            "HTTPS://Example.org/Case",
            "https://127.0.0.1/",
            "http://[::1]:8080/",
            "https://例え.テスト/論文",
        ] {
            assert_eq!(super::browser_url_target(value), Some(value), "{value}");
        }
        assert_eq!(
            super::browser_url_target("  https://example.org/a  "),
            Some("https://example.org/a")
        );
        for value in [
            "",
            " ",
            "example.org",
            "/article",
            "//example.org/a",
            "https:example.org",
            "https:/example.org",
            "https:///example.org",
            "https://",
            "https://?q=one",
            "https://#fragment",
            "https://bad host/",
            "https://exam\nple.org",
            "https://exam\tple.org",
            "https://example.org\\path",
            "https://[::1",
            "https://example.org:bad/",
            "javascript:alert(1)",
            "data:text/html,test",
            "file:///tmp/article",
            "mailto:author@example.org",
            "ftp://example.org/",
        ] {
            assert_eq!(super::browser_url_target(value), None, "{value:?}");
        }
    }

    #[test]
    fn url_validation_preserves_stored_text_and_unrelated_metadata() {
        let mut detail = journal_article();
        for value in [
            "  https://example.org/a  ",
            "not a URL",
            "javascript:alert(1)",
        ] {
            detail.url = Some(value.into());
            detail.doi = Some("https://doi.org/10.1000/example".into());
            let before = field_rows(&detail);
            let _ = super::browser_url_target(detail.url.as_deref().unwrap());
            assert_eq!(field_rows(&detail), before);
            assert_eq!(detail.url.as_deref(), Some(value));
            assert_eq!(
                before.iter().find(|(label, _)| label == "URL").unwrap().1,
                [value]
            );
            assert_eq!(
                before.iter().find(|(label, _)| label == "DOI").unwrap().1,
                ["https://doi.org/10.1000/example"]
            );
        }
    }

    impl gpui_kit::Render for DetailPanel {
        fn render(
            &mut self,
            _: &mut gpui_kit::Window,
            cx: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            use gpui_kit::{InteractiveElement as _, ParentElement as _, Styled as _};
            gpui_kit::div()
                .size_full()
                .track_focus(&self.1)
                .key_context("LibraryApp")
                .on_action(
                    cx.listener(|this, _: &crate::library::ToggleEditMode, _, _| {
                        this.2.push("edit");
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &crate::library::CycleColumnSort, _, _| {
                        this.2.push("sort");
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &crate::library::OpenCommandPalette, _, _| {
                        this.2.push("palette");
                    }),
                )
                .child(super::render_panel(
                    super::DetailView::Fields(&self.0),
                    None,
                    Vec::new(),
                    None,
                    cx,
                ))
        }
    }

    #[gpui_kit::test]
    fn url_action_opens_stored_target_with_pointer_and_keyboard(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(crate::bind_library_keys);
        let mut detail = journal_article();
        detail.url = Some("https://example.org/article?q=one#section".into());
        use gpui_kit::AppContext as _;
        // The real library starts with focus in its table. Root's contextual
        // Tab binding needs a focused descendant; a bare test window has none.
        let panel = cx.new(|cx| DetailPanel(detail, cx.focus_handle().tab_stop(false), Vec::new()));
        let (_, cx) = cx.add_window_view(|window, cx| {
            panel.read(cx).1.clone().focus(window, cx);
            gpui_kit::base::Root::new(panel.clone(), window, cx)
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        // The table shortcuts remain available before focus enters the link.
        press_key(cx, "enter");
        press_key(cx, "space");
        cx.update(|_, cx| {
            panel.update(cx, |panel, _| {
                assert_eq!(panel.2, ["edit", "sort"]);
                panel.2.clear();
            });
        });
        press_key(cx, "tab");
        cx.update(|window, cx| {
            assert!(window.focused(cx).is_some());
            window.draw(cx).clear(cx);
        });
        assert_eq!(cx.opened_url(), None);
        press_key(cx, "enter");
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("https://example.org/article?q=one#section")
        );
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = cx.debug_bounds("detail-open-url").expect("URL open action");
        assert!(bounds.size.width > gpui_kit::px(0.));
        arm_url_probe(cx);
        cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("https://example.org/article?q=one#section")
        );
        // Each event must replace the sentinel with the stored target.
        for key in ["enter", "space"] {
            arm_url_probe(cx);
            press_key(cx, key);
            assert_eq!(
                cx.opened_url().as_deref(),
                Some("https://example.org/article?q=one#section")
            );
        }
        // Unrelated application shortcuts must still reach the ancestor.
        press_key(cx, "cmd-k");
        cx.update(|_, cx| assert_eq!(panel.read(cx).2, ["palette"]));
        cx.update(|_, cx| {
            assert_eq!(
                panel.read(cx).0.url.as_deref(),
                Some("https://example.org/article?q=one#section")
            );
        });
        // Selection changes must replace the old callback, trim only the target,
        // and leave both the stored URL and its metadata text untouched.
        let stored = "  https://example.org/replacement?q=two#part  ";
        cx.update(|_, cx| {
            panel.update(cx, |panel, cx| {
                panel.0.url = Some(stored.into());
                cx.notify();
            });
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = cx.debug_bounds("detail-open-url").unwrap();
        arm_url_probe(cx);
        cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
        assert_eq!(cx.opened_url().as_deref(), Some(stored.trim()));
        cx.update(|_, cx| {
            assert_eq!(panel.read(cx).0.url.as_deref(), Some(stored));
            assert_eq!(
                field_rows(&panel.read(cx).0)
                    .iter()
                    .find(|(label, _)| label == "URL")
                    .unwrap()
                    .1,
                [stored]
            );
        });

        // Replacing the selected detail must not retain the old URL's action.
        for value in [
            Some("javascript:alert(1)"),
            Some("https:///missing-host"),
            Some("https://bad host/"),
            Some("/relative"),
            Some(""),
            Some("  "),
            None,
        ] {
            cx.update(|_, cx| {
                panel.update(cx, |panel, cx| {
                    panel.0.url = value.map(str::to_owned);
                    panel.0.doi = Some("https://doi.org/10.1000/example".into());
                    cx.notify();
                });
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(cx.debug_bounds("detail-open-url").is_none(), "{value:?}");
            arm_url_probe(cx);
            cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
            for key in ["tab", "enter", "space"] {
                press_key(cx, key);
                assert_eq!(
                    cx.opened_url().as_deref(),
                    Some(NO_ACTION_URL),
                    "{value:?}: {key}"
                );
            }
            cx.update(|_, cx| {
                let detail = &panel.read(cx).0;
                assert_eq!(detail.url.as_deref(), value);
                let rows = field_rows(detail);
                assert_eq!(
                    rows.iter()
                        .find(|(label, _)| label == "URL")
                        .map(|(_, lines)| lines[0].as_str()),
                    value
                );
                assert_eq!(
                    detail.doi.as_deref(),
                    Some("https://doi.org/10.1000/example")
                );
            });
        }
    }
    #[test]
    fn imported_bibliographic_fields_are_visible_in_the_detail_panel() {
        // Reuse a stored resource projection through the public core types.
        let mut detail = journal_article();
        detail.doi = Some("10.1000/example".into());
        detail.url = Some("https://example.org".into());
        detail.pages = Some(ibex_core::Pages::parse("10–20").unwrap());
        detail.isbns = vec!["978-0-306-40615-7".into()];
        detail.keywords = vec!["history".into(), "computing".into()];
        detail.abstract_text = Some("A summary".into());
        let rows = field_rows(&detail);
        for (label, values) in [
            ("DOI", vec!["10.1000/example"]),
            ("URL", vec!["https://example.org"]),
            ("Pages", vec!["10–20"]),
            ("ISBN", vec!["978-0-306-40615-7"]),
            ("Keywords", vec!["history", "computing"]),
            ("Abstract", vec!["A summary"]),
        ] {
            assert_eq!(
                rows.iter().find(|r| r.0 == label).map(|r| r
                    .1
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()),
                Some(values),
                "{label}"
            );
        }
    }

    /// A journal article carrying every optional field the panel can show.
    fn journal_article() -> ResourceDetail {
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
            favorite: false,
            tags: vec![],
            collections: vec![],
            attachments: vec![],
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
        // Tags render as a caller-supplied hashtag value, not a text row.
        let mut with_tags = journal_article();
        with_tags.tags = vec![
            ibex_core::Tag::try_new(
                ibex_core::TagId::generate(),
                NonEmptyStr::try_new("epistemology").unwrap(),
            ),
            ibex_core::Tag::try_new(
                ibex_core::TagId::generate(),
                NonEmptyStr::try_new("methods").unwrap(),
            ),
        ];
        assert_eq!(field_rows(&with_tags).len(), rows.len());
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
