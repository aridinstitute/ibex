//! Where Ibex keeps the library database, its backups, and the shared
//! config file. The GUI and CLI both resolve paths here, so they always
//! agree on one library and one `config.toml`.
//!
//! Defaults:
//! - database: `<platform data dir>/<Ibex|ibex>/database`
//!   (macOS: `~/Library/Application Support/Ibex/database`; Windows:
//!   `%APPDATA%\Ibex\database`; Linux and other Unix: `$XDG_DATA_HOME/ibex/database`,
//!   usually `~/.local/share/ibex/database`, following the lowercase XDG convention)
//! - backups: a `backups` directory beside the default database
//! - config: `~/.ibex/config.toml` on every platform
//!
//! `--data` / `IBEX_DATA` replace the database directory and move backups to
//! a sibling `<database dir name>-backups` directory, so backups always sit
//! next to the library they protect and never mix with another library's.
//! `IBEX_CONFIG_DIR` replaces the config directory; the config location never
//! depends on `--data`.

use std::path::{Path, PathBuf};

/// Environment variable naming the database directory (same as `--data`).
pub const DATA_ENV: &str = "IBEX_DATA";
/// Environment variable naming the directory that holds `config.toml`.
pub const CONFIG_DIR_ENV: &str = "IBEX_CONFIG_DIR";
/// File name of the shared config inside the config directory.
pub const CONFIG_FILE_NAME: &str = "config.toml";

#[cfg(any(target_os = "macos", windows))]
const APP_DIR_NAME: &str = "Ibex";
#[cfg(not(any(target_os = "macos", windows)))]
const APP_DIR_NAME: &str = "ibex";

/// Resolved locations for one run of a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// SurrealKV directory of the library.
    pub database: PathBuf,
    /// Directory for library backups.
    pub backups: PathBuf,
    /// The shared `config.toml`.
    pub config_file: PathBuf,
}

impl Paths {
    /// Directory that holds [`Paths::config_file`].
    pub fn config_dir(&self) -> &Path {
        self.config_file
            .parent()
            .expect("the config file always has a directory")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("cannot locate the home directory for ~/.ibex; set {CONFIG_DIR_ENV}")]
    NoHome,
    #[error("cannot locate the platform data directory; pass --data or set {DATA_ENV}")]
    NoDataDir,
    #[error("invalid library directory {}: {reason}", path.display())]
    InvalidData { path: PathBuf, reason: String },
}

/// The base directories paths resolve against. [`AppPaths::system`] reads
/// the real ones; tests inject their own so nothing touches the real home.
#[derive(Debug, Clone)]
pub struct AppPaths {
    home: Option<PathBuf>,
    data_dir: Option<PathBuf>,
}

impl AppPaths {
    pub fn system() -> Self {
        Self::new(dirs::home_dir(), dirs::data_dir())
    }

    pub fn new(home: Option<PathBuf>, data_dir: Option<PathBuf>) -> Self {
        Self { home, data_dir }
    }

    /// Resolve with the process environment: [`AppPaths::system`] bases and
    /// `IBEX_CONFIG_DIR` as the config override. `data_override` is the
    /// client's `--data` value (clap already folds in `IBEX_DATA`).
    pub fn resolve_from_env(data_override: Option<PathBuf>) -> Result<Paths, PathError> {
        Self::system().resolve(
            data_override,
            std::env::var_os(CONFIG_DIR_ENV).map(PathBuf::from),
        )
    }

    /// Resolve every path. Empty overrides count as absent. Nothing is
    /// created here; opening the library or saving config does that.
    pub fn resolve(
        &self,
        data_override: Option<PathBuf>,
        config_dir_override: Option<PathBuf>,
    ) -> Result<Paths, PathError> {
        let config_dir = match non_empty(config_dir_override) {
            Some(dir) => dir,
            None => self.home.clone().ok_or(PathError::NoHome)?.join(".ibex"),
        };
        let config_file = config_dir.join(CONFIG_FILE_NAME);

        let (database, backups) = match non_empty(data_override) {
            Some(data) => {
                let database =
                    std::path::absolute(&data).map_err(|err| PathError::InvalidData {
                        path: data.clone(),
                        reason: err.to_string(),
                    })?;
                let (Some(parent), Some(name)) = (database.parent(), database.file_name()) else {
                    return Err(PathError::InvalidData {
                        path: data,
                        reason: "it must name a directory below a parent".into(),
                    });
                };
                let mut backups_name = name.to_os_string();
                backups_name.push("-backups");
                let backups = parent.join(backups_name);
                (database, backups)
            }
            None => {
                let root = self
                    .data_dir
                    .clone()
                    .ok_or(PathError::NoDataDir)?
                    .join(APP_DIR_NAME);
                (root.join("database"), root.join("backups"))
            }
        };

        Ok(Paths {
            database,
            backups,
            config_file,
        })
    }
}

