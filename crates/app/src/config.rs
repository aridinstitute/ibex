//! The one `config.toml` shared by the GUI and CLI.
//!
//! Each client owns some keys and must not drop the others, so every write
//! is a read-modify-write of the whole table under an advisory lock, staged
//! in a private temp file in the same directory, synced, and renamed over
//! the old file. A crash mid-write leaves the previous config intact, and
//! secrets never sit in a world-readable file.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub use toml::Table;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{action} {}: {source}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("parse {}: {source}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Handle on a config file path. Holds no state, so a fresh read sees what
/// any other client last wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFile {
    path: PathBuf,
}

impl ConfigFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The current table; a missing file is an empty config.
    pub fn load(&self) -> Result<Table, ConfigError> {
        match fs::read_to_string(&self.path) {
            Ok(raw) => raw.parse::<Table>().map_err(|source| ConfigError::Parse {
                path: self.path.clone(),
                source,
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Table::new()),
            Err(source) => Err(self.io("read", &self.path, source)),
        }
    }

    /// Apply `edit` to the latest table and write the result atomically,
    /// returning what was written. Keys `edit` leaves alone survive as the
    /// other client wrote them. A failed edit writes nothing.
    pub fn update<E>(&self, edit: impl FnOnce(&mut Table) -> Result<(), E>) -> Result<Table, E>
    where
        E: From<ConfigError>,
    {
        let dir = self.dir();
        create_private_dir(dir).map_err(|source| self.io("create", dir, source))?;
        let _lock = self.lock()?;
        let mut table = self.load()?;
        edit(&mut table)?;
        let contents = toml::to_string(&table).map_err(ConfigError::from)?;
        self.write_atomically(contents.as_bytes())?;
        Ok(table)
    }

    fn dir(&self) -> &Path {
        match self.path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => Path::new("."),
        }
    }

    fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| crate::paths::CONFIG_FILE_NAME.to_owned())
    }

    /// Serializes concurrent GUI and CLI writers so neither loses the
    /// other's keys between its read and its rename.
    fn lock(&self) -> Result<File, ConfigError> {
        let path = self.dir().join(format!(".{}.lock", self.file_name()));
        let file = private_options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| self.io("open", &path, source))?;
        file.lock()
            .map_err(|source| self.io("lock", &path, source))?;
        Ok(file)
    }

    fn write_atomically(&self, contents: &[u8]) -> Result<(), ConfigError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let staged = self.dir().join(format!(
            ".{}.{}.{}.tmp",
            self.file_name(),
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // Exclusive creation never reuses (or follows a symlink at) a stale
        // staging path, so the secret only ever lands in a fresh 0600 file.
        let result = (|| {
            let mut file = private_options()
                .write(true)
                .create_new(true)
                .open(&staged)
                .map_err(|source| self.io("create", &staged, source))?;
            file.write_all(contents)
                .and_then(|()| file.sync_all())
                .map_err(|source| self.io("write", &staged, source))?;
            drop(file);
            fs::rename(&staged, &self.path).map_err(|source| self.io("replace", &self.path, source))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&staged);
            return result;
        }
        sync_dir(self.dir());
        Ok(())
    }

    fn io(&self, action: &'static str, path: &Path, source: io::Error) -> ConfigError {
        ConfigError::Io {
            action,
            path: path.to_path_buf(),
            source,
        }
    }
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
}

/// Create `dir` (and missing parents) as 0700 on Unix. An existing
/// directory keeps its permissions: it may be one the user chose.
fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Make the rename durable. Best-effort: some platforms cannot open a
/// directory for syncing, and the rename has already happened.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(key: &str, value: &str) -> impl FnOnce(&mut Table) -> Result<(), ConfigError> {
        let (key, value) = (key.to_owned(), value.to_owned());
        move |table| {
            table.insert(key, toml::Value::String(value));
            Ok(())
        }
    }

    #[test]
    fn missing_file_loads_as_an_empty_table() {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigFile::new(dir.path().join("config.toml"));
        assert_eq!(config.load().unwrap(), Table::new());
        assert!(!config.path().exists());
    }

    #[test]
    fn update_preserves_keys_it_does_not_touch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "jev_model = \"typesafe/jev-1\"\n[theme.light]\naccent = \"#0064FF\"\n",
        )
        .unwrap();
        let config = ConfigFile::new(&path);
        config.update(set("left_sidebar_width", "wide")).unwrap();
        let table = config.load().unwrap();
        assert_eq!(table["jev_model"].as_str(), Some("typesafe/jev-1"));
        assert_eq!(table["theme"]["light"]["accent"].as_str(), Some("#0064FF"));
        assert_eq!(table["left_sidebar_width"].as_str(), Some("wide"));
    }

    #[test]
    fn failed_edit_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "kept = 1\n").unwrap();
        let config = ConfigFile::new(&path);
        let result: Result<Table, anyhow_like::Error> = config.update(|table| {
            table.insert("dropped".into(), toml::Value::Integer(2));
            Err(anyhow_like::Error)
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "kept = 1\n");
    }

    #[test]
    fn malformed_config_is_an_error_and_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "not = [valid").unwrap();
        let config = ConfigFile::new(&path);
        assert!(matches!(config.load(), Err(ConfigError::Parse { .. })));
        assert!(config.update(set("a", "b")).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "not = [valid");
    }

    #[test]
    fn update_leaves_no_staging_files_behind_even_when_the_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = ConfigFile::new(&path);
        config.update(set("a", "b")).unwrap();
        // A directory where the file belongs makes the rename fail.
        let blocked = ConfigFile::new(dir.path().join("blocked.toml"));
        fs::create_dir(blocked.path()).unwrap();
        assert!(blocked.update(set("secret", "s3cret")).is_err());
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[cfg(unix)]
    #[test]
    fn written_config_is_private_and_a_created_directory_is_0700() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path().join(".ibex");
        let path = config_dir.join("config.toml");
        let config = ConfigFile::new(&path);
        config
            .update(set("openrouter_api_key", "test-only"))
            .unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&config_dir), 0o700);

        // A permissive file left by something else is replaced by a private one.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        config.update(set("jev_model", "m")).unwrap();
        assert_eq!(mode(&path), 0o600);
    }

    mod anyhow_like {
        #[derive(Debug)]
        pub struct Error;
        impl From<super::ConfigError> for Error {
            fn from(_: super::ConfigError) -> Self {
                Error
            }
        }
    }
}
