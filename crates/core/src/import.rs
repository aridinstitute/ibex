//! Format-neutral bibliography import drafts and diagnostics.
//!
//! Parsers produce these types without store access. Shared orchestration
//! resolves entities and persists; preview/conflict UX builds on the same
//! draft later.

use std::collections::BTreeMap;
use std::fmt;

use crate::{
    CreatorRole, Name, NewResource, NonEmptyStr, ResourceId, ResourceKind,
};

/// Parser/schema version stamped on drafts from this crate revision.
pub const IMPORT_PARSER_VERSION: u32 = 2;

/// Shared source-size ceiling applied at the import boundary (not only GUI).
pub const IMPORT_MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;

/// Stable diagnostic codes for import parse and orchestration.
pub mod codes {
    pub const SOURCE_TOO_LARGE: &str = "import.source_too_large";
    pub const SYNTAX: &str = "import.syntax";
    pub const DUPLICATE_KEY: &str = "import.duplicate_key";
    pub const REFERENCE_CYCLE: &str = "import.reference_cycle";
    pub const REFERENCE_TOO_DEEP: &str = "import.reference_too_deep";
    pub const REFERENCE_EXPAND: &str = "import.reference_expand";
    pub const UNSUPPORTED_TYPE: &str = "import.unsupported_type";
    pub const MISSING_TITLE: &str = "import.missing_title";
    pub const INVALID_FIELD: &str = "import.invalid_field";
    pub const AMBIGUOUS_CREATOR: &str = "import.ambiguous_creator";
    pub const REFERENCE_NO_PARENT_LINK: &str = "import.reference_no_parent_link";
    pub const ENTRY_SKIPPED: &str = "import.entry_skipped";
}

/// Temporary identity local to one [`ImportDraft`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ImportTempId(u64);

impl ImportTempId {
    pub fn new(n: u64) -> Self {
        Self(n)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ImportTempId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "temp:{}", self.0)
    }
}

/// Source format that produced a draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportFormat {
    Biblatex,
    /// In-memory draft for tests proving a second parser can reuse save.
    Test,
}

/// Provenance for an entire import draft.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourceProvenance {
    pub format: ImportFormat,
    pub parser_version: u32,
}

/// Provenance retained for one source entry.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EntryProvenance {
    pub source_key: String,
    pub entry_type: Option<String>,
    /// Field map before reference expansion (includes note/annotation).
    pub original_fields: BTreeMap<String, String>,
    /// Field map after reference expansion / mapping (excludes note/annotation).
    #[serde(default)]
    pub expanded_fields: BTreeMap<String, String>,
    /// Field → ancestor source key for inherited values.
    #[serde(default)]
    pub inherited_origins: BTreeMap<String, String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub annotation: Option<String>,
}

/// Creator not yet matched to a stored person.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UnresolvedCreator {
    pub role: CreatorRole,
    pub name: Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

/// Structured diagnostic from parse or later import stages.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub entry_key: Option<String>,
    pub field: Option<String>,
    pub location: Option<String>,
    pub message: String,
}

impl ImportDiagnostic {
    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            severity: DiagnosticSeverity::Error,
            entry_key: None,
            field: None,
            location: None,
            message: message.into(),
        }
    }

    pub fn warning(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            severity: DiagnosticSeverity::Warning,
            entry_key: None,
            field: None,
            location: None,
            message: message.into(),
        }
    }

    pub fn with_entry(mut self, key: impl Into<String>) -> Self {
        self.entry_key = Some(key.into());
        self
    }

    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }
}

/// One successfully mapped source entry awaiting entity resolution.
///
/// Creators and series remain unresolved; `resource.creators` stays empty and
/// `resource.series_id` unset until shared orchestration runs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportEntryDraft {
    pub temp_id: ImportTempId,
    /// Mapped fields; creators empty and series unset until orchestration.
    pub resource: NewResource,
    pub unresolved_creators: Vec<UnresolvedCreator>,
    pub series_title: Option<String>,
    pub series_number: Option<NonEmptyStr>,
    pub provenance: EntryProvenance,
    /// True when the source used crossref/xref (metadata inheritance only today).
    pub had_reference_fields: bool,
}

impl ImportEntryDraft {
    pub fn source_key(&self) -> &str {
        &self.provenance.source_key
    }

    pub fn kind(&self) -> ResourceKind {
        self.resource.kind
    }

    pub fn title(&self) -> &NonEmptyStr {
        &self.resource.title
    }
}

/// Format-neutral import draft: parse output before matching or persistence.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportDraft {
    pub provenance: SourceProvenance,
    pub entries: Vec<ImportEntryDraft>,
    pub diagnostics: Vec<ImportDiagnostic>,
}

impl ImportDraft {
    pub fn empty(format: ImportFormat) -> Self {
        Self {
            provenance: SourceProvenance {
                format,
                parser_version: IMPORT_PARSER_VERSION,
            },
            entries: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

/// Outcome category for a source entry after save (Task 06 fills more).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportEntryOutcomeKind {
    Imported,
    Reused,
    Skipped,
    Conflicted,
    Failed,
    NotAttempted,
}

/// Committed resource produced by an import entry.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportedEntry {
    pub entry_key: String,
    pub resource_id: ResourceId,
}

/// Human-facing issue row; may carry a stable [`codes`] value.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportIssue {
    pub entry_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
}

impl ImportIssue {
    pub fn new(entry_key: Option<String>, message: impl Into<String>) -> Self {
        Self {
            entry_key,
            code: None,
            message: message.into(),
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}

/// Import completion report with categories reserved for later tasks.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportReport {
    pub imported: Vec<ImportedEntry>,
    #[serde(default)]
    pub reused: Vec<ImportedEntry>,
    pub skipped: Vec<ImportIssue>,
    #[serde(default)]
    pub conflicted: Vec<ImportIssue>,
    #[serde(default)]
    pub failed: Vec<ImportIssue>,
    #[serde(default)]
    pub not_attempted: Vec<ImportIssue>,
    pub warnings: Vec<ImportIssue>,
}
