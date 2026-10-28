use std::fmt;

use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

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

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NameError {
    #[error("name requires a non-empty first or last")]
    Empty,
}

/// Mononyms (Augustine) live in `last` — the primary indexing / sort slot in bibliography
/// convention. Nobiliary particles stay in [`Self::particle`], not folded into `last`.
/// Organizational names set [`Self::is_literal`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Name {
    first: Option<NonEmptyStr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    particle: Option<NonEmptyStr>,
    last: Option<NonEmptyStr>,
    suffix: Option<NonEmptyStr>,
    /// Corporate / organizational name: opaque for matching, displayed as `last`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    literal: bool,
}

impl Name {
    pub fn try_new(
        first: Option<impl AsRef<str>>,
        last: Option<impl AsRef<str>>,
        suffix: Option<impl AsRef<str>>,
    ) -> Result<Self, NameError> {
        Self::try_new_parts(first, None::<&str>, last, suffix)
    }

    pub fn try_new_parts(
        first: Option<impl AsRef<str>>,
        particle: Option<impl AsRef<str>>,
        last: Option<impl AsRef<str>>,
        suffix: Option<impl AsRef<str>>,
    ) -> Result<Self, NameError> {
        let first = normalize_part(first)?;
        let particle = normalize_part(particle)?;
        let last = normalize_part(last)?;
        let suffix = normalize_part(suffix)?;
        if first.is_none() && last.is_none() {
            return Err(NameError::Empty);
        }
        Ok(Self {
            first,
            particle,
            last,
            suffix,
            literal: false,
        })
    }

    /// Organizational or otherwise opaque credited name (stored in `last`).
    pub fn corporate(name: impl AsRef<str>) -> Result<Self, NameError> {
        let last = normalize_part(Some(name))?;
        let Some(last) = last else {
            return Err(NameError::Empty);
        };
        Ok(Self {
            first: None,
            particle: None,
            last: Some(last),
            suffix: None,
            literal: true,
        })
    }

    pub fn first(&self) -> Option<&str> {
        self.first.as_ref().map(NonEmptyStr::as_str)
    }

    pub fn particle(&self) -> Option<&str> {
        self.particle.as_ref().map(NonEmptyStr::as_str)
    }

    pub fn last(&self) -> Option<&str> {
        self.last.as_ref().map(NonEmptyStr::as_str)
    }

    pub fn suffix(&self) -> Option<&str> {
        self.suffix.as_ref().map(NonEmptyStr::as_str)
    }

    pub fn is_literal(&self) -> bool {
        self.literal
    }

