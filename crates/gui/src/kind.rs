//! Presentation of [`ResourceKind`]: a row icon and a sentence-case label.

use std::borrow::Cow;

use gpui_kit::assets::{Assets, IconName, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};
use oeuvre_core::ResourceKind;

// Every icon `kind_icon` returns must be listed here, or it renders blank:
// only the component defaults are embedded otherwise.
icon_assets!(
    KindIcons,
    [
        Book,
        BookA,
        BookOpen,
        BookOpenText,
        ClipboardList,
        CodeXml,
        Database,
        FilePen,
        FileText,
        Globe,
        GraduationCap,
        Library,
        Mic,
        Newspaper,
        NotebookText,
        PenLine,
        Presentation,
        Radio,
        Rss,
        ScrollText
    ]
);

/// Kind icons layered over the component library's default icons.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = KindIcons.load(path)? {
            return Ok(Some(bytes));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(KindIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// Sentence-case label for a kind: `journalArticle` → `Journal article`.
pub fn kind_label(kind: ResourceKind) -> String {
    let mut out = String::new();
    for (i, c) in kind.as_str().chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_ascii_uppercase() {
            out.push(' ');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

pub fn kind_icon(kind: ResourceKind) -> IconName {
    match kind {
        ResourceKind::Book => IconName::Book,
        ResourceKind::JournalArticle => IconName::FileText,
        ResourceKind::BookSection => IconName::BookOpen,
        ResourceKind::ConferencePaper => IconName::Presentation,
        ResourceKind::Preprint => IconName::FilePen,
        ResourceKind::Thesis => IconName::GraduationCap,
        ResourceKind::Report => IconName::ClipboardList,
        ResourceKind::Webpage => IconName::Globe,
        ResourceKind::Dataset => IconName::Database,
        ResourceKind::Manuscript => IconName::ScrollText,
        ResourceKind::MagazineArticle => IconName::BookOpenText,
        ResourceKind::NewspaperArticle => IconName::Newspaper,
        ResourceKind::ComputerProgram => IconName::CodeXml,
        ResourceKind::DictionaryEntry => IconName::BookA,
        ResourceKind::EncyclopediaArticle => IconName::Library,
        ResourceKind::BlogPost => IconName::PenLine,
        ResourceKind::PodcastEpisode => IconName::Mic,
        ResourceKind::Journal => IconName::NotebookText,
        ResourceKind::Podcast => IconName::Radio,
        ResourceKind::Blog => IconName::Rss,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_sentence_case() {
        assert_eq!(kind_label(ResourceKind::Book), "Book");
        assert_eq!(kind_label(ResourceKind::JournalArticle), "Journal article");
        assert_eq!(kind_label(ResourceKind::ComputerProgram), "Computer program");
        assert_eq!(kind_label(ResourceKind::PodcastEpisode), "Podcast episode");
    }

    #[test]
    fn related_kinds_get_distinct_icons() {
        assert_ne!(kind_icon(ResourceKind::Podcast), kind_icon(ResourceKind::PodcastEpisode));
        assert_ne!(kind_icon(ResourceKind::Book), kind_icon(ResourceKind::BookSection));
    }
}
