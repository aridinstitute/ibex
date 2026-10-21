//! Presentation of [`ResourceKind`]: a row icon, its color, and a
//! sentence-case label.

use std::borrow::Cow;

use gpui_kit::assets::{Assets, IconName, icon_assets};
use gpui_kit::{AssetSource, Hsla, Result, SharedString, rgb};
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

pub fn kind_symbol(kind: ResourceKind) -> gpui_kit::component::Icon {
    let name = match kind {
        ResourceKind::Book => "book.closed",
        ResourceKind::JournalArticle => "doc.text",
        ResourceKind::BookSection => "book",
        ResourceKind::ConferencePaper => "person.3",
        ResourceKind::Preprint => "doc.badge.clock",
        ResourceKind::Thesis => "graduationcap",
        ResourceKind::Report => "list.clipboard",
        ResourceKind::Webpage => "globe",
        ResourceKind::Dataset => "externaldrive",
        ResourceKind::Manuscript => "scroll",
        ResourceKind::MagazineArticle => "magazine",
        ResourceKind::NewspaperArticle => "newspaper",
        ResourceKind::ComputerProgram => "chevron.left.forwardslash.chevron.right",
        ResourceKind::DictionaryEntry => "character.book.closed",
        ResourceKind::EncyclopediaArticle => "books.vertical",
        ResourceKind::BlogPost => "square.and.pencil",
        ResourceKind::PodcastEpisode => "mic",
        ResourceKind::Journal => "text.book.closed",
        ResourceKind::Podcast => "dot.radiowaves.left.and.right",
        ResourceKind::Blog => "antenna.radiowaves.left.and.right",
    };
    crate::icons::symbol(name, kind_icon(kind))
}

/// Medium a kind belongs to, for the Kind column's icon color. Twenty hues
/// at 18px would be indistinguishable, so kinds of one medium share a hue;
/// the icon shape still separates them within a family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindFamily {
    /// Books and other print reference works.
    Print,
    /// Articles in journals, conference proceedings, magazines, and papers.
    Article,
    /// Standalone grey literature: reports and theses.
    Report,
    /// Web pages and blog posts.
    Web,
    /// Podcast episodes.
    Audio,
    /// Datasets and software.
    Data,
    /// Container kinds, the ones [`ResourceKind::is_container`] names: the
    /// journal, podcast, or blog an item appeared in.
    Serial,
}

/// Medium `kind` draws its icon color from.
pub fn kind_family(kind: ResourceKind) -> KindFamily {
    match kind {
        ResourceKind::Book
        | ResourceKind::BookSection
        | ResourceKind::DictionaryEntry
        | ResourceKind::EncyclopediaArticle
        | ResourceKind::Manuscript => KindFamily::Print,
        ResourceKind::JournalArticle
        | ResourceKind::ConferencePaper
        | ResourceKind::Preprint
        | ResourceKind::MagazineArticle
        | ResourceKind::NewspaperArticle => KindFamily::Article,
        ResourceKind::Report | ResourceKind::Thesis => KindFamily::Report,
        ResourceKind::Webpage | ResourceKind::BlogPost => KindFamily::Web,
        ResourceKind::PodcastEpisode => KindFamily::Audio,
        ResourceKind::Dataset | ResourceKind::ComputerProgram => KindFamily::Data,
        ResourceKind::Journal | ResourceKind::Podcast | ResourceKind::Blog => KindFamily::Serial,
    }
}

