//! Document text for topic clustering.

use std::path::Path;

use ibex_core::{Attachment, ResourceDetail};

/// Where clustering text came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextSource {
    /// At least one attachment contributed extractable body text.
    Attachment,
    /// Title / subtitle / abstract / keywords only.
    Metadata,
}

/// Extracted document body plus provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentText {
    pub body: String,
    pub source: TextSource,
}

/// Soft cap so huge PDFs do not dominate clustering time or memory.
pub const MAX_DOCUMENT_CHARS: usize = 50_000;

const ATTACHMENT_TEXT_TYPES: &[&str] = &[
    "txt", "md", "markdown", "text", "html", "htm", "xml", "csv", "tex", "bib", "json", "rst",
];

/// Prefer attachment content; fall back to bibliographic fields.
pub fn extract_document_text(detail: &ResourceDetail) -> DocumentText {
    let mut attachment_parts = Vec::new();
    for attachment in &detail.attachments {
        if let Some(text) = extract_attachment_text(attachment) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                attachment_parts.push(trimmed.to_owned());
            }
        }
    }

    if !attachment_parts.is_empty() {
        let mut parts = Vec::new();
        push_field(&mut parts, detail.title.as_str());
        if let Some(subtitle) = &detail.subtitle {
            push_field(&mut parts, subtitle.as_str());
        }
        parts.extend(attachment_parts);
        return DocumentText {
            body: truncate(parts.join("\n\n")),
            source: TextSource::Attachment,
        };
    }

    DocumentText {
        body: truncate(metadata_text(detail)),
        source: TextSource::Metadata,
    }
}

/// Read text from a single attachment when the type is supported.
pub fn extract_attachment_text(attachment: &Attachment) -> Option<String> {
    extract_path_text(&attachment.file_path, &attachment.attachment_type)
}

fn extract_path_text(path: &Path, attachment_type: &str) -> Option<String> {
    let kind = attachment_type.to_ascii_lowercase();
    if kind == "pdf" {
        return pdf_extract::extract_text(path)
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
    }
    if ATTACHMENT_TEXT_TYPES.iter().any(|t| *t == kind) {
        return std::fs::read_to_string(path)
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
    }
    None
}

fn metadata_text(detail: &ResourceDetail) -> String {
    let mut parts = Vec::new();
    push_field(&mut parts, detail.title.as_str());
    if let Some(subtitle) = &detail.subtitle {
        push_field(&mut parts, subtitle.as_str());
    }
    if let Some(addon) = &detail.title_addon {
        push_field(&mut parts, addon.as_str());
    }
    if let Some(short) = &detail.short_title {
        push_field(&mut parts, short.as_str());
    }
    if let Some(abstract_text) = &detail.abstract_text {
        push_field(&mut parts, abstract_text);
    }
    for keyword in &detail.keywords {
        push_field(&mut parts, keyword);
    }
    parts.join("\n")
}

fn push_field(parts: &mut Vec<String>, value: &str) {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        parts.push(trimmed.to_owned());
    }
}

fn truncate(s: String) -> String {
    if s.chars().count() <= MAX_DOCUMENT_CHARS {
        return s;
    }
    s.chars().take(MAX_DOCUMENT_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ibex_core::{
        Attachment, AttachmentId, NonEmptyStr, ResourceDetail, ResourceId, ResourceKind, UtcMillis,
    };
    use std::path::PathBuf;

    fn detail_with(title: &str) -> ResourceDetail {
        ResourceDetail {
            id: ResourceId::generate(),
            kind: ResourceKind::JournalArticle,
            title: NonEmptyStr::try_new(title).unwrap(),
            creators: vec![],
            publisher: None,
            published: None,
            accessed: None,
            container_title: None,
            series_title: None,
            series_number: None,
            favorite: false,
            tags: vec![],
            collections: vec![],
            attachments: vec![],
            date_added: UtcMillis::from_unix_ms(0),
            date_edited: UtcMillis::from_unix_ms(0),
            subtitle: None,
            title_addon: None,
            short_title: None,
            place: None,
            volume: None,
            issue: None,
            pages: None,
            total_pages: None,
            article_number: None,
            edition: None,
            doi: None,
            url: None,
            eprint: None,
            isbns: vec![],
            issns: vec![],
            isrns: vec![],
            original_date: None,
            event_date: None,
            language: None,
            document_type: None,
            publication_method: None,
            institution: None,
            organization: None,
            school: None,
            abstract_text: None,
            keywords: vec![],
        }
    }

    #[test]
    fn metadata_fallback_joins_title_subtitle_abstract() {
        let mut detail = detail_with("Deep Learning");
        detail.subtitle = Some(NonEmptyStr::try_new("A survey").unwrap());
        detail.abstract_text = Some("Neural networks and representation learning.".into());
        detail.keywords = vec!["deep learning".into()];
        let text = extract_document_text(&detail);
        assert_eq!(text.source, TextSource::Metadata);
        assert!(text.body.contains("Deep Learning"));
        assert!(text.body.contains("A survey"));
        assert!(text.body.contains("Neural networks"));
        assert!(text.body.contains("deep learning"));
    }

    #[test]
    fn attachment_text_preferred_over_abstract() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(
            &path,
            "Attachment body about photosynthesis and chlorophyll.",
        )
        .unwrap();
        let mut detail = detail_with("Plant Paper");
        detail.abstract_text = Some("This abstract should be ignored.".into());
        detail.attachments = vec![Attachment {
            id: AttachmentId::generate(),
            attachment_type: "txt".into(),
            title: "notes".into(),
            file_path: path,
        }];
        let text = extract_document_text(&detail);
        assert_eq!(text.source, TextSource::Attachment);
        assert!(text.body.contains("photosynthesis"));
        assert!(text.body.contains("Plant Paper"));
        assert!(!text.body.contains("ignored"));
    }

    #[test]
    fn unsupported_attachment_falls_back_to_metadata() {
        let mut detail = detail_with("Binary Blob");
        detail.subtitle = Some(NonEmptyStr::try_new("Still useful").unwrap());
        detail.attachments = vec![Attachment {
            id: AttachmentId::generate(),
            attachment_type: "png".into(),
            title: "figure".into(),
            file_path: PathBuf::from("/tmp/does-not-need-to-exist.png"),
        }];
        let text = extract_document_text(&detail);
        assert_eq!(text.source, TextSource::Metadata);
        assert!(text.body.contains("Still useful"));
    }

    #[test]
    fn truncates_oversized_attachment_bodies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.txt");
        let huge = "word ".repeat(MAX_DOCUMENT_CHARS);
        std::fs::write(&path, &huge).unwrap();
        let mut detail = detail_with("Huge");
        detail.attachments = vec![Attachment {
            id: AttachmentId::generate(),
            attachment_type: "txt".into(),
            title: "huge".into(),
            file_path: path,
        }];
        let text = extract_document_text(&detail);
        assert_eq!(text.source, TextSource::Attachment);
        assert!(text.body.chars().count() <= MAX_DOCUMENT_CHARS);
    }
}
