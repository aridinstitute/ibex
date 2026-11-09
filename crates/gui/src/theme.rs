//! Ibex's light and dark palettes: glassy translucent surfaces over the
//! workspace wash, with the configurable accents on top.

use crate::settings::{ThemeColorTarget, ThemeSettings};
use gpui_kit::component::{Theme, ThemeColor, ThemeMode};
use gpui_kit::{App, px, rgb, rgba};

pub fn apply(mode: ThemeMode, settings: &ThemeSettings, cx: &mut App) {
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| {
        theme.colors = configured_palette(mode, theme.colors, settings);
        // Root sets the window rem size from this, so every rem-based size
        // in the kit shrinks with it.
        theme.font_size = px(11.);
        // No outer halo on focused controls: the input border itself picks up
        // the accent ring color (see palette), which reads cleaner in dense
        // filter rows and the detail pane.
        theme.focus_ring = false;
    });
}

/// Accent plus glass surfaces. Kit widgets (table, list, sidebar, inputs)
/// paint these fills; keeping them translucent is what makes the whole
/// window read as one frosted sheet over the workspace gradient.
/// Dark uses AppKit's dark `systemBlue`. Light uses a deeper blue: GPUI
/// paints in sRGB, where `#007AFF` reads paler than Display P3 `systemBlue`.
fn palette(mode: ThemeMode, mut colors: ThemeColor) -> ThemeColor {
    let dark = mode.is_dark();
    let pick = |light: u32, dark_value: u32| if dark { dark_value } else { light };
    let blue = rgb(pick(0x0064ff, 0x0a84ff)).into();
    // Focused borders use a step darker than the fill accent so the edge
    // reads against light surfaces without a glow ring.
    let focus_border = rgb(pick(0x0047bd, 0x0071db)).into();
    // Selections tint while retaining the normal text color.
    let tint = rgba(pick(0x0064ff47, 0x0a84ff4d)).into();
    let row_tint = rgba(pick(0x0064ff33, 0x0a84ff40)).into();

    // Glass sheets: translucent white/ink so the atmospheric wash shows through.
    let glass = if dark {
        rgba(0x14182066) // ~40% ink
    } else {
        rgba(0xffffff59) // ~35% white
    }
    .into();
    let glass_raised = if dark {
        rgba(0x1c223080)
    } else {
        rgba(0xffffff73)
    }
    .into();
    let glass_hover = if dark {
        rgba(0xffffff14)
    } else {
        rgba(0x0000000f)
    }
    .into();
    let hairline = if dark {
        rgba(0xffffff1f)
    } else {
        rgba(0x0000001a)
    }
    .into();

    colors.background = glass;
    colors.secondary = glass;
    colors.secondary_hover = glass_hover;
    colors.secondary_active = glass_raised;
    colors.muted = glass_raised;
    colors.accent = glass_hover;
    colors.border = hairline;
    colors.input = hairline;
    colors.sidebar = glass;
    colors.sidebar_border = hairline;
    colors.sidebar_accent = glass_hover;
    colors.list = glass;
    colors.list_even = glass;
    colors.list_head = glass_raised;
    colors.list_hover = glass_hover;
    colors.table = glass;
    colors.table_even = glass;
    colors.table_head = glass_raised;
    colors.table_hover = glass_hover;
    colors.table_row_border = hairline;
    colors.popover = glass_raised;
    colors.tab = glass;
    colors.tab_active = glass_raised;
    colors.tab_bar = glass;
    colors.title_bar = rgba(0x00000000).into();
    colors.title_bar_border = rgba(0x00000000).into();
    colors.status_bar = glass;
    colors.status_bar_border = rgba(0x00000000).into();

    colors.primary = blue;
    colors.primary_hover = rgb(pick(0x0055e0, 0x3b9cff)).into();
    colors.primary_active = focus_border;
    colors.primary_foreground = rgb(0xffffff).into();
    colors.button_primary = colors.primary;
    colors.button_primary_hover = colors.primary_hover;
    colors.button_primary_active = colors.primary_active;
    colors.button_primary_foreground = colors.primary_foreground;
    colors.ring = focus_border;
    colors.caret = blue;
    colors.link = blue;
    colors.link_hover = colors.primary_hover;
    colors.link_active = colors.primary_active;
    colors.selection = tint;
    colors.table_active = row_tint;
    colors.table_active_border = blue;
    colors.list_active = tint;
    colors.list_active_border = blue;
    colors
}

