//! Markdown notes are independent records, not resource metadata.
use crate::{ResourceId, StoreError, UtcMillis};
use std::fmt;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct NoteId(ulid::Ulid);

impl NoteId {
    /// Monotonic so two notes created in the same millisecond still list in
    /// creation order. `list_notes` sorts by `created_at`, then id, and a
    /// random id can sort before one created earlier in that same millisecond.
    pub fn generate() -> Self {
        static GENERATOR: Mutex<ulid::Generator> = Mutex::new(ulid::Generator::new());
        let mut generator = GENERATOR
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let id = generator
            .generate()
            .unwrap_or_else(|overflow| overflow.commit_overflow_increment());
        Self(id)
    }
    pub fn parse(s: &str) -> Result<Self, ulid::DecodeError> {
        Ok(Self(ulid::Ulid::from_string(s)?))
    }
}
impl fmt::Display for NoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    #[default]
    Note,
    Summary,
    Paraphrase,
    Quote,
}

impl NoteKind {
    pub const ALL: [Self; 4] = [Self::Note, Self::Summary, Self::Paraphrase, Self::Quote];
    pub fn label(self) -> &'static str {
        match self {
            Self::Note => "Note",
            Self::Summary => "Summary",
            Self::Paraphrase => "Paraphrase",
            Self::Quote => "Quote",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationKind {
    #[default]
    Page,
    Chapter,
    Section,
    Timestamp,
}

impl LocationKind {
    pub const ALL: [Self; 4] = [Self::Page, Self::Chapter, Self::Section, Self::Timestamp];
    pub fn label(self) -> &'static str {
        match self {
            Self::Page => "Page",
            Self::Chapter => "Chapter",
            Self::Section => "Section",
            Self::Timestamp => "Timestamp",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CitationLocation {
    pub kind: LocationKind,
    /// Retains ranges, Roman numerals, and nonnumeric section labels.
    pub value: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NoteContent {
    pub kind: NoteKind,
    /// Preserve whitespace: trimming can change Markdown's meaning.
    pub body_markdown: String,
    pub locations: Vec<CitationLocation>,
}
impl NoteContent {
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.body_markdown.trim().is_empty() {
            return Err(StoreError::invalid("note body must not be empty"));
        }
        if self
            .locations
            .iter()
            .any(|location| location.value.trim().is_empty())
        {
            return Err(StoreError::invalid("citation location must not be empty"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Note {
    pub id: NoteId,
    /// Optional in the model for standalone notes; creation currently requires a resource.
    pub resource_id: Option<ResourceId>,
    pub content: NoteContent,
    pub created_at: UtcMillis,
    pub updated_at: UtcMillis,
}

/// One row in the cross-resource notes feed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NoteFeedItem {
    pub note: Note,
    /// Title of the linked resource when the note still has one.
    pub resource_title: Option<crate::NonEmptyStr>,
}

#[cfg(test)]
mod tests {
    use super::NoteId;

    #[test]
    fn generated_ids_increase() {
        let mut previous = NoteId::generate().to_string();
        for _ in 0..32 {
            let next = NoteId::generate().to_string();
            assert!(previous < next);
            previous = next;
        }
    }
}
