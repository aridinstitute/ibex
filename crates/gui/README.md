# GUI settings

The GUI reads `config.toml` from the OS application-data directory under
`oeuvre` (on macOS: `~/Library/Application Support/oeuvre/config.toml`).
This location does not change when opening a library with `--data`.
The file is created on the first settings save.

## Theme colors

```toml
[theme]
accent = "#0064FF"
secondary = "#D9DDE5"
```

- `accent` colors buttons, links, focus borders, selection tints, and the
  background gradient's starting edge.
- `secondary` sets the background gradient's end color: **accent → secondary**.
- Colors use `#RRGGBB`, case-insensitive, without transparency.
- Omit `accent` to use system blue; omit `secondary` to use an accent-tinted
  neutral endpoint. Invalid hex values fall back to those defaults without
  discarding other preferences.

Click the toolbar's color dot to edit the accent, or open the command palette
with **⌘K** and choose **Set accent color…** or **Set secondary color…**.
The editor starts with the current color. Press **Enter** or click **Save** to
apply and persist it; **Escape** goes back without applying. A save error leaves
the previous theme active and allows retrying.

The sun/moon button switches light/dark mode without clearing custom colors.
Manual TOML changes are read when the app next starts.
