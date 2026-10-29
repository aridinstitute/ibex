//! Tabbed theme-color drafts. Preview never mutates persisted preferences.
use crate::settings::{ThemeSettings, parse_hex_color};
use gpui_kit::base::{ColorSwatch, TestSupportExt as _};
use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, ColorName, Sizable as _, ThemeMode, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

#[derive(Clone)]
pub enum ThemeEditorEvent {
    Preview(ThemeSettings),
    Save(ThemeSettings),
    Cancel,
}

pub struct ThemeEditor {
    mode: ThemeMode,
    draft: ThemeSettings,
    tab: usize,
    inputs: [Entity<InputState>; 2],
    values: [String; 2],
    error: Option<String>,
    focus: FocusHandle,
}

impl EventEmitter<ThemeEditorEvent> for ThemeEditor {}
impl Focusable for ThemeEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn color_hex(color: Hsla) -> String {
    let color = color.to_rgb();
    format!(
        "#{:02X}{:02X}{:02X}",
        (color.r * 255.).round() as u8,
        (color.g * 255.).round() as u8,
        (color.b * 255.).round() as u8
    )
}

impl ThemeEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let inputs = [
            cx.new(|cx| InputState::new(window, cx).placeholder("#RRGGBB")),
            cx.new(|cx| InputState::new(window, cx).placeholder("#RRGGBB")),
        ];
        for (index, input) in inputs.iter().enumerate() {
            cx.subscribe(input, move |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.stage(index, &value, cx);
                }
            })
            .detach();
        }
        Self {
            mode: ThemeMode::Light,
            draft: ThemeSettings::default(),
            tab: 0,
            inputs,
            values: [String::new(), String::new()],
            error: None,
            focus: cx.focus_handle(),
        }
    }

    pub fn begin(
        &mut self,
        mode: ThemeMode,
        settings: ThemeSettings,
        colors: [Hsla; 2],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode = mode;
        self.draft = settings;
        self.values = colors.map(color_hex);
        self.tab = 0;
        self.error = None;
        for (input, color) in self.inputs.iter().zip(colors) {
            input.update(cx, |input, cx| {
                input.set_value(color_hex(color), window, cx)
            });
        }
        cx.notify();
    }

    pub fn draft(&self) -> &ThemeSettings {
        &self.draft
    }

    pub fn failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        cx.notify();
    }

    fn stage(&mut self, index: usize, value: &str, cx: &mut Context<Self>) {
        let Ok(value) = parse_hex_color(value) else {
            self.error = Some("Enter a color as #RRGGBB".into());
            cx.notify();
            return;
        };
        // Input initialization must not turn inherited defaults into overrides.
        let normalized = format!("#{value:06X}");
        if self.values[index] != normalized {
            self.values[index] = normalized;
            let colors = if self.mode.is_dark() {
                &mut self.draft.dark
            } else {
                &mut self.draft.light
            };
            let slot = if index == 0 {
                &mut colors.accent
            } else {
                &mut colors.secondary
            };
            *slot = Some(format!("#{value:06X}"));
            cx.emit(ThemeEditorEvent::Preview(self.draft.clone()));
        }
        self.error = None;
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        for (index, input) in self.inputs.clone().iter().enumerate() {
            let value = input.read(cx).value().to_string();
            if parse_hex_color(&value).is_err() {
                self.error = Some("Enter a color as #RRGGBB".into());
                cx.notify();
                return;
            }
            self.stage(index, &value, cx);
        }
        cx.emit(ThemeEditorEvent::Save(self.draft.clone()));
    }
}

impl Render for ThemeEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.inputs[self.tab].read(cx).value().to_string();
        let selected = parse_hex_color(&selected).ok();
        let invalid = self
            .inputs
            .iter()
            .position(|input| parse_hex_color(&input.read(cx).value()).is_err());
        let valid = invalid.is_none();
        let error = invalid
            .map(|index| {
                format!(
                    "Enter #RRGGBB in the {} tab",
                    if index == 0 { "Accent" } else { "Secondary" }
                )
            })
            .or_else(|| self.error.clone());
        let families = [
            ColorName::Neutral,
            ColorName::Red,
            ColorName::Orange,
            ColorName::Yellow,
            ColorName::Green,
            ColorName::Cyan,
            ColorName::Blue,
            ColorName::Purple,
            ColorName::Pink,
        ];
        let scales = [50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950];
        v_flex()
            .id("theme-color-editor")
            .key_context("ThemeColorEditor")
            .test_support()
            .track_focus(&self.focus)
            .w(rems(26.))
            .gap_3()
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.mode.is_dark() {
                        "Dark appearance"
                    } else {
                        "Light appearance"
                    }),
            )
            .child(
                div().id("theme-color-tabs").test_support().child(
                    TabBar::new("theme-tabs")
                        .segmented()
                        .selected_index(self.tab)
                        .on_click(cx.listener(|this, index: &usize, _, cx| {
                            this.tab = *index;
                            cx.notify();
                        }))
                        .child(
                            Tab::new()
                                .aria_label("Accent")
                                .child(div().id("theme-accent-tab").test_support().child("Accent"))
                                .flex_1(),
                        )
                        .child(
                            Tab::new()
                                .aria_label("Secondary")
                                .child(
                                    div()
                                        .id("theme-secondary-tab")
                                        .test_support()
                                        .child("Secondary"),
                                )
                                .flex_1(),
                        ),
                ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .children(families.into_iter().map(|family| {
                        h_flex().gap_1().children(scales.into_iter().map(|scale| {
                            let color = family.scale(scale);
                            let hex = color_hex(color);
                            let active = parse_hex_color(&hex).ok() == selected;
                            let editor = cx.entity().downgrade();
                            let index = self.tab;
                            let border = if active {
                                cx.theme().foreground
                            } else {
                                cx.theme().border
                            };
                            let ring = cx.theme().ring;
                            div()
                                .id(SharedString::from(format!(
                                    "theme-swatch-{family:?}-{scale}"
                                )))
                                .test_support()
                                .child(
                                    ColorSwatch::new(
                                        SharedString::from(format!(
                                            "theme-color-{family:?}-{scale}"
                                        )),
                                        color,
                                    )
                                    .accessibility_label(format!("{family:?} {scale}, {hex}"))
                                    .selected(active)
                                    .size_6()
                                    .bg(color)
                                    .rounded_sm()
                                    .border_2()
                                    .border_color(border)
                                    .focus_visible(move |style| style.border_color(ring))
                                    .on_click(
                                        move |_, _, window, cx| {
                                            _ = editor.update(cx, |this, cx| {
                                                this.inputs[index].update(cx, |input, cx| {
                                                    input.set_value(hex.clone(), window, cx)
                                                });
                                                this.stage(index, &hex, cx);
                                            });
                                        },
                                    ),
                                )
                        }))
                    })),
            )
            .child(
                div()
                    .id("theme-color-hex")
                    .test_support()
                    .child(Input::new(&self.inputs[self.tab]).small()),
            )
            .when_some(error, |view, error| {
                view.child(
                    div()
                        .id("theme-color-error")
                        .test_support()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("theme-color-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ThemeEditorEvent::Cancel))),
                    )
                    .child(
                        Button::new("theme-color-save")
                            .primary()
                            .label("Save")
                            .disabled(!valid)
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    ),
            )
    }
}
