//! Recommend-only classification of resources into collections.
//!
//! Matching never mutates anything: shells show the suggestions and the user
//! decides whether to accept them.

use std::collections::HashSet;

use crate::{Collection, CollectionId, ResourceDetail};

/// Longest accepted keyword rule, in characters.
pub const KEYWORD_MAX_CHARS: usize = 128;
/// Most keyword rules one collection may carry.
pub const KEYWORDS_MAX_PER_COLLECTION: usize = 64;

/// The text of a resource that keyword rules are matched against.
#[derive(Debug, Clone, Copy)]
pub struct ClassifyInput<'a> {
    pub title: &'a str,
    pub abstract_text: Option<&'a str>,
    pub keywords: &'a [String],
}

impl<'a> From<&'a ResourceDetail> for ClassifyInput<'a> {
    fn from(d: &'a ResourceDetail) -> Self {
        Self {
            title: d.title.as_str(),
            abstract_text: d.abstract_text.as_deref(),
            keywords: &d.keywords,
        }
    }
}

/// Lowercased alphanumeric words; everything else is a boundary.
fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn contains_phrase(haystack: &[String], phrase: &[String]) -> bool {
    !phrase.is_empty()
        && haystack.len() >= phrase.len()
        && haystack.windows(phrase.len()).any(|w| w == phrase)
}

/// Collections whose keyword rules match `input`, deduped, in `collections`
/// order.
///
/// A rule matches when its words appear contiguously, compared
/// case-insensitively on word boundaries ("art" does not match "earth").
/// Title, abstract, and each source keyword are matched separately, so a
/// phrase never spans two fields.
pub fn keyword_suggestions<'a>(
    input: impl Into<ClassifyInput<'a>>,
    collections: &[Collection],
) -> Vec<CollectionId> {
    let input = input.into();
    let mut fields = vec![words(input.title)];
    if let Some(a) = input.abstract_text {
        fields.push(words(a));
    }
    fields.extend(input.keywords.iter().map(|k| words(k)));

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for collection in collections {
        if seen.contains(&collection.id) {
            continue;
        }
        let hit = collection.keywords.iter().any(|rule| {
            let phrase = words(rule);
            fields.iter().any(|f| contains_phrase(f, &phrase))
        });
        if hit {
            seen.insert(collection.id);
            out.push(collection.id);
        }
    }
    out
}

/// Parse a comma-separated keyword rule list.
///
/// Entries are trimmed; blank input yields no rules. Duplicates are dropped
/// case-insensitively, keeping the first spelling.
pub fn parse_keywords(raw: &str) -> Result<Vec<String>, String> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for part in raw.split(',') {
        let kw = part.trim();
        if kw.is_empty() {
            return Err("keywords cannot be empty".into());
        }
        if kw.chars().count() > KEYWORD_MAX_CHARS {
            return Err(format!(
                "keywords must be at most {KEYWORD_MAX_CHARS} characters"
            ));
        }
        if !kw.chars().any(char::is_alphanumeric) {
            return Err(format!("keyword \"{kw}\" has no letters or digits"));
        }
        if seen.insert(kw.to_lowercase()) {
            out.push(kw.to_string());
        }
    }
    if out.len() > KEYWORDS_MAX_PER_COLLECTION {
        return Err(format!(
            "a collection can have at most {KEYWORDS_MAX_PER_COLLECTION} keywords"
        ));
    }
    Ok(out)
}

