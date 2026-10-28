mod keys;
mod sink;

pub(crate) use keys::Key;
pub(crate) use keys::{
    APP_UPTIME_MS, APP_VERSION, COLLECTION_FOREST_ROOTS, COLLECTION_ID, COLLECTION_NAME,
    COLLECTION_PARENT_ID, DURATION_US, ERROR_KIND, ERROR_MESSAGE, IMPORT_ENTRY_KEY,
    IMPORT_IMPORTED_COUNT, IMPORT_SKIPPED_COUNT, IMPORT_WARNING_COUNT, PERSON_DISPLAY, PERSON_ID,
    REQUEST_ID, RESOURCE_CHANGED_FIELDS, RESOURCE_FAVORITE, RESOURCE_FILTER_COLLECTION,
    RESOURCE_FILTER_CREATOR, RESOURCE_FILTER_FAVORITE, RESOURCE_FILTER_KIND,
    RESOURCE_FILTER_PUBLISHED_FROM, RESOURCE_FILTER_PUBLISHED_TO, RESOURCE_FILTER_SEARCH,
    RESOURCE_FILTER_TAG, RESOURCE_FILTER_TITLE, RESOURCE_GRID_HAS_MORE, RESOURCE_GRID_PARENT,
    RESOURCE_GRID_ROWS, RESOURCE_ID, RESOURCE_KIND, RESOURCE_PAGE_LIMIT, RESOURCE_PAGE_OFFSET,
    RESOURCE_PARENT_ID, RESOURCE_SORT_DIRECTION, RESOURCE_SORT_FIELD, RESOURCE_TITLE, SERIES_ID,
    SERIES_TITLE, STATUS, STORE_BACKEND, STORE_LATENCY_US, STORE_OK, SURFACE, TAG_ID, TAG_TEXT,
};
pub use sink::JsonLinesSink;

use std::borrow::Cow;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::Surface;

/// ULID: time-ordered, so execution ids sort with wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub struct ExecutionId(ulid::Ulid);

impl ExecutionId {
    pub(crate) fn generate() -> Self {
        Self(ulid::Ulid::generate())
    }
}

impl std::fmt::Display for ExecutionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Scalar values only — wide events stay flat for columnar ingestion.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(untagged)]
pub enum FieldValue {
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(Cow<'static, str>),
}

impl From<bool> for FieldValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<i64> for FieldValue {
    fn from(v: i64) -> Self {
        Self::I64(v)
    }
}

impl From<u64> for FieldValue {
    fn from(v: u64) -> Self {
        Self::U64(v)
    }
}

impl From<u32> for FieldValue {
    fn from(v: u32) -> Self {
        Self::U64(u64::from(v))
    }
}

impl From<usize> for FieldValue {
    fn from(v: usize) -> Self {
        Self::U64(v as u64)
    }
}

impl From<f64> for FieldValue {
    fn from(v: f64) -> Self {
        Self::F64(v)
    }
}

impl From<&'static str> for FieldValue {
    fn from(v: &'static str) -> Self {
        Self::Str(Cow::Borrowed(v))
    }
}

impl From<String> for FieldValue {
    fn from(v: String) -> Self {
        Self::Str(Cow::Owned(v))
    }
}

impl From<Surface> for FieldValue {
    fn from(v: Surface) -> Self {
        Self::Str(Cow::Borrowed(v.as_str()))
    }
}

impl From<ExecutionId> for FieldValue {
    fn from(v: ExecutionId) -> Self {
        Self::Str(Cow::Owned(v.to_string()))
    }
}

/// In-flight event for one execution. Only `EmitGuard` creates/finishes it.
pub(crate) struct Event {
    id: ExecutionId,
    command: &'static str,
    ts_unix_ms: u64,
    fields: Vec<(Key, FieldValue)>,
}

impl Event {
    pub(crate) fn new(id: ExecutionId, command: &'static str) -> Self {
        let ts_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self {
            id,
            command,
            ts_unix_ms,
            fields: Vec::new(),
        }
    }

    pub(crate) fn id(&self) -> ExecutionId {
        self.id
    }

    pub(crate) fn set(&mut self, key: Key, value: impl Into<FieldValue>) {
        self.fields.push((key, value.into()));
    }

    pub(crate) fn finish(mut self, status: Status, elapsed: Duration) -> FinishedEvent {
        self.set(STATUS, status.as_str());
        self.set(DURATION_US, elapsed.as_micros() as u64);

        let mut deduped: Vec<(Key, FieldValue)> = Vec::with_capacity(self.fields.len());
        for (key, value) in self.fields {
            if let Some((_, slot)) = deduped.iter_mut().find(|(k, _)| *k == key) {
                *slot = value;
            } else {
                deduped.push((key, value));
            }
        }

        FinishedEvent {
            id: self.id,
            command: self.command,
            status,
            duration: elapsed,
            ts_unix_ms: self.ts_unix_ms,
            fields: deduped.into_iter().map(|(k, v)| (k.as_str(), v)).collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Error,
    Cancelled,
    Panicked,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Error => "error",
            Status::Cancelled => "cancelled",
            Status::Panicked => "panicked",
        }
    }
}

/// Immutable completed event. What sinks receive and tests assert on.
#[derive(Debug, Clone)]
pub struct FinishedEvent {
    id: ExecutionId,
    command: &'static str,
    status: Status,
    duration: Duration,
    ts_unix_ms: u64,
    fields: Vec<(&'static str, FieldValue)>,
}

impl FinishedEvent {
    pub fn id(&self) -> ExecutionId {
        self.id
    }

    pub fn command(&self) -> &'static str {
        self.command
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    pub fn get(&self, key: &str) -> Option<&FieldValue> {
        self.fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    pub fn fields(&self) -> impl Iterator<Item = (&'static str, &FieldValue)> {
        self.fields.iter().map(|(k, v)| (*k, v))
    }
}

impl serde::Serialize for FinishedEvent {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;

        let base = [
            "ts_unix_ms",
            "execution_id",
            "command",
            "status",
            "duration_us",
        ];
        let mut map = serializer.serialize_map(Some(base.len() + self.fields.len()))?;
        map.serialize_entry("ts_unix_ms", &self.ts_unix_ms)?;
        map.serialize_entry("execution_id", &self.id)?;
        map.serialize_entry("command", &self.command)?;
        map.serialize_entry("status", &self.status.as_str())?;
        map.serialize_entry("duration_us", &(self.duration.as_micros() as u64))?;
        for (key, value) in &self.fields {
            if base.contains(key) {
                continue;
            }
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// Where canonical lines go. Called exactly once per `execute`.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: &FinishedEvent);
}