    /// Human display: "First Particle Last, Suffix" / corporate `last` alone.
    pub fn display(&self) -> String {
        if self.literal {
            return self.last().unwrap_or("").to_owned();
        }
        let mut out = String::new();
        if let Some(first) = self.first() {
            out.push_str(first);
        }
        if let Some(particle) = self.particle() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(particle);
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

    pub fn key(&self) -> NameKey {
        NameKey::from_name(self)
    }
}

/// Index identity for a structured [`Name`].
///
/// Wire form is Unicode-normalized (NFKD, marks stripped, lowercased)
/// `last|particle|first|suffix` (missing part → empty). Literal names use
/// `*|normalized-display|||` so they do not collide with personal name slots.
/// Not the display string — formatting must not change which people a lookup hits.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NameKey(String);

impl NameKey {
    pub fn from_name(name: &Name) -> Self {
        if name.is_literal() {
            return Self(format!("*|{}|||", normalize_key_part(name.last().unwrap_or(""))));
        }
        Self(format!(
            "{}|{}|{}|{}",
            normalize_key_part(name.last().unwrap_or("")),
            normalize_key_part(name.particle().unwrap_or("")),
            normalize_key_part(name.first().unwrap_or("")),
            normalize_key_part(name.suffix().unwrap_or(""))
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Fold accents for matching without treating the fold as a unique identity.
fn normalize_key_part(s: &str) -> String {
    s.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn normalize_part(part: Option<impl AsRef<str>>) -> Result<Option<NonEmptyStr>, NameError> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PersonId(ulid::Ulid);

impl PersonId {
    pub fn generate() -> Self {
        Self(ulid::Ulid::generate())
    }

    pub fn parse(s: &str) -> Result<Self, ulid::DecodeError> {
        Ok(Self(ulid::Ulid::from_string(s)?))
    }

    pub fn as_ulid(self) -> ulid::Ulid {
        self.0
    }
}

impl fmt::Display for PersonId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PersonError {
    #[error("alias must not equal the primary name")]
    AliasDuplicatesPrimary,
    #[error("duplicate alias")]
    DuplicateAlias,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Person {
    pub id: PersonId,
    pub name: Name,
    pub aliases: Vec<Name>,
}

impl Person {
    pub fn try_new(id: PersonId, name: Name, aliases: Vec<Name>) -> Result<Self, PersonError> {
        for (i, alias) in aliases.iter().enumerate() {
            if alias == &name {
                return Err(PersonError::AliasDuplicatesPrimary);
            }
            if aliases[..i].contains(alias) {
                return Err(PersonError::DuplicateAlias);
            }
        }
        Ok(Self { id, name, aliases })
    }

    pub fn indexed_names(&self) -> impl Iterator<Item = &Name> {
        std::iter::once(&self.name).chain(self.aliases.iter())
    }

    pub fn name_keys(&self) -> impl Iterator<Item = NameKey> + '_ {
        self.indexed_names().map(Name::key)
    }
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
        assert_eq!(parsed.as_ulid(), id.as_ulid());
    }

    #[test]
    fn person_id_parse_rejects_garbage() {
        assert!(PersonId::parse("not-a-ulid").is_err());
    }

    #[test]
    fn name_key_stable_across_display_formatting() {
        let a = Name::try_new(Some("Mark"), Some("Twain"), None::<&str>).unwrap();
        let b = Name::try_new(Some("MARK"), Some("TWAIN"), None::<&str>).unwrap();
        assert_eq!(a.key(), b.key());
        assert_eq!(a.key().as_str(), "twain||mark|");
    }

    #[test]
    fn name_key_mononym_in_last() {
        let name = Name::try_new(None::<&str>, Some("Augustine"), None::<&str>).unwrap();
        assert_eq!(name.key().as_str(), "augustine|||");
    }

    #[test]
    fn name_key_includes_suffix() {
        let name = Name::try_new(Some("Jane"), Some("Doe"), Some("Jr.")).unwrap();
        assert_eq!(name.key().as_str(), "doe||jane|jr.");
    }

    #[test]
    fn name_keeps_particle_separate() {
        let name =
            Name::try_new_parts(Some("Ludwig"), Some("van"), Some("Beethoven"), None::<&str>)
                .unwrap();
        assert_eq!(name.particle(), Some("van"));
        assert_eq!(name.last(), Some("Beethoven"));
        assert_eq!(name.display(), "Ludwig van Beethoven");
        assert_eq!(name.key().as_str(), "beethoven|van|ludwig|");
    }

    #[test]
    fn name_key_folds_accents_without_changing_display() {
        let accented = Name::try_new(Some("José"), Some("García"), None::<&str>).unwrap();
        let plain = Name::try_new(Some("Jose"), Some("Garcia"), None::<&str>).unwrap();
        assert_eq!(accented.key(), plain.key());
        assert_eq!(accented.display(), "José García");
    }

    #[test]
    fn corporate_name_is_literal() {
        let name = Name::corporate("World Bank").unwrap();
        assert!(name.is_literal());
        assert_eq!(name.display(), "World Bank");
        assert!(name.key().as_str().starts_with("*|"));
    }

    #[test]
    fn person_try_new_rejects_alias_equal_primary() {
        let name = Name::try_new(Some("Mark"), Some("Twain"), None::<&str>).unwrap();
        let err = Person::try_new(PersonId::generate(), name.clone(), vec![name]).unwrap_err();
        assert_eq!(err, PersonError::AliasDuplicatesPrimary);
    }

    #[test]
    fn person_try_new_rejects_duplicate_aliases() {
        let primary = Name::try_new(Some("Samuel"), Some("Clemens"), None::<&str>).unwrap();
        let alias = Name::try_new(Some("Mark"), Some("Twain"), None::<&str>).unwrap();
        let err =
            Person::try_new(PersonId::generate(), primary, vec![alias.clone(), alias]).unwrap_err();
        assert_eq!(err, PersonError::DuplicateAlias);
    }

    #[test]
    fn person_indexed_names_primary_then_aliases() {
        let primary = Name::try_new(Some("Samuel"), Some("Clemens"), None::<&str>).unwrap();
        let alias = Name::try_new(Some("Mark"), Some("Twain"), None::<&str>).unwrap();
        let person =
            Person::try_new(PersonId::generate(), primary.clone(), vec![alias.clone()]).unwrap();
        let names: Vec<_> = person.indexed_names().cloned().collect();
        assert_eq!(names, vec![primary, alias]);
        assert_eq!(person.name_keys().count(), 2);
    }
}
