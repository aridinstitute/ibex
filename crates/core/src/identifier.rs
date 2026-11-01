//! Bibliographic identifiers retaining original spelling plus optional canonical form.

/// One identifier as supplied by the source, with an optional lookup key.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Identifier {
    pub original: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical: Option<String>,
}

impl<'de> serde::Deserialize<'de> for Identifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Literal(String),
            Structured {
                original: String,
                #[serde(default)]
                canonical: Option<String>,
            },
        }
        match Wire::deserialize(deserializer)? {
            Wire::Literal(s) => Ok(Self::new(s)),
            Wire::Structured {
                original,
                canonical,
            } => Ok(Self {
                original,
                canonical,
            }),
        }
    }
}

impl Identifier {
    pub fn new(original: impl Into<String>) -> Self {
        let original = original.into();
        let canonical = canonicalize(&original);
        Self {
            original,
            canonical,
        }
    }

    /// Retain spelling only; do not invent a canonical form.
    pub fn literal(original: impl Into<String>) -> Self {
        Self {
            original: original.into(),
            canonical: None,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.original
    }

    pub fn lookup_key(&self) -> &str {
        self.canonical.as_deref().unwrap_or(&self.original)
    }
}

impl From<&str> for Identifier {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Identifier {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl AsRef<str> for Identifier {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for Identifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Accept a plain string (legacy) or a structured `{original, canonical}` object.
pub fn deserialize_identifiers<'de, D>(deserializer: D) -> Result<Vec<Identifier>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum WireItem {
        Literal(String),
        Structured(Identifier),
    }
    let items = Vec::<WireItem>::deserialize(deserializer)?;
    Ok(items
        .into_iter()
        .map(|item| match item {
            WireItem::Literal(s) => Identifier::new(s),
            WireItem::Structured(id) => id,
        })
        .collect())
}

fn canonicalize(original: &str) -> Option<String> {
    let trimmed = original.trim();
    if trimmed.is_empty() {
        return None;
    }
    // ISBN/ISSN-like: strip separators and case-fold the check digit's X.
    let compact: String = trimmed
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if compact.is_empty() {
        None
    } else {
        Some(compact)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_original_spelling_and_adds_canonical() {
        let id = Identifier::new("978-0-306-40615-7");
        assert_eq!(id.as_str(), "978-0-306-40615-7");
        assert_eq!(id.canonical.as_deref(), Some("9780306406157"));
    }

    #[test]
    fn multiple_identifiers_keep_distinct_originals() {
        let ids: Vec<Identifier> = ["978-0-306-40615-7", "0-306-40615-2"]
            .into_iter()
            .map(Identifier::from)
            .collect();
        assert_eq!(ids[0].as_str(), "978-0-306-40615-7");
        assert_eq!(ids[1].as_str(), "0-306-40615-2");
        assert_ne!(ids[0].lookup_key(), ids[1].lookup_key());
    }
}
