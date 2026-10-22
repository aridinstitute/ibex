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
        theme.font_size = px(11.);
    });
}

/// Recolors only accent-bearing fields; backgrounds, text, borders, and
/// hover fills keep the kit's neutral values. Dark uses AppKit's dark
/// `systemBlue`. Light uses a deeper blue: GPUI paints in sRGB, where
/// `#007AFF` reads paler than the Display P3 `systemBlue` macOS draws.
fn palette(mode: ThemeMode, mut colors: ThemeColor) -> ThemeColor {
    let dark = mode.is_dark();
    let pick = |light: u32, dark_value: u32| if dark { dark_value } else { light };
    let blue = rgb(pick(0x0064ff, 0x0a84ff)).into();
    // Text selections and dropdown highlights tint the surface, because
    // their text keeps the theme foreground. Table selection fills solid,
    // and the grid switches the highlighted row or column cells to
    // `primary_foreground`.
    let tint = rgba(pick(0x0064ff47, 0x0a84ff4d)).into();

    colors.primary = blue;
    colors.primary_hover = rgb(pick(0x0055e0, 0x3b9cff)).into();
    colors.primary_active = rgb(pick(0x0047bd, 0x0071db)).into();
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
    colors.table_active = blue;
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
        for (mode, blue) in [(ThemeMode::Light, 0x0064ff), (ThemeMode::Dark, 0x0a84ff)] {
            let base = neutral_base();
            let colors = palette(mode, base);
            let blue: Hsla = rgb(blue).into();
            assert_eq!(colors.primary, blue);
            assert_eq!(colors.ring, blue);
            assert_eq!(colors.table_active_border, blue);
            assert!(colors.primary.s > 0.95, "accent must be fully saturated");
            assert_eq!(colors.table_active, blue, "selected rows fill solid");
            assert!(colors.list_active.a < 1., "dropdown highlights tint");
            assert!(colors.selection.a < 1., "text selection tints");
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
