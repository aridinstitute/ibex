use std::fmt;

use crate::NonEmptyStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SeriesId(ulid::Ulid);

impl SeriesId {
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

impl fmt::Display for SeriesId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NewSeries {
    pub title: NonEmptyStr,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Series {
    pub id: SeriesId,
    pub title: NonEmptyStr,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SeriesError {
    // Title emptiness lives in NonEmptyStr; reserved for future invariants.
}

impl Series {
    pub fn try_new(id: SeriesId, new: NewSeries) -> Result<Self, SeriesError> {
        Ok(Self {
            id,
            title: new.title,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_id_parse_round_trip() {
        let id = SeriesId::generate();
        let parsed = SeriesId::parse(&id.to_string()).unwrap();
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_ulid(), id.as_ulid());
    }

    #[test]
    fn try_new_keeps_title() {
        let title = NonEmptyStr::try_new("Loeb Classical Library").unwrap();
        let series = Series::try_new(
            SeriesId::generate(),
            NewSeries {
                title: title.clone(),
            },
        )
        .unwrap();
        assert_eq!(series.title, title);
    }
}