fn non_empty(path: Option<PathBuf>) -> Option<PathBuf> {
    path.filter(|path| !path.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bases() -> AppPaths {
        AppPaths::new(
            Some(PathBuf::from("/home/ada")),
            Some(PathBuf::from("/home/ada/data")),
        )
    }

    #[test]
    fn defaults_put_the_library_in_the_platform_data_dir_and_config_in_dot_ibex() {
        let paths = bases().resolve(None, None).unwrap();
        let root = PathBuf::from("/home/ada/data").join(APP_DIR_NAME);
        assert_eq!(paths.database, root.join("database"));
        assert_eq!(paths.backups, root.join("backups"));
        assert_eq!(
            paths.config_file,
            PathBuf::from("/home/ada/.ibex/config.toml")
        );
        assert_eq!(paths.config_dir(), Path::new("/home/ada/.ibex"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_defaults_live_under_application_support() {
        let paths = AppPaths::new(
            Some(PathBuf::from("/Users/ada")),
            Some(PathBuf::from("/Users/ada/Library/Application Support")),
        )
        .resolve(None, None)
        .unwrap();
        assert_eq!(
            paths.database,
            PathBuf::from("/Users/ada/Library/Application Support/Ibex/database")
        );
        assert_eq!(
            paths.backups,
            PathBuf::from("/Users/ada/Library/Application Support/Ibex/backups")
        );
        assert_eq!(
            paths.config_file,
            PathBuf::from("/Users/ada/.ibex/config.toml")
        );
    }

    #[test]
    fn data_override_moves_the_database_and_its_backups_but_not_the_config() {
        let default = bases().resolve(None, None).unwrap();
        let paths = bases()
            .resolve(Some(PathBuf::from("/srv/libraries/thesis")), None)
            .unwrap();
        assert_eq!(paths.database, PathBuf::from("/srv/libraries/thesis"));
        assert_eq!(
            paths.backups,
            PathBuf::from("/srv/libraries/thesis-backups")
        );
        assert_eq!(paths.config_file, default.config_file);
    }

    #[test]
    fn config_override_replaces_only_the_config_directory() {
        let default = bases().resolve(None, None).unwrap();
        let paths = bases()
            .resolve(None, Some(PathBuf::from("/tmp/ibex-config")))
            .unwrap();
        assert_eq!(
            paths.config_file,
            PathBuf::from("/tmp/ibex-config/config.toml")
        );
        assert_eq!(paths.database, default.database);
        assert_eq!(paths.backups, default.backups);
    }

    #[test]
    fn relative_data_override_resolves_against_the_working_directory() {
        let paths = bases()
            .resolve(Some(PathBuf::from("library")), None)
            .unwrap();
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(paths.database, cwd.join("library"));
        assert_eq!(paths.backups, cwd.join("library-backups"));
    }

    #[test]
    fn empty_overrides_count_as_absent() {
        let paths = bases()
            .resolve(Some(PathBuf::new()), Some(PathBuf::new()))
            .unwrap();
        assert_eq!(paths, bases().resolve(None, None).unwrap());
    }

    #[test]
    fn a_root_data_override_is_rejected() {
        let err = bases().resolve(Some(PathBuf::from("/")), None).unwrap_err();
        assert!(matches!(err, PathError::InvalidData { .. }), "{err}");
    }

    #[test]
    fn missing_bases_fail_only_when_needed() {
        let none = AppPaths::new(None, None);
        assert!(matches!(none.resolve(None, None), Err(PathError::NoHome)));
        assert!(matches!(
            none.resolve(None, Some(PathBuf::from("/c"))),
            Err(PathError::NoDataDir)
        ));
        let paths = none
            .resolve(Some(PathBuf::from("/d/lib")), Some(PathBuf::from("/c")))
            .unwrap();
        assert_eq!(paths.database, PathBuf::from("/d/lib"));
    }
}