/// Validate an already-split keyword list the same way as [`parse_keywords`].
pub fn normalize_keywords(keywords: &[String]) -> Result<Vec<String>, String> {
    if keywords.is_empty() {
        return Ok(Vec::new());
    }
    if keywords.iter().any(|k| k.trim().is_empty()) {
        return Err("keywords cannot be empty".into());
    }
    if keywords.iter().any(|k| k.contains(',')) {
        return Err("keywords cannot contain commas".into());
    }
    parse_keywords(&keywords.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NonEmptyStr;

    fn coll(name: &str, keywords: &[&str]) -> Collection {
        let mut c = Collection::try_new(
            CollectionId::generate(),
            NonEmptyStr::try_new(name).unwrap(),
        );
        c.keywords = keywords.iter().map(|k| k.to_string()).collect();
        c
    }

    fn input<'a>(title: &'a str, abs: Option<&'a str>, kws: &'a [String]) -> ClassifyInput<'a> {
        ClassifyInput {
            title,
            abstract_text: abs,
            keywords: kws,
        }
    }

    #[test]
    fn matches_case_insensitively_on_word_boundaries() {
        let art = coll("Art", &["art"]);
        let none: Vec<String> = vec![];
        assert!(
            keyword_suggestions(
                input("On the Earth", None, &none),
                std::slice::from_ref(&art)
            )
            .is_empty()
        );
        assert_eq!(
            keyword_suggestions(
                input("Modern ART, revisited", None, &none),
                std::slice::from_ref(&art)
            ),
            vec![art.id]
        );
    }

    #[test]
    fn phrases_match_contiguously_and_within_one_field() {
        let methods = coll("Methods", &["research methods"]);
        let cs = [methods.clone()];
        let none: Vec<String> = vec![];
        assert_eq!(
            keyword_suggestions(
                input("Qualitative research-methods today", None, &none),
                &cs
            ),
            vec![methods.id]
        );
        assert!(keyword_suggestions(input("research and methods", None, &none), &cs).is_empty());
        let kws = vec!["research".to_string(), "methods".to_string()];
        assert!(keyword_suggestions(input("x", None, &kws), &cs).is_empty());
    }

    #[test]
    fn searches_abstract_and_source_keywords() {
        let a = coll("Bibliometrics", &["bibliometrics"]);
        let b = coll("Provenance", &["provenance"]);
        let cs = [a.clone(), b.clone()];
        let kws = vec!["Provenance".to_string()];
        assert_eq!(
            keyword_suggestions(input("x", Some("A bibliometrics study."), &kws), &cs),
            vec![a.id, b.id]
        );
    }

    #[test]
    fn dedupes_and_keeps_input_order() {
        let a = coll("A", &["alpha", "beta"]);
        let b = coll("B", &["beta"]);
        let cs = [b.clone(), a.clone(), a.clone()];
        let none: Vec<String> = vec![];
        assert_eq!(
            keyword_suggestions(input("alpha beta", None, &none), &cs),
            vec![b.id, a.id]
        );
    }

    #[test]
    fn collections_without_rules_never_match() {
        let a = coll("A", &[]);
        let none: Vec<String> = vec![];
        assert!(keyword_suggestions(input("A", None, &none), &[a]).is_empty());
    }

    #[test]
    fn parse_keywords_trims_and_dedupes() {
        assert_eq!(
            parse_keywords(" Provenance , research methods, provenance ").unwrap(),
            vec!["Provenance".to_string(), "research methods".to_string()]
        );
        assert_eq!(parse_keywords("   ").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn parse_keywords_rejects_bad_entries() {
        assert!(parse_keywords("a,,b").is_err());
        assert!(parse_keywords("a, ").is_err());
        assert!(parse_keywords("--").is_err());
        assert!(parse_keywords(&"x".repeat(KEYWORD_MAX_CHARS + 1)).is_err());
        assert!(parse_keywords(&"x".repeat(KEYWORD_MAX_CHARS)).is_ok());
        let many: Vec<String> = (0..=KEYWORDS_MAX_PER_COLLECTION)
            .map(|i| format!("k{i}"))
            .collect();
        assert!(parse_keywords(&many.join(",")).is_err());
    }

    #[test]
    fn normalize_keywords_validates_lists() {
        assert_eq!(
            normalize_keywords(&["a".into(), " A ".into(), "b".into()]).unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(normalize_keywords(&["a,b".into()]).is_err());
        assert!(normalize_keywords(&["".into()]).is_err());
    }
}
