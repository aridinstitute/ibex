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
pub(crate) const PERSON_ID: Key = Key::new("person.id");
pub(crate) const PERSON_DISPLAY: Key = Key::new("person.display");
pub(crate) const RESOURCE_ID: Key = Key::new("resource.id");
pub(crate) const RESOURCE_KIND: Key = Key::new("resource.kind");
pub(crate) const RESOURCE_TITLE: Key = Key::new("resource.title");
pub(crate) const RESOURCE_CHANGED_FIELDS: Key = Key::new("resource.changed_fields");
pub(crate) const RESOURCE_FAVORITE: Key = Key::new("resource.favorite");
pub(crate) const RESOURCE_FILTER_CREATOR: Key = Key::new("resource.filter_creator");
pub(crate) const RESOURCE_FILTER_FAVORITE: Key = Key::new("resource.filter_favorite");
pub(crate) const RESOURCE_FILTER_KIND: Key = Key::new("resource.filter_kind");
pub(crate) const RESOURCE_FILTER_PUBLISHED_FROM: Key = Key::new("resource.filter_published_from");
pub(crate) const RESOURCE_FILTER_PUBLISHED_TO: Key = Key::new("resource.filter_published_to");
pub(crate) const RESOURCE_FILTER_TITLE: Key = Key::new("resource.filter_title");
pub(crate) const RESOURCE_FILTER_SEARCH: Key = Key::new("resource.filter_search");
pub(crate) const RESOURCE_PAGE_LIMIT: Key = Key::new("resource.page_limit");
pub(crate) const RESOURCE_PAGE_OFFSET: Key = Key::new("resource.page_offset");
pub(crate) const RESOURCE_GRID_PARENT: Key = Key::new("resource.grid_parent");
pub(crate) const RESOURCE_PARENT_ID: Key = Key::new("resource.parent_id");
pub(crate) const RESOURCE_GRID_ROWS: Key = Key::new("resource.grid_rows");
pub(crate) const RESOURCE_GRID_HAS_MORE: Key = Key::new("resource.grid_has_more");
pub(crate) const RESOURCE_SORT_FIELD: Key = Key::new("resource.sort_field");
pub(crate) const RESOURCE_SORT_DIRECTION: Key = Key::new("resource.sort_direction");
pub(crate) const SERIES_ID: Key = Key::new("series.id");
pub(crate) const SERIES_TITLE: Key = Key::new("series.title");
pub(crate) const TAG_ID: Key = Key::new("tag.id");
pub(crate) const TAG_TEXT: Key = Key::new("tag.text");
pub(crate) const RESOURCE_FILTER_TAG: Key = Key::new("resource.filter_tag");
pub(crate) const RESOURCE_FILTER_COLLECTION: Key = Key::new("resource.filter_collection");
pub(crate) const COLLECTION_ID: Key = Key::new("collection.id");
pub(crate) const COLLECTION_NAME: Key = Key::new("collection.name");
pub(crate) const COLLECTION_PARENT_ID: Key = Key::new("collection.parent_id");
pub(crate) const COLLECTION_FOREST_ROOTS: Key = Key::new("collection.forest_roots");

pub(crate) const IMPORT_ENTRY_KEY: Key = Key::new("import.entry_key");
pub(crate) const IMPORT_IMPORTED_COUNT: Key = Key::new("import.imported_count");
pub(crate) const IMPORT_SKIPPED_COUNT: Key = Key::new("import.skipped_count");
pub(crate) const IMPORT_WARNING_COUNT: Key = Key::new("import.warning_count");
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
        PERSON_ID,
        PERSON_DISPLAY,
        RESOURCE_ID,
        RESOURCE_KIND,
        RESOURCE_TITLE,
        RESOURCE_CHANGED_FIELDS,
        RESOURCE_FAVORITE,
        RESOURCE_FILTER_CREATOR,
        RESOURCE_FILTER_FAVORITE,
        RESOURCE_FILTER_KIND,
        RESOURCE_FILTER_PUBLISHED_FROM,
        RESOURCE_FILTER_PUBLISHED_TO,
        RESOURCE_FILTER_TITLE,
        RESOURCE_FILTER_SEARCH,
        RESOURCE_PAGE_LIMIT,
        RESOURCE_PAGE_OFFSET,
        RESOURCE_GRID_PARENT,
        RESOURCE_PARENT_ID,
        RESOURCE_GRID_ROWS,
        RESOURCE_GRID_HAS_MORE,
        RESOURCE_SORT_FIELD,
        RESOURCE_SORT_DIRECTION,
        SERIES_ID,
        SERIES_TITLE,
        TAG_ID,
        TAG_TEXT,
        RESOURCE_FILTER_TAG,
        RESOURCE_FILTER_COLLECTION,
        COLLECTION_ID,
        COLLECTION_NAME,
        COLLECTION_PARENT_ID,
        COLLECTION_FOREST_ROOTS,
        IMPORT_ENTRY_KEY,
        IMPORT_IMPORTED_COUNT,
        IMPORT_SKIPPED_COUNT,
        IMPORT_WARNING_COUNT,
    ];

    #[test]
    fn key_strings_are_unique_and_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for key in ALL {
            let s = key.as_str();
            assert!(seen.insert(s), "duplicate key string: {s}");
            assert!(
                s.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c == '.')
                    && !s.is_empty()
                    && !s.starts_with('.')
                    && !s.ends_with('.')
                    && !s.contains(".."),
                "malformed key: {s}"
            );
        }
    }
}
