# GUI settings

The GUI reads `config.toml` from the OS application-data directory under
`oeuvre` (on macOS: `~/Library/Application Support/oeuvre/config.toml`).
This location does not change when opening a library with `--data`.
The file is created on the first settings save.

## Theme colors

```toml
[theme.light]
accent = "#0064FF"
secondary = "#D9DDE5"

[theme.dark]
accent = "#0A84FF"
secondary = "#262C35"
```

- `accent` colors buttons, links, focus borders, selection tints, and tints the
  background gradient's top-right starting corner; the wash itself stays neutral.
- `secondary` sets the background gradient's bottom-left end corner exactly:
  **accent-tinted → secondary**.
- Colors use `#RRGGBB`, case-insensitive, without transparency.
- Omit `accent` to use system blue; omit `secondary` to use the neutral
  endpoint. Invalid hex values fall back to those defaults without
  discarding other preferences.

Click the toolbar's color dot to open a popover with **Accent** and **Secondary**
tabs. Choose a swatch or enter a hex color. Each tab keeps its draft while you
switch between them, and the window previews both colors together. **Save**
persists both colors in one operation for the active light/dark appearance only.
**Cancel**, **Escape**, or clicking outside discards unsaved previews. A save
error keeps the popover open so you can retry or cancel.

For direct hex entry, open the command palette with **⌘K** and choose **Set accent
color…** or **Set secondary color…**. These commands save the entered color for
the active appearance when you press **Enter** or click **Save**.

The sun/moon button switches light/dark mode without clearing custom colors.
Legacy `[theme]` accent/secondary values are read as light-mode preferences only;
dark mode starts with its own defaults. The next settings save writes the split
`[theme.light]` and `[theme.dark]` tables.
Manual TOML changes are read when the app next starts.
