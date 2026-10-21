//! Settings persisted as config.toml in the OS application data directory.
//! The file appears on the first save; until then every setting lives at its
//! built-in default.

use std::{collections::BTreeMap, fs, io, path::Path};

use serde::{Deserialize, Serialize};

/// Name of the settings file inside the app data directory.
const CONFIG_FILE: &str = "config.toml";

/// User-tailored GUI state that survives restarts.
///
/// Column widths are keyed by column key rather than position, so
/// reordering columns never scrambles the saved widths.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub column_widths: BTreeMap<String, f64>,
}

impl Settings {
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
        fs::write(&staged, toml::to_string(self)?)?;
        fs::rename(&staged, dir.join(CONFIG_FILE))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
