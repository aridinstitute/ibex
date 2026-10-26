use std::fmt;

use crate::NonEmptyStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TagId(ulid::Ulid);

impl TagId {
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

impl fmt::Display for TagId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Library tag: free-text label, unique by exact stored string.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Tag {
    pub id: TagId,
    pub text: NonEmptyStr,
}

impl Tag {
    pub fn try_new(id: TagId, text: NonEmptyStr) -> Self {
        Self { id, text }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_id_parse_round_trip() {
        let id = TagId::generate();
        let parsed = TagId::parse(&id.to_string()).unwrap();
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_ulid(), id.as_ulid());
    }

    #[test]
    fn try_new_keeps_text() {
        let text = NonEmptyStr::try_new("epistemology").unwrap();
        let tag = Tag::try_new(TagId::generate(), text.clone());
        assert_eq!(tag.text, text);
    }
}