fn configured_palette(mode: ThemeMode, colors: ThemeColor, settings: &ThemeSettings) -> ThemeColor {
    let mut colors = palette(mode, colors);
    let Some(value) = settings.color(mode, ThemeColorTarget::Accent) else {
        return colors;
    };
    let accent: gpui_kit::Hsla = rgb(value).into();
    let focus = gpui_kit::Hsla {
        l: (accent.l * 0.75).clamp(0., 1.),
        ..accent
    };
    let hover = gpui_kit::Hsla {
        l: if mode.is_dark() {
            (accent.l + 0.1).min(1.)
        } else {
            accent.l * 0.88
        },
        ..accent
    };
    colors.primary = accent;
    colors.primary_hover = hover;
    colors.primary_active = focus;
    // Choose the higher-contrast text, including for very bright custom accents.
    let channel = |value: f32| {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let color = rgb(value);
    let luminance =
        0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b);
    colors.primary_foreground = rgb(if luminance > 0.179 {
        0x000000
    } else {
        0xffffff
    })
    .into();
    colors.button_primary = accent;
    colors.button_primary_hover = hover;
    colors.button_primary_active = focus;
    colors.button_primary_foreground = colors.primary_foreground;
    colors.ring = focus;
    colors.caret = accent;
    colors.link = accent;
    colors.link_hover = hover;
    colors.link_active = focus;
    colors.selection = accent.opacity(if mode.is_dark() { 0.30 } else { 0.28 });
    colors.table_active = accent.opacity(if mode.is_dark() { 0.20 } else { 0.15 });
    colors.table_active_border = accent;
    colors.list_active = colors.selection;
    colors.list_active_border = accent;
    colors
}

#[cfg(test)]
mod tests {
    use super::palette;
    use gpui_kit::component::{ThemeColor, ThemeMode};
    use gpui_kit::{Hsla, rgb};

    fn neutral_base() -> ThemeColor {
        let grey = |l| Hsla {
            h: 0.,
            s: 0.,
            l,
            a: 1.,
        };
        ThemeColor {
            background: grey(0.98),
            foreground: grey(0.1),
            border: grey(0.85),
            accent: grey(0.93),
            table: grey(0.99),
            table_even: grey(0.96),
            table_head: grey(0.95),
            table_hover: grey(0.94),
            muted: grey(0.92),
            ..ThemeColor::default()
        }
    }

    #[test]
    fn accent_is_system_blue_and_surfaces_go_glass() {
        for (mode, blue, focus) in [
            (ThemeMode::Light, 0x0064ff, 0x0047bd),
            (ThemeMode::Dark, 0x0a84ff, 0x0071db),
        ] {
            let base = neutral_base();
            let colors = palette(mode, base);
            let blue: Hsla = rgb(blue).into();
            let focus: Hsla = rgb(focus).into();
            assert_eq!(colors.primary, blue);
            // Focused borders use the darker accent step, not the fill blue.
            assert_eq!(colors.ring, focus);
            assert_eq!(colors.table_active_border, blue);
            assert!(colors.primary.s > 0.95, "accent must be fully saturated");
            assert_eq!(colors.table_active.h, blue.h);
            assert!(
                colors.table_active.a <= 0.26,
                "selected rows should tint, not fill"
            );
            assert!(colors.list_active.a < 1., "dropdown highlights tint");
            assert!(colors.selection.a < 1., "text selection tints");
            // Surfaces go glass: translucent so the workspace wash reads through.
            assert!(
                colors.background.a < 0.55,
                "background should be a glass sheet, got a={}",
                colors.background.a
            );
            assert!(colors.table.a < 0.55, "table should be glass");
            assert!(colors.border.a < 0.2, "borders should be hairlines");
            assert_eq!(colors.foreground, base.foreground);
            assert_eq!(colors.title_bar.a, 0., "title bar must stay seamless");
        }
    }

    #[test]
    fn configured_accent_updates_both_modes_keeping_glass_surfaces() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let custom = crate::settings::ThemeColors {
                accent: Some("#FFFF00".into()),
                secondary: Some("#102030".into()),
            };
            let settings = crate::settings::ThemeSettings {
                light: custom.clone(),
                dark: custom,
            };
            let colors = super::configured_palette(mode, neutral_base(), &settings);
            assert_eq!(colors.primary, Hsla::from(rgb(0xffff00)));
            assert_eq!(colors.button_primary, colors.primary);
            assert_eq!(colors.primary_foreground, Hsla::from(rgb(0x000000)));
            assert_eq!(colors.table_active.h, colors.primary.h);
            assert!(colors.table_active.a < 0.35);
            assert!(colors.background.a < 0.55, "glass must survive custom accents");
        }
    }

    #[test]
    fn palette_uses_only_the_requested_appearance() {
        let settings: crate::settings::ThemeSettings =
            toml::from_str("[light]\naccent = \"#FFFF00\"\n[dark]\naccent = \"#445566\"\n")
                .unwrap();
        let light = super::configured_palette(ThemeMode::Light, neutral_base(), &settings);
        let dark = super::configured_palette(ThemeMode::Dark, neutral_base(), &settings);
        assert_eq!(light.primary, Hsla::from(rgb(0xffff00)));
        assert_eq!(dark.primary, Hsla::from(rgb(0x445566)));
        assert_ne!(light.primary, dark.primary);
    }
}
