//! Document text for topic clustering.

use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};
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
/// Byte cap when reading attachments (before UTF-8 decode / truncate).
pub const MAX_ATTACHMENT_BYTES: u64 = 2 * 1024 * 1024;

const PLAIN_TEXT_TYPES: &[&str] = &["txt", "md", "markdown", "text", "tex", "bib", "rst"];
const MARKUP_TEXT_TYPES: &[&str] = &["html", "htm"];

/// Prefer attachment content (plus title/subtitle/abstract/keywords);
/// fall back to bibliographic fields alone.
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
        push_metadata_fields(&mut parts, detail);
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
///
/// PDF extraction panics are caught and treated as unreadable.
pub fn extract_attachment_text(attachment: &Attachment) -> Option<String> {
    extract_path_text(&attachment.file_path, &attachment.attachment_type)
}

fn extract_path_text(path: &Path, attachment_type: &str) -> Option<String> {
    let kind = attachment_type.to_ascii_lowercase();
    if kind == "pdf" {
        return extract_pdf_text(path);
    }
    if PLAIN_TEXT_TYPES.iter().any(|t| *t == kind) {
        return read_capped_string(path).filter(|s| !s.trim().is_empty());
    }
    if MARKUP_TEXT_TYPES.iter().any(|t| *t == kind) {
        return read_capped_string(path)
            .map(|s| strip_tags(&s))
            .filter(|s| !s.trim().is_empty());
    }
    None
}

fn extract_pdf_text(path: &Path) -> Option<String> {
    let path = path.to_path_buf();
    let result = catch_unwind(AssertUnwindSafe(|| pdf_extract::extract_text(&path)));
    match result {
        Ok(Ok(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        }
        Ok(Err(_)) | Err(_) => None,
    }
}

fn read_capped_string(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    let mut limited = (&mut file).take(MAX_ATTACHMENT_BYTES);
    limited.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn metadata_text(detail: &ResourceDetail) -> String {
    let mut parts = Vec::new();
    push_metadata_fields(&mut parts, detail);
    parts.join("\n")
}

fn push_metadata_fields(parts: &mut Vec<String>, detail: &ResourceDetail) {
    push_field(parts, detail.title.as_str());
    if let Some(subtitle) = &detail.subtitle {
        push_field(parts, subtitle.as_str());
    }
    if let Some(addon) = &detail.title_addon {
        push_field(parts, addon.as_str());
    }
    if let Some(short) = &detail.short_title {
        push_field(parts, short.as_str());
    }
    if let Some(abstract_text) = &detail.abstract_text {
        push_field(parts, abstract_text);
    }
    for keyword in &detail.keywords {
        push_field(parts, keyword);
    }
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
    fn attachment_keeps_metadata_fields_alongside_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(
            &path,
            "Attachment body about photosynthesis and chlorophyll.",
        )
        .unwrap();
        let mut detail = detail_with("Plant Paper");
        detail.abstract_text = Some("Keep this abstract for clustering.".into());
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
        assert!(text.body.contains("Keep this abstract"));
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

    #[test]
    fn strips_html_tags_from_markup_attachments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.html");
        std::fs::write(
            &path,
            "<html><body><div class=\"x\">climate carbon emissions</div></body></html>",
        )
        .unwrap();
        let text = extract_path_text(&path, "html").unwrap();
        assert!(text.contains("climate"));
        assert!(!text.contains("div"));
        assert!(!text.contains("class"));
    }

    #[test]
    fn pdf_extract_panic_is_swallowed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.pdf");
        // Not a valid PDF; extractors may error or panic — either must be None.
        std::fs::write(&path, b"%PDF-not-really\njunk").unwrap();
        assert!(extract_path_text(&path, "pdf").is_none());
    }

    #[test]
    fn byte_cap_avoids_loading_entire_huge_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        // Write more than the byte cap; reader must still succeed with a prefix.
        let chunk = vec![b'a'; 64 * 1024];
        let writes = (MAX_ATTACHMENT_BYTES as usize / chunk.len()) + 4;
        for _ in 0..writes {
            use std::io::Write;
            f.write_all(&chunk).unwrap();
        }
        drop(f);
        let text = read_capped_string(&path).unwrap();
        assert!(text.len() as u64 <= MAX_ATTACHMENT_BYTES);
    }
}