/// Row-icon color for `kind`, `dark` selecting the variant that reads on the
/// dark palette. Family hues sit at least 26° apart on the wheel and none
/// lands within 25° of the theme's accent blue, so a colored icon never
/// reads as the selection fill.
pub fn kind_color(kind: ResourceKind, dark: bool) -> Hsla {
    // Light variants are deep enough for 3:1 against the white table; dark
    // variants lift the same hue for the dark surfaces.
    let hex = match (kind_family(kind), dark) {
        (KindFamily::Print, false) => 0xd97706,
        (KindFamily::Print, true) => 0xfbbf24,
        (KindFamily::Article, false) => 0x4f46e5,
        (KindFamily::Article, true) => 0x818cf8,
        (KindFamily::Report, false) => 0xdc2626,
        (KindFamily::Report, true) => 0xf87171,
        (KindFamily::Web, false) => 0x0d9488,
        (KindFamily::Web, true) => 0x2dd4bf,
        (KindFamily::Audio, false) => 0x9333ea,
        (KindFamily::Audio, true) => 0xc084fc,
        (KindFamily::Data, false) => 0x16a34a,
        (KindFamily::Data, true) => 0x4ade80,
        (KindFamily::Serial, false) => 0xdb2777,
        (KindFamily::Serial, true) => 0xf472b6,
    };
    rgb(hex).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::Rgba;

    /// Every kind, so the family and color tables are checked exhaustively.
    const ALL_KINDS: [ResourceKind; 20] = [
        ResourceKind::Book,
        ResourceKind::JournalArticle,
        ResourceKind::BookSection,
        ResourceKind::ConferencePaper,
        ResourceKind::Preprint,
        ResourceKind::Thesis,
        ResourceKind::Report,
        ResourceKind::Webpage,
        ResourceKind::Dataset,
        ResourceKind::Manuscript,
        ResourceKind::MagazineArticle,
        ResourceKind::NewspaperArticle,
        ResourceKind::ComputerProgram,
        ResourceKind::DictionaryEntry,
        ResourceKind::EncyclopediaArticle,
        ResourceKind::BlogPost,
        ResourceKind::PodcastEpisode,
        ResourceKind::Journal,
        ResourceKind::Podcast,
        ResourceKind::Blog,
    ];

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

    #[test]
    fn each_family_paints_one_color_and_families_differ() {
        let mut by_family = Vec::new();
        for kind in ALL_KINDS {
            let color = kind_color(kind, false);
            match by_family
                .iter()
                .find(|(family, _)| *family == kind_family(kind))
            {
                Some((_, existing)) => assert_eq!(*existing, color),
                None => by_family.push((kind_family(kind), color)),
            }
        }
        assert_eq!(by_family.len(), 7, "every family is represented");
        for (i, (family, color)) in by_family.iter().enumerate() {
            for (other, other_color) in &by_family[i + 1..] {
                assert_ne!(color, other_color, "{family:?} and {other:?} share a color");
            }
        }
    }

    #[test]
    fn dark_variants_lift_the_same_hue() {
        for kind in ALL_KINDS {
            let light = kind_color(kind, false);
            let dark = kind_color(kind, true);
            assert!(dark.l > light.l, "{kind:?} must lighten in dark mode");
            assert!(
                (dark.h - light.h).abs() < 0.04,
                "{kind:?} must keep its hue across modes"
            );
        }
    }

    #[test]
    fn light_variants_clear_three_to_one_on_white() {
        for kind in ALL_KINDS {
            let Rgba { r, g, b, .. } = kind_color(kind, false).into();
            let linear = |channel: f32| {
                if channel <= 0.04045 {
                    channel / 12.92
                } else {
                    ((channel + 0.055) / 1.055).powf(2.4)
                }
            };
            let luminance = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
            let contrast = 1.05 / (luminance + 0.05);
            assert!(contrast >= 3., "{kind:?} contrast on white is {contrast}");
        }
    }

    #[test]
    fn family_hues_stay_far_enough_apart_to_name() {
        let hues = ALL_KINDS
            .into_iter()
            .map(|kind| kind_color(kind, false).h)
            .fold(Vec::new(), |mut hues: Vec<f32>, hue| {
                if !hues.contains(&hue) {
                    hues.push(hue);
                }
                hues
            });
        assert_eq!(hues.len(), 7, "one hue per family");
        // The closest pair sits 27° apart; keep a margin so a future tweak
        // cannot collapse two families into one color.
        for (i, hue) in hues.iter().enumerate() {
            for other in &hues[i + 1..] {
                let gap = (hue - other).abs();
                assert!(gap.min(1. - gap) >= 0.069, "{hue} and {other} nearly match");
            }
        }
    }

    #[test]
    fn no_family_hue_lands_on_the_theme_accent() {
        // Mirrors `theme::palette`'s light accent. A kind icon that close to
        // the selection fill would read as the accent, not as a kind.
        let accent: Hsla = rgb(0x0064ff).into();
        for kind in ALL_KINDS {
            let gap = (kind_color(kind, false).h - accent.h).abs();
            assert!(
                gap.min(1. - gap) >= 0.06,
                "{kind:?} sits on the accent blue"
            );
        }
    }

    #[test]
    fn every_container_kind_takes_the_serial_color() {
        // Tied to `is_container` rather than a hand-kept list, so a new
        // container kind cannot quietly land in another family.
        for kind in ALL_KINDS.into_iter().filter(|kind| kind.is_container()) {
            assert_eq!(kind_family(kind), KindFamily::Serial, "{kind:?}");
        }
    }

    #[test]
    fn container_kinds_do_not_share_their_contents_color() {
        for (container, content) in [
            (ResourceKind::Journal, ResourceKind::JournalArticle),
            (ResourceKind::Podcast, ResourceKind::PodcastEpisode),
            (ResourceKind::Blog, ResourceKind::BlogPost),
        ] {
            assert!(container.is_container(), "{container:?} is a container");
            assert_ne!(kind_color(container, false), kind_color(content, false));
        }
    }
}
