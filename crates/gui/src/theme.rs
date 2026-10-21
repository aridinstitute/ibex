//! Oeuvre's light and dark palettes: the kit's neutral surfaces with the
//! macOS system blue as the one accent.

use gpui_kit::component::{Theme, ThemeColor, ThemeMode};
use gpui_kit::{App, px, rgb, rgba};

pub fn apply(mode: ThemeMode, cx: &mut App) {
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| {
        theme.colors = palette(mode, theme.colors);
        // Root sets the window rem size from this, so every rem-based size
        // in the kit shrinks with it.
        theme.font_size = px(12.);
    });
}

/// Recolors only accent-bearing fields; backgrounds, text, borders, and
/// hover fills keep the kit's neutral values. Light and dark use AppKit's
/// `systemBlue` for each appearance.
fn palette(mode: ThemeMode, mut colors: ThemeColor) -> ThemeColor {
    let dark = mode.is_dark();
    let pick = |light: u32, dark_value: u32| if dark { dark_value } else { light };
    let blue = rgb(pick(0x007aff, 0x0a84ff)).into();
    // Selections tint the neutral surface instead of filling it, so row
    // text keeps the theme foreground in both appearances.
    let tint = rgba(pick(0x007aff2e, 0x0a84ff4d)).into();

    colors.primary = blue;
    colors.primary_hover = rgb(pick(0x0069db, 0x3b9cff)).into();
    colors.primary_active = rgb(pick(0x0058b8, 0x0071db)).into();
    colors.primary_foreground = rgb(0xffffff).into();
    colors.button_primary = colors.primary;
    colors.button_primary_hover = colors.primary_hover;
    colors.button_primary_active = colors.primary_active;
    colors.button_primary_foreground = colors.primary_foreground;
    colors.ring = blue;
    colors.caret = blue;
    colors.link = blue;
    colors.link_hover = colors.primary_hover;
    colors.link_active = colors.primary_active;
    colors.selection = tint;
    colors.table_active = tint;
    colors.table_active_border = blue;
    colors.list_active = tint;
    colors.list_active_border = blue;
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
    fn accent_is_system_blue_and_surfaces_stay_neutral() {
        for (mode, blue) in [(ThemeMode::Light, 0x007aff), (ThemeMode::Dark, 0x0a84ff)] {
            let base = neutral_base();
            let colors = palette(mode, base);
            let blue: Hsla = rgb(blue).into();
            assert_eq!(colors.primary, blue);
            assert_eq!(colors.ring, blue);
            assert_eq!(colors.table_active_border, blue);
            assert!(colors.primary.s > 0.95, "accent must be fully saturated");
            assert!(colors.table_active.a < 1., "selection tints, not fills");
            for (field, after, before) in [
                ("background", colors.background, base.background),
                ("foreground", colors.foreground, base.foreground),
                ("border", colors.border, base.border),
                ("accent", colors.accent, base.accent),
                ("table", colors.table, base.table),
                ("table_even", colors.table_even, base.table_even),
                ("table_head", colors.table_head, base.table_head),
                ("table_hover", colors.table_hover, base.table_hover),
                ("muted", colors.muted, base.muted),
            ] {
                assert_eq!(after, before, "{field} must keep the neutral base");
            }
        }
    }
}
