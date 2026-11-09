//! Offline user documentation, embedded from `docs/user-guide.md`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, TitlebarOptions,
    Window, WindowBounds, WindowOptions, px, size,
};

const USER_GUIDE: &str = include_str!("../../../docs/user-guide.md");
static OPEN_SEQUENCE: AtomicU64 = AtomicU64::new(1);

gpui_kit::actions!([OpenDocumentation]);

pub struct DocumentationWindow {
    handle: AnyWindowHandle,
}

impl Global for DocumentationWindow {}

impl DocumentationWindow {
    pub fn open(cx: &mut App) -> anyhow::Result<()> {
        let started = Instant::now();
        let result = Self::open_or_focus(cx);
        eprintln!(
            "{}",
            serde_json::json!({
                "level": "info",
                "command": "documentation.open",
                "surface": "gui",
                "request_id": OPEN_SEQUENCE.fetch_add(1, Ordering::Relaxed),
                "duration_us": started.elapsed().as_micros() as u64,
                "status": if result.is_ok() { "ok" } else { "error" },
                "outcome": result.as_ref().ok(),
                "error": result.as_ref().err().map(ToString::to_string),
            })
        );
        result.map(|_| ())
    }

    fn open_or_focus(cx: &mut App) -> anyhow::Result<&'static str> {
        if let Some(handle) = cx.try_global::<Self>().map(|state| state.handle)
            && cx
                .update_window(handle, |_, window, _| window.activate_window())
                .is_ok()
        {
            return Ok("focused");
        }
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(760.), px(640.)),
                cx,
            ))),
            window_min_size: Some(size(px(420.), px(320.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Ibex documentation".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                let text = cx.new(|cx| TextViewState::markdown(USER_GUIDE, cx));
                let focus = text.read(cx).focus_handle().clone();
                window.focus(&focus, cx);
                DocumentationView { text }
            })
        })?;
        cx.set_global(Self { handle });
        Ok("opened")
    }
}

struct DocumentationView {
    text: Entity<TextViewState>,
}

impl Render for DocumentationView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = cx.theme().is_dark();
        let wash = gpui_kit::linear_gradient(
            225.,
            gpui_kit::linear_color_stop(
                gpui_kit::Hsla::from(gpui_kit::rgb(if dark { 0x12141a } else { 0xe8eef4 })),
                0.,
            ),
            gpui_kit::linear_color_stop(
                gpui_kit::Hsla::from(gpui_kit::rgb(if dark { 0x1a222c } else { 0xd5dde8 })),
                1.,
            ),
        );
        let surface = cx.theme().background.opacity(if dark { 0.82 } else { 0.88 });
        v_flex()
            .id("documentation-content")
            .test_support()
            .relative()
            .size_full()
            .bg(wash)
            .text_color(cx.theme().foreground)
            .child(
                v_flex()
                    .relative()
                    .size_full()
                    .bg(surface)
                    .p_6()
                    .child(TextView::new(&self.text).scrollable(true).size_full()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{DocumentationView, DocumentationWindow};
    use gpui_kit::component::ThemeMode;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, ScrollDelta, TestAppContext, point, px};

    #[gpui_kit::test]
    fn documentation_opens_reuses_and_reopens(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            DocumentationWindow::open(cx).unwrap();
            assert_eq!(cx.windows().len(), 1);
            let first = cx.windows()[0];
            DocumentationWindow::open(cx).unwrap();
            assert_eq!(cx.windows(), vec![first]);
            cx.update_window(first, |_, window, _| window.remove_window())
                .unwrap();
            DocumentationWindow::open(cx).unwrap();
            assert_eq!(cx.windows().len(), 1);
            assert_ne!(cx.windows()[0], first);
        });
    }

    #[gpui_kit::test]
    fn documentation_renders_markdown_and_scrolls_in_both_themes(cx: &mut TestAppContext) {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let handle = cx.update(|cx| {
                gpui_kit::init(cx);
                crate::theme::apply(mode, &crate::settings::ThemeSettings::default(), cx);
                DocumentationWindow::open(cx).unwrap();
                cx.windows()[0]
            });
            let text = cx
                .update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    let root = window.root::<gpui_kit::base::Root>().unwrap().unwrap();
                    let view = root
                        .read(cx)
                        .view()
                        .clone()
                        .downcast::<DocumentationView>()
                        .unwrap();
                    let text = view.read(cx).text.clone();
                    let rendered = text.read(cx).rendered_text();
                    assert!(rendered.as_str().contains("Ibex user guide"));
                    assert!(rendered.as_str().contains("Keyboard shortcuts"));
                    assert!(!rendered.as_str().contains("# Ibex user guide"));
                    assert!(text.read(cx).focus_handle().is_focused(window));
                    text
                })
                .unwrap();
            // The user-authored guide may be short, so overflow uses its own content.
            let long = (1..=80)
                .map(|n| format!("## Section {n}\n\nParagraph {n}.\n"))
                .collect::<String>();
            cx.update(|cx| text.update(cx, |state, cx| state.set_text(&long, cx)));
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let before = text.read(cx).list_state().logical_scroll_top();
                window.scroll(
                    "documentation-content",
                    ScrollDelta::Pixels(point(px(0.), px(-450.))),
                    cx,
                );
                let after = text.read(cx).list_state().logical_scroll_top();
                assert!(
                    after.item_ix > before.item_ix || after.offset_in_item > before.offset_in_item,
                    "the guide must scroll when its content exceeds the window"
                );
                window.remove_window();
            })
            .unwrap();
        }
    }
}
