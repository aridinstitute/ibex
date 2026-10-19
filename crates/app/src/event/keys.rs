/// A log field name. Private constructor keeps the vocabulary closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Key(&'static str);

impl Key {
    const fn new(s: &'static str) -> Self {
        Key(s)
    }

    pub(crate) fn as_str(self) -> &'static str {
        self.0
    }
}

pub(crate) const SURFACE: Key = Key::new("surface");
pub(crate) const REQUEST_ID: Key = Key::new("request_id");
pub(crate) const APP_VERSION: Key = Key::new("app.version");
pub(crate) const STATUS: Key = Key::new("status");
pub(crate) const DURATION_US: Key = Key::new("duration_us");
pub(crate) const ERROR_KIND: Key = Key::new("error.kind");
pub(crate) const ERROR_MESSAGE: Key = Key::new("error.message");

pub(crate) const APP_UPTIME_MS: Key = Key::new("app.uptime_ms");
pub(crate) const STORE_BACKEND: Key = Key::new("store.backend");
pub(crate) const STORE_OK: Key = Key::new("store.ok");
pub(crate) const STORE_LATENCY_US: Key = Key::new("store.latency_us");

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[Key] = &[
        SURFACE,
        REQUEST_ID,
        APP_VERSION,
        STATUS,
        DURATION_US,
        ERROR_KIND,
        ERROR_MESSAGE,
        APP_UPTIME_MS,
        STORE_BACKEND,
        STORE_OK,
        STORE_LATENCY_US,
    ];

    #[test]
    fn key_strings_are_unique_and_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for key in ALL {
            let s = key.as_str();
            assert!(
                seen.insert(s),
                "duplicate key string: {s}"
            );
            assert!(
                s.chars().all(|c| c.is_ascii_lowercase() || c == '_' || c == '.')
                    && !s.is_empty()
                    && !s.starts_with('.')
                    && !s.ends_with('.')
                    && !s.contains(".."),
                "malformed key: {s}"
            );
        }
    }
}
