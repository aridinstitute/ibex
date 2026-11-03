use std::{fmt, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct AttachmentId(ulid::Ulid);

impl AttachmentId {
    pub fn generate() -> Self {
        Self(ulid::Ulid::generate())
    }

    pub fn parse(s: &str) -> Result<Self, ulid::DecodeError> {
        Ok(Self(ulid::Ulid::from_string(s)?))
    }
}

impl fmt::Display for AttachmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Attachment {
    pub id: AttachmentId,
    pub attachment_type: String,
    pub title: String,
    pub file_path: PathBuf,
}
