//! Person identity and structured personal names.

use std::fmt;

/// Trimmed non-empty string. Empty or whitespace-only input is rejected.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct NonEmptyStr(String);

impl NonEmptyStr {
    pub fn try_new(s: impl AsRef<str>) -> Result<Self, NameError> {
        let trimmed = s.as_ref().trim();
        if trimmed.is_empty() {
            Err(NameError::Empty)
        } else {
            Ok(Self(trimmed.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for NonEmptyStr {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Failure to form a [`Name`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NameError {
    #[error("name requires a non-empty first or last")]
    Empty,
}

/// Structured personal name. At least one of first or last is present after `try_new`.
///
/// Mononyms (Augustine) live in `last` — the primary indexing / sort slot in bibliography
/// convention.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Name {
    first: Option<NonEmptyStr>,
    last: Option<NonEmptyStr>,
    suffix: Option<NonEmptyStr>,
}

impl Name {
    /// Parse at the domain boundary. Trims; empty strings become `None`.
    /// Requires at least one of first or last; suffix alone is invalid.
    pub fn try_new(
        first: Option<impl AsRef<str>>,
        last: Option<impl AsRef<str>>,
        suffix: Option<impl AsRef<str>>,
    ) -> Result<Self, NameError> {
        let first = normalize_part(first)?;
        let last = normalize_part(last)?;
        let suffix = normalize_part(suffix)?;
        if first.is_none() && last.is_none() {
            return Err(NameError::Empty);
        }
        Ok(Self {
            first,
            last,
            suffix,
        })
    }

    pub fn first(&self) -> Option<&str> {
        self.first.as_ref().map(NonEmptyStr::as_str)
    }

    pub fn last(&self) -> Option<&str> {
        self.last.as_ref().map(NonEmptyStr::as_str)
    }

    pub fn suffix(&self) -> Option<&str> {
        self.suffix.as_ref().map(NonEmptyStr::as_str)
    }

    /// Human display: "First Last, Suffix" / "Last" / "First" / "First, Suffix".
    pub fn display(&self) -> String {
        let mut out = String::new();
        if let Some(first) = self.first() {
            out.push_str(first);
        }
        if let Some(last) = self.last() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(last);
        }
        if let Some(suffix) = self.suffix() {
            out.push_str(", ");
            out.push_str(suffix);
        }
        out
    }
}

fn normalize_part(
    part: Option<impl AsRef<str>>,
) -> Result<Option<NonEmptyStr>, NameError> {
    match part {
        None => Ok(None),
        Some(s) => {
            let trimmed = s.as_ref().trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(NonEmptyStr::try_new(trimmed)?))
            }
        }
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

/// Opaque person identity. Not interchangeable with other ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PersonId(uuid::Uuid);

impl PersonId {
    /// Allocated by store implementors on create. Not for shells to invent casually.
    pub fn generate() -> Self {
        Self(uuid::Uuid::now_v7())
    }

    pub fn parse(s: &str) -> Result<Self, uuid::Error> {
        Ok(Self(uuid::Uuid::parse_str(s)?))
    }

    pub fn as_uuid(self) -> uuid::Uuid {
        self.0
    }
}

impl fmt::Display for PersonId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Stored person record. Commands return this; store is sole writer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Person {
    pub id: PersonId,
    pub name: Name,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_rejects_empty() {
        assert_eq!(
            Name::try_new(None::<&str>, None::<&str>, None::<&str>),
            Err(NameError::Empty)
        );
    }

    #[test]
    fn name_rejects_whitespace_only() {
        assert_eq!(
            Name::try_new(Some("  "), Some("\t"), Some("  ")),
            Err(NameError::Empty)
        );
    }

    #[test]
    fn name_rejects_suffix_alone() {
        assert_eq!(
            Name::try_new(None::<&str>, None::<&str>, Some("Jr.")),
            Err(NameError::Empty)
        );
    }

    #[test]
    fn name_mononym_in_last() {
        let name = Name::try_new(None::<&str>, Some("Augustine"), None::<&str>).unwrap();
        assert_eq!(name.first(), None);
        assert_eq!(name.last(), Some("Augustine"));
        assert_eq!(name.suffix(), None);
        assert_eq!(name.display(), "Augustine");
    }

    #[test]
    fn name_structured_with_suffix() {
        let name = Name::try_new(Some("Jane"), Some("Doe"), Some("Jr.")).unwrap();
        assert_eq!(name.first(), Some("Jane"));
        assert_eq!(name.last(), Some("Doe"));
        assert_eq!(name.suffix(), Some("Jr."));
        assert_eq!(name.display(), "Jane Doe, Jr.");
    }

    #[test]
    fn name_trims_parts() {
        let name = Name::try_new(Some("  Jane  "), Some(" Doe "), Some(" Jr. ")).unwrap();
        assert_eq!(name.first(), Some("Jane"));
        assert_eq!(name.last(), Some("Doe"));
        assert_eq!(name.suffix(), Some("Jr."));
    }

    #[test]
    fn person_id_parse_round_trip() {
        let id = PersonId::generate();
        let parsed = PersonId::parse(&id.to_string()).unwrap();
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_uuid(), id.as_uuid());
    }

    #[test]
    fn person_id_parse_rejects_garbage() {
        assert!(PersonId::parse("not-a-uuid").is_err());
    }
}
