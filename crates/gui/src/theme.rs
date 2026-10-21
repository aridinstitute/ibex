//! Oeuvre's blue-accented light and dark palettes.

use gpui_kit::component::{Theme, ThemeColor, ThemeMode};
use gpui_kit::{App, px, rgb};

pub fn apply(mode: ThemeMode, cx: &mut App) {
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| {
        theme.colors = palette(mode, theme.colors);
        theme.font_size = px(12.);
    });
}

fn palette(mode: ThemeMode, mut colors: ThemeColor) -> ThemeColor {
    let dark = mode.is_dark();
    let color = |light, dark_color| rgb(if dark { dark_color } else { light }).into();
    colors.background = color(0xf5f8ff, 0x111827);
    colors.foreground = color(0x17243b, 0xe5edfa);
    colors.primary = color(0x1764db, 0x5b9dff);
    colors.primary_hover = color(0x1254bd, 0x80b4ff);
    colors.primary_active = color(0x10469e, 0x3884ee);
    colors.primary_foreground = color(0xffffff, 0x0b1628);
    colors.accent = color(0xe1edff, 0x213a60);
    colors.accent_foreground = color(0x144fa8, 0xc5ddff);
    colors.muted = color(0xe8eef8, 0x202d43);
    colors.muted_foreground = color(0x536580, 0xa3b4ce);
    colors.border = color(0xcbd7e9, 0x31425e);
    colors.input = colors.border;
    colors.ring = colors.primary;
    colors.caret = colors.primary;
    colors.selection = color(0xc9dfff, 0x294f80);
    colors.link = colors.primary;
    colors.link_hover = colors.primary_hover;
    colors.link_active = colors.primary_active;
    colors.button = color(0xffffff, 0x1b2940);
    colors.button_foreground = colors.foreground;
    colors.button_hover = colors.accent;
    colors.button_active = colors.selection;
    colors.button_primary = colors.primary;
    colors.button_primary_hover = colors.primary_hover;
    colors.button_primary_active = colors.primary_active;
    colors.button_primary_foreground = colors.primary_foreground;
    colors.secondary = colors.muted;
    colors.secondary_foreground = colors.foreground;
    colors.secondary_hover = colors.accent;
    colors.secondary_active = colors.selection;
    colors.table = color(0xffffff, 0x141e30);
    colors.table_even = color(0xf0f5fd, 0x19263b);
    colors.table_head = color(0xe4ecf9, 0x22324c);
    colors.table_head_foreground = colors.foreground;
    colors.table_hover = color(0xe6f0ff, 0x233b5e);
    colors.table_active = color(0xcce1ff, 0x284c79);
    colors.table_active_border = colors.primary;
    colors.table_row_border = color(0xe3eaf5, 0x24334a);
    colors.table_foot = colors.table_head;
    colors.table_foot_foreground = colors.foreground;
    colors.list = colors.table;
    colors.list_even = colors.table_even;
    colors.list_head = colors.table_head;
    colors.list_hover = colors.table_hover;
    colors.list_active = colors.table_active;
    colors.list_active_border = colors.primary;
    colors.popover = colors.table;
    colors.popover_foreground = colors.foreground;
    colors.description_list_label = colors.table_head;
    colors.description_list_label_foreground = colors.muted_foreground;
    colors.title_bar = colors.background;
    colors.title_bar_border = colors.border;
    colors.scrollbar_thumb = color(0xa6bad8, 0x496181);
    colors.scrollbar_thumb_hover = colors.primary;
    colors.switch = colors.muted;
    colors.skeleton = colors.muted;
    colors
}

#[cfg(test)]
mod tests {
    use super::palette;
    use gpui_kit::component::{ThemeColor, ThemeMode};

    #[test]
    fn both_palettes_use_blue_accents_and_distinct_readable_surfaces() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let colors = palette(mode, ThemeColor::default());
            assert!(colors.primary.h > 0.55 && colors.primary.h < 0.65);
            assert!(colors.primary.s > 0.7);
            assert_ne!(colors.table, colors.table_even);
            assert_ne!(colors.table, colors.table_head);
            assert_eq!(colors.ring, colors.primary);
            assert_eq!(colors.table_active_border, colors.primary);
            if mode.is_dark() {
                assert!(colors.background.l < 0.15);
                assert!(colors.foreground.l > 0.85);
            } else {
                assert!(colors.background.l > 0.95);
                assert!(colors.foreground.l < 0.2);
            }
        }
    }
}
