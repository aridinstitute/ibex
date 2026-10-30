//! Settings persisted as config.toml in the OS application data directory.
//! The file appears on the first save; until then every setting lives at its
//! built-in default.

use std::{collections::BTreeMap, fs, io, io::Write as _, path::Path};

use gpui_kit::component::ThemeMode;
use serde::{Deserialize, Serialize};

/// Name of the settings file inside the app data directory.
const CONFIG_FILE: &str = "config.toml";

/// Each appearance owns its colors; legacy shared colors migrate to light only.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "StoredThemeSettings")]
pub struct ThemeSettings {
    pub light: ThemeColors,
    pub dark: ThemeColors,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeColors {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct StoredThemeSettings {
    light: ThemeColors,
    dark: ThemeColors,
    accent: Option<String>,
    secondary: Option<String>,
}

impl From<StoredThemeSettings> for ThemeSettings {
    fn from(stored: StoredThemeSettings) -> Self {
        Self {
            light: ThemeColors {
                accent: stored.light.accent.or(stored.accent),
                secondary: stored.light.secondary.or(stored.secondary),
            },
            dark: stored.dark,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeColorTarget {
    Accent,
    Secondary,
}

/// Accept exactly #RRGGBB; opacity is derived by the theme, not user input.
pub fn parse_hex_color(value: &str) -> Result<u32, &'static str> {
    let value = value.trim();
    let digits = value.strip_prefix('#').unwrap_or_default();
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Enter a color as #RRGGBB, for example #0064FF");
    }
    u32::from_str_radix(digits, 16).map_err(|_| "Invalid hex color")
}

impl ThemeSettings {
    pub fn color(&self, mode: ThemeMode, target: ThemeColorTarget) -> Option<u32> {
        let colors = if mode.is_dark() {
            &self.dark
        } else {
            &self.light
        };
        let value = match target {
            ThemeColorTarget::Accent => &colors.accent,
            ThemeColorTarget::Secondary => &colors.secondary,
        };
        value
            .as_deref()
            .and_then(|value| parse_hex_color(value).ok())
    }
}
/// User-tailored GUI state that survives restarts.
///
/// Column widths are keyed by column key rather than position, so
/// reordering columns never scrambles the saved widths.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub column_widths: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_sidebar_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_sidebar_width: Option<f64>,
    #[serde(default)]
    pub theme: ThemeSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openrouter_api_key: Option<ApiKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jev_model: Option<String>,
}

/// A secret that never prints through `Debug`, so settings can be logged
/// or asserted on without leaking it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

impl Settings {
    /// Store the OpenRouter key; blank input removes it.
    pub fn set_openrouter_api_key(&mut self, key: &str, dir: &Path) -> anyhow::Result<()> {
        let key = key.trim();
        if key.chars().any(char::is_whitespace) {
            anyhow::bail!("API keys cannot contain spaces");
        }
        let mut updated = self.clone();
        updated.openrouter_api_key = (!key.is_empty()).then(|| ApiKey(key.to_owned()));
        updated.save(dir)?;
        *self = updated;
        Ok(())
    }

    /// Store the Jev model id; blank input restores the default.
    pub fn set_jev_model(&mut self, model: &str, dir: &Path) -> anyhow::Result<()> {
        let model = model.trim();
        if model.chars().any(char::is_whitespace) {
            anyhow::bail!("Model ids cannot contain spaces");
        }
        let mut updated = self.clone();
        updated.jev_model =
            (!model.is_empty() && model != crate::jev::DEFAULT_JEV_MODEL).then(|| model.to_owned());
        updated.save(dir)?;
        *self = updated;
        Ok(())
    }

    pub fn jev_model(&self) -> &str {
        self.jev_model
            .as_deref()
            .unwrap_or(crate::jev::DEFAULT_JEV_MODEL)
    }

    /// Validate and persist before publishing a new color to the running UI.
    pub fn set_theme_color(
        &mut self,
        mode: ThemeMode,
        target: ThemeColorTarget,
        hex: &str,
        dir: &Path,
    ) -> anyhow::Result<()> {
        let color = parse_hex_color(hex).map_err(anyhow::Error::msg)?;
        let mut updated = self.clone();
        let colors = if mode.is_dark() {
            &mut updated.theme.dark
        } else {
            &mut updated.theme.light
        };
        let slot = match target {
            ThemeColorTarget::Accent => &mut colors.accent,
            ThemeColorTarget::Secondary => &mut colors.secondary,
        };
        *slot = Some(format!("#{color:06X}"));
        updated.save(dir)?;
        *self = updated;
        Ok(())
    }
    /// Settings from config.toml in `dir`. A missing file is a first run;
    /// an unreadable or malformed one is a warning, because preferences
    /// must never keep the GUI from opening.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(CONFIG_FILE);
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                eprintln!("warning: ignoring {}: {err}", path.display());
                return Self::default();
            }
        };
        match toml::from_str(&raw) {
            Ok(settings) => settings,
            Err(err) => {
                eprintln!("warning: ignoring {}: {err}", path.display());
                Self::default()
            }
        }
    }

    /// Write settings to config.toml in `dir`, creating the directory.
    ///
    /// The file is staged then renamed, so a crash mid-write cannot leave a
    /// truncated config behind.
    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        fs::create_dir_all(dir)?;
        let staged = dir.join(format!("{CONFIG_FILE}.tmp"));
        let contents = toml::to_string(self)?;
        // Never write a secret into a stale, potentially permissive staging
        // file (or follow a stale symlink). Exclusive creation also fails
        // safely if another writer creates the path after its removal.
        match fs::remove_file(&staged) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&staged)?;
        file.write_all(contents.as_bytes())?;
        drop(file);
        fs::rename(&staged, dir.join(CONFIG_FILE))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn secret_config_is_private_on_creation_and_after_permissive_rewrites() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(CONFIG_FILE);
        let staged = dir.path().join(format!("{CONFIG_FILE}.tmp"));
        let mut settings = Settings::default();
        for existing in [None, Some(&config), Some(&staged)] {
            if let Some(path) = existing {
                fs::write(path, "old settings").unwrap();
                fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
            }
            settings
                .set_openrouter_api_key("test-only-not-a-real-key", dir.path())
                .unwrap();
            assert_eq!(
                fs::metadata(&config).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(Settings::load(dir.path()), settings);
            assert!(!staged.exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn secret_staging_file_is_private_even_when_rename_fails() {
        use std::os::unix::fs::PermissionsExt as _;

        for stale_staged in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let staged = dir.path().join(format!("{CONFIG_FILE}.tmp"));
            fs::create_dir(dir.path().join(CONFIG_FILE)).unwrap();
            if stale_staged {
                fs::write(&staged, "old settings").unwrap();
                fs::set_permissions(&staged, fs::Permissions::from_mode(0o644)).unwrap();
            }
            let mut settings = Settings::default();
            assert!(
                settings
                    .set_openrouter_api_key("test-only-not-a-real-key", dir.path())
                    .is_err()
            );
            assert_eq!(
                fs::metadata(&staged).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(settings, Settings::default());
        }
    }

    #[test]
    fn load_returns_defaults_when_config_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
    }

    #[test]
    fn load_returns_defaults_when_config_is_malformed() {
        let dir = tempfile::tempdir().unwrap();
        // Parses as TOML but not as Settings, exercising the parse-failure
        // path rather than the missing-file path.
        fs::write(dir.path().join(CONFIG_FILE), "column_widths = 12").unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
    }

    #[test]
    fn column_widths_round_trip_through_config_toml() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            column_widths: BTreeMap::from([
                ("title".to_owned(), 310.),
                ("creators".to_owned(), 260.),
            ]),
            ..Settings::default()
        };
        settings.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), settings);
    }

    #[test]
    fn save_creates_a_missing_directory_and_round_trips_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("oeuvre/settings");
        Settings::default().save(&nested).unwrap();
        assert!(nested.join(CONFIG_FILE).is_file());
        assert_eq!(Settings::load(&nested), Settings::default());
    }

    #[test]
    fn sidebar_widths_load_from_config_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(CONFIG_FILE),
            "left_sidebar_width = 230.0\nright_sidebar_width = 410.0\n",
        )
        .unwrap();
        let settings = Settings::load(dir.path());
        let saved = toml::to_string(&settings).unwrap();
        assert!(saved.contains("left_sidebar_width = 230.0"));
        assert!(saved.contains("right_sidebar_width = 410.0"));
        settings.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), settings);
    }

    #[test]
    fn theme_colors_load_and_survive_a_settings_save() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(CONFIG_FILE),
            "[theme]\naccent = \"#ff8800\"\nsecondary = \"#102030\"\n",
        )
        .unwrap();
        let settings = Settings::load(dir.path());
        settings.save(dir.path()).unwrap();
        let saved = fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
        assert!(saved.to_lowercase().contains("accent = \"#ff8800\""));
        assert!(saved.to_lowercase().contains("secondary = \"#102030\""));
        assert_eq!(Settings::load(dir.path()), settings);
    }

    #[test]
    fn theme_color_updates_validate_and_preserve_other_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = Settings {
            left_sidebar_width: Some(245.),
            ..Settings::default()
        };
        settings
            .set_theme_color(
                ThemeMode::Light,
                ThemeColorTarget::Accent,
                " #ab12ef ",
                dir.path(),
            )
            .unwrap();
        settings
            .set_theme_color(
                ThemeMode::Light,
                ThemeColorTarget::Secondary,
                "#102030",
                dir.path(),
            )
            .unwrap();
        assert_eq!(settings.theme.light.accent.as_deref(), Some("#AB12EF"));
        assert_eq!(settings.theme.light.secondary.as_deref(), Some("#102030"));
        assert_eq!(Settings::load(dir.path()), settings);
        assert_eq!(settings.left_sidebar_width, Some(245.));
        let before = settings.clone();
        for invalid in ["red", "#fff", "#12345678", "#gg0000", "123456", "#你好"] {
            assert!(
                settings
                    .set_theme_color(
                        ThemeMode::Light,
                        ThemeColorTarget::Accent,
                        invalid,
                        dir.path()
                    )
                    .is_err()
            );
            assert_eq!(settings, before);
            assert_eq!(Settings::load(dir.path()), before);
        }
    }

    #[test]
    fn theme_color_save_failure_keeps_the_previous_settings() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("file");
        fs::write(&blocked, "not a directory").unwrap();
        let mut settings = Settings::default();
        assert!(
            settings
                .set_theme_color(
                    ThemeMode::Light,
                    ThemeColorTarget::Accent,
                    "#123456",
                    &blocked
                )
                .is_err()
        );
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn jev_settings_round_trip_and_blank_input_clears_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = Settings {
            left_sidebar_width: Some(245.),
            ..Settings::default()
        };
        assert_eq!(settings.jev_model(), crate::jev::DEFAULT_JEV_MODEL);
        settings
            .set_openrouter_api_key(" sk-or-v1-abc ", dir.path())
            .unwrap();
        settings
            .set_jev_model("typesafe/jev-2", dir.path())
            .unwrap();
        let loaded = Settings::load(dir.path());
        assert_eq!(loaded, settings);
        assert_eq!(
            loaded.openrouter_api_key.as_ref().map(ApiKey::expose),
            Some("sk-or-v1-abc")
        );
        assert_eq!(loaded.jev_model(), "typesafe/jev-2");
        assert_eq!(loaded.left_sidebar_width, Some(245.));
        assert!(!format!("{loaded:?}").contains("sk-or-v1-abc"));
        assert!(settings.set_openrouter_api_key("a b", dir.path()).is_err());
        assert!(settings.set_jev_model("a b", dir.path()).is_err());
        assert_eq!(Settings::load(dir.path()), loaded);

        settings.set_openrouter_api_key("  ", dir.path()).unwrap();
        settings.set_jev_model("", dir.path()).unwrap();
        let saved = fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
        assert!(!saved.contains("openrouter_api_key"));
        assert!(!saved.contains("jev_model"));
        assert_eq!(
            Settings::load(dir.path()).jev_model(),
            crate::jev::DEFAULT_JEV_MODEL
        );
    }

    #[test]
    fn invalid_config_color_falls_back_without_losing_other_settings() {
        let settings: Settings = toml::from_str(
            "left_sidebar_width = 245.0\n[theme]\naccent = \"wrong\"\nsecondary = \"#123456\"\n",
        )
        .unwrap();
        assert_eq!(settings.left_sidebar_width, Some(245.));
        assert_eq!(
            settings
                .theme
                .color(ThemeMode::Light, ThemeColorTarget::Accent),
            None
        );
        assert_eq!(
            settings
                .theme
                .color(ThemeMode::Light, ThemeColorTarget::Secondary),
            Some(0x123456)
        );
    }

    #[test]
    fn legacy_theme_colors_are_kept_for_light_but_not_dark() {
        let settings: Settings =
            toml::from_str("[theme]\naccent = \"#FF8800\"\nsecondary = \"#102030\"\n").unwrap();
        let saved = toml::to_string(&settings).unwrap();
        assert!(saved.contains("[theme.light]"));
        assert!(saved.contains("[theme.dark]"));
        assert_eq!(saved.matches("#FF8800").count(), 1);
        assert_eq!(toml::from_str::<Settings>(&saved).unwrap(), settings);
    }

    #[test]
    fn light_and_dark_colors_round_trip_and_update_independently() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings: Settings = toml::from_str(
            "[theme.light]\naccent = \"#FFFF00\"\nsecondary = \"#00FFFF\"\n[theme.dark]\naccent = \"#664400\"\nsecondary = \"#003344\"\n",
        ).unwrap();
        settings
            .set_theme_color(
                ThemeMode::Dark,
                ThemeColorTarget::Accent,
                "#556677",
                dir.path(),
            )
            .unwrap();
        assert_eq!(settings.theme.light.accent.as_deref(), Some("#FFFF00"));
        assert_eq!(settings.theme.dark.accent.as_deref(), Some("#556677"));
        assert_eq!(settings.theme.dark.secondary.as_deref(), Some("#003344"));
        settings
            .set_theme_color(
                ThemeMode::Light,
                ThemeColorTarget::Secondary,
                "#112233",
                dir.path(),
            )
            .unwrap();
        assert_eq!(settings.theme.light.secondary.as_deref(), Some("#112233"));
        assert_eq!(settings.theme.dark.secondary.as_deref(), Some("#003344"));
        assert_eq!(Settings::load(dir.path()), settings);
        assert_eq!(
            settings
                .theme
                .color(ThemeMode::Light, ThemeColorTarget::Accent),
            Some(0xffff00)
        );
        assert_eq!(
            settings
                .theme
                .color(ThemeMode::Dark, ThemeColorTarget::Accent),
            Some(0x556677)
        );
    }

    #[test]
    fn explicit_light_settings_take_precedence_over_legacy_shared_colors() {
        let settings: Settings = toml::from_str(
            "[theme]\naccent = \"#FF8800\"\nsecondary = \"#102030\"\n[theme.light]\naccent = \"#123456\"\n",
        ).unwrap();
        assert_eq!(settings.theme.light.accent.as_deref(), Some("#123456"));
        assert_eq!(settings.theme.light.secondary.as_deref(), Some("#102030"));
        assert_eq!(settings.theme.dark, ThemeColors::default());
    }
}
