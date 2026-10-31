//! Structured page ranges with lossless literal fallback.

use crate::{NameError, NonEmptyStr};

/// One bound of a page span: optional numeric value plus retained spelling.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PageBound {
    pub literal: NonEmptyStr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<NonEmptyStr>,
}

impl PageBound {
    pub fn parse(raw: &str) -> Result<Self, NameError> {
        let trimmed = raw.trim();
        let literal = NonEmptyStr::try_new(trimmed)?;
        let (prefix, digits) = split_prefix_digits(trimmed);
        let number = digits.and_then(|d| d.parse().ok());
        let prefix = match prefix {
            Some(p) if !p.is_empty() => Some(NonEmptyStr::try_new(p)?),
            _ => None,
        };
        Ok(Self {
            literal,
            number,
            prefix,
        })
    }
}

/// A single page or an inclusive range (`10--20`, `S10-S20`, `iv-xii`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PageSpan {
    pub start: PageBound,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<PageBound>,
}

/// Page field: always keeps the source literal; spans are filled when recognized.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Pages {
    pub literal: NonEmptyStr,
    #[serde(default)]
    pub spans: Vec<PageSpan>,
}

impl<'de> serde::Deserialize<'de> for Pages {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Structured {
            literal: NonEmptyStr,
            #[serde(default)]
            spans: Vec<PageSpan>,
        }
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Literal(String),
            Structured(Structured),
        }
        match Wire::deserialize(deserializer)? {
            Wire::Literal(s) => Self::parse(&s).map_err(serde::de::Error::custom),
            Wire::Structured(Structured { literal, spans }) => Ok(Self { literal, spans }),
        }
    }
}

impl Pages {
    /// Parse a BibLaTeX-style pages value. Unrecognized input still round-trips
    /// via [`Self::literal`] with an empty [`Self::spans`] list.
    pub fn parse(raw: &str) -> Result<Self, NameError> {
        let trimmed = raw.trim();
        let literal = NonEmptyStr::try_new(trimmed)?;
        let normalized = trimmed
            .replace(['\u{2013}', '\u{2014}'], "-")
            .replace("--", "-");
        let spans = normalized
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(parse_span)
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default();
        // If any fragment failed to form a bound, treat the whole value as
        // unrecognized rather than a partial structured list.
        let spans = if spans_cover_literal(&spans, &normalized) {
            spans
        } else {
            // Retry: if the whole string is one span, keep it; else empty.
            match parse_span(&normalized) {
                Ok(span) => vec![span],
                Err(_) => Vec::new(),
            }
        };
        Ok(Self { literal, spans })
    }

    pub fn as_str(&self) -> &str {
        self.literal.as_str()
    }

    pub fn is_structured(&self) -> bool {
        !self.spans.is_empty()
    }
}

impl AsRef<str> for Pages {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for Pages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Accept either a plain string (legacy rows / JSON fixtures) or a structured object.
pub fn deserialize_pages_opt<'de, D>(deserializer: D) -> Result<Option<Pages>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Literal(String),
        Structured(Pages),
    }
    Ok(match Option::<Wire>::deserialize(deserializer)? {
        None => None,
        Some(Wire::Structured(pages)) => Some(pages),
        Some(Wire::Literal(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(Pages::parse(trimmed).map_err(serde::de::Error::custom)?)
            }
        }
    })
}

fn parse_span(raw: &str) -> Result<PageSpan, NameError> {
    let trimmed = raw.trim();
    if let Some((start, end)) = trimmed.split_once('-') {
        let start = start.trim();
        let end = end.trim();
        if start.is_empty() || end.is_empty() {
            return Err(NameError::Empty);
        }
        Ok(PageSpan {
            start: PageBound::parse(start)?,
            end: Some(PageBound::parse(end)?),
        })
    } else {
        Ok(PageSpan {
            start: PageBound::parse(trimmed)?,
            end: None,
        })
    }
}

fn split_prefix_digits(raw: &str) -> (Option<&str>, Option<&str>) {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        i += 1;
    }
    if i == 0 {
        // Pure digits or roman/other — no alphabetic prefix.
        if raw.chars().all(|c| c.is_ascii_digit()) {
            return (None, Some(raw));
        }
        return (None, None);
    }
    let prefix = &raw[..i];
    let rest = &raw[i..];
    if rest.chars().all(|c| c.is_ascii_digit()) && !rest.is_empty() {
        (Some(prefix), Some(rest))
    } else {
        (None, None)
    }
}

fn spans_cover_literal(spans: &[PageSpan], normalized: &str) -> bool {
    if spans.is_empty() {
        return false;
    }
    // Every comma-separated fragment produced a span.
    let parts = normalized
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .count();
    parts == spans.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numeric_range_with_en_dash_or_double_hyphen() {
        for raw in ["10--20", "10–20", "10-20"] {
            let pages = Pages::parse(raw).unwrap();
            assert_eq!(pages.as_str(), raw.trim());
            assert_eq!(pages.spans.len(), 1);
            assert_eq!(pages.spans[0].start.number, Some(10));
            assert_eq!(pages.spans[0].end.as_ref().unwrap().number, Some(20));
        }
    }

    #[test]
    fn parses_prefixed_range() {
        let pages = Pages::parse("S10-S20").unwrap();
        assert!(pages.is_structured());
        assert_eq!(
            pages.spans[0]
                .start
                .prefix
                .as_ref()
                .map(NonEmptyStr::as_str),
            Some("S")
        );
        assert_eq!(pages.spans[0].start.number, Some(10));
        assert_eq!(pages.spans[0].end.as_ref().unwrap().number, Some(20));
    }

    #[test]
    fn parses_roman_range_as_literal_bounds() {
        let pages = Pages::parse("iv-xii").unwrap();
        assert_eq!(pages.spans.len(), 1);
        assert_eq!(pages.spans[0].start.literal.as_str(), "iv");
        assert_eq!(pages.spans[0].start.number, None);
        assert_eq!(pages.spans[0].end.as_ref().unwrap().literal.as_str(), "xii");
    }

    #[test]
    fn parses_discontinuous_list() {
        let pages = Pages::parse("1-3, 8-10").unwrap();
        assert_eq!(pages.spans.len(), 2);
        assert_eq!(pages.spans[0].start.number, Some(1));
        assert_eq!(pages.spans[0].end.as_ref().unwrap().number, Some(3));
        assert_eq!(pages.spans[1].start.number, Some(8));
        assert_eq!(pages.spans[1].end.as_ref().unwrap().number, Some(10));
    }

    #[test]
    fn unrecognized_label_keeps_literal_without_forcing_structure() {
        let pages = Pages::parse("passim").unwrap();
        assert_eq!(pages.as_str(), "passim");
        // Single non-numeric token is still one unstructured-looking span with
        // no number — acceptable structured form with lossless literal.
        assert_eq!(pages.spans.len(), 1);
        assert_eq!(pages.spans[0].start.number, None);
        assert!(pages.spans[0].end.is_none());
    }
}
