//! Edit panel layout: which rows the form shows, in what order and section,
//! and what each row is called. Window-free, so the registry runs under plain
//! unit tests; `LibraryApp` renders whatever this module lists.

use ibex_core::ResourceKind;

use crate::edit::BibliographicField;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    Core,
    Container,
    Publication,
    Identifiers,
    Content,
    Organize,
}

impl Section {
    pub const ALL: [Self; 6] = [
        Self::Core,
        Self::Container,
        Self::Publication,
        Self::Identifiers,
        Self::Content,
        Self::Organize,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Core => "Details",
            Self::Container => "Container",
            Self::Publication => "Publication",
            Self::Identifiers => "Identifiers & links",
            Self::Content => "Content",
            Self::Organize => "Organize",
        }
    }
}

/// One row of the edit panel. Rows that are not a single bibliographic
/// input (kind, creators, parent, series, the identifier group, tags and
/// collections) get their own variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FormField {
    Kind,
    Title,
    Creators,
    Parent,
    Series,
    Identifiers,
    Tags,
    Collections,
    Bib(BibliographicField),
}

impl FormField {
    /// Panel order. Section membership is fixed; sections appear in
    /// `Section` order along this list.
    pub const ORDER: &[Self] = {
        use BibliographicField as F;
        &[
            // Core
            Self::Kind,
            Self::Title,
            Self::Bib(F::Subtitle),
            Self::Bib(F::TitleAddon),
            Self::Bib(F::ShortTitle),
            Self::Creators,
            // Container
            Self::Parent,
            Self::Bib(F::Venue),
            Self::Bib(F::Volume),
            Self::Bib(F::Issue),
            Self::Bib(F::Pages),
            Self::Bib(F::ArticleNumber),
            Self::Bib(F::TotalPages),
            // Publication
            Self::Bib(F::Publisher),
            Self::Bib(F::Place),
            Self::Bib(F::Published),
            Self::Bib(F::OriginalDate),
            Self::Bib(F::EventDate),
            Self::Bib(F::Edition),
            Self::Series,
            Self::Bib(F::SeriesNumber),
            Self::Bib(F::Institution),
            Self::Bib(F::Organization),
            Self::Bib(F::DocumentType),
            Self::Bib(F::Language),
            Self::Bib(F::PublicationMethod),
            // Identifiers & links
            Self::Identifiers,
            Self::Bib(F::Url),
            Self::Bib(F::Accessed),
            Self::Bib(F::Citekey),
            // Content
            Self::Bib(F::AbstractText),
            Self::Bib(F::Keywords),
            // Organize
            Self::Tags,
            Self::Collections,
        ]
    };

    pub fn section(self) -> Section {
        use BibliographicField as F;
        match self {
            Self::Kind
            | Self::Title
            | Self::Creators
            | Self::Bib(F::Subtitle | F::TitleAddon | F::ShortTitle) => Section::Core,
            Self::Parent
            | Self::Bib(
                F::Venue | F::Volume | F::Issue | F::Pages | F::ArticleNumber | F::TotalPages,
            ) => Section::Container,
            Self::Series
            | Self::Bib(
                F::Publisher
                | F::Place
                | F::Published
                | F::OriginalDate
                | F::EventDate
                | F::Edition
                | F::SeriesNumber
                | F::Institution
                | F::Organization
                | F::DocumentType
                | F::Language
                | F::PublicationMethod,
            ) => Section::Publication,
            Self::Identifiers | Self::Bib(F::Url | F::Accessed | F::Citekey) => {
                Section::Identifiers
            }
            Self::Bib(F::AbstractText | F::Keywords) => Section::Content,
            Self::Tags | Self::Collections => Section::Organize,
            // Identifier and display fields live inside another row.
            Self::Bib(
                field @ (F::Doi
                | F::Eprint
                | F::Isbns
                | F::Issns
                | F::Isrns
                | F::PublishedDisplay
                | F::AccessedDisplay
                | F::OriginalDateDisplay
                | F::EventDateDisplay),
            ) => Self::for_bib(field).section(),
        }
    }

    pub fn label(self, kind: ResourceKind) -> &'static str {
        use BibliographicField as F;
        use ResourceKind as K;
        match (self, kind) {
            (Self::Kind, _) => "Kind",
            (Self::Title, _) => "Title",
            (Self::Creators, _) => "Creators",
            (Self::Parent, _) => "Parent",
            (Self::Series, _) => "Series",
            (Self::Identifiers, _) => "Identifiers",
            (Self::Tags, _) => "Tags",
            (Self::Collections, _) => "Collections",
            (Self::Bib(F::Venue), K::Webpage) => "Website",
            (Self::Bib(F::Venue), K::ConferencePaper) => "Proceedings",
            (Self::Bib(F::Venue), _) => "Published in",
            (Self::Bib(F::Edition), K::Dataset | K::ComputerProgram) => "Version",
            (Self::Bib(F::Institution), K::Thesis) => "University",
            (Self::Bib(F::Institution), K::Manuscript) => "Archive",
            (Self::Bib(field), _) => field.label(),
        }
    }
    /// Payload field to the row that edits it.
    pub fn for_bib(field: BibliographicField) -> Self {
        use BibliographicField as F;
        match field {
            F::Doi | F::Eprint | F::Isbns | F::Issns | F::Isrns => Self::Identifiers,
            F::PublishedDisplay => Self::Bib(F::Published),
            F::AccessedDisplay => Self::Bib(F::Accessed),
            F::OriginalDateDisplay => Self::Bib(F::OriginalDate),
            F::EventDateDisplay => Self::Bib(F::EventDate),
            field => Self::Bib(field),
        }
    }
}

/// Rows `kind` shows even when empty; the rest wait behind "Add field…".
pub fn is_main(kind: ResourceKind, field: FormField) -> bool {
    use BibliographicField as F;
    use FormField as R;
    use ResourceKind as K;
    if matches!(
        field,
        R::Kind
            | R::Title
            | R::Creators
            | R::Bib(F::Published)
            | R::Bib(F::AbstractText)
            | R::Tags
            | R::Collections
    ) {
        return true;
    }
    let extra: &[FormField] = match kind {
        K::Book => &[
            R::Bib(F::Subtitle),
            R::Bib(F::Publisher),
            R::Bib(F::Place),
            R::Bib(F::Edition),
            R::Series,
            R::Bib(F::SeriesNumber),
            R::Identifiers,
        ],
        K::BookSection | K::DictionaryEntry | K::EncyclopediaArticle => {
            &[R::Parent, R::Bib(F::Pages), R::Identifiers]
        }
        K::JournalArticle => &[
            R::Parent,
            R::Bib(F::Venue),
            R::Bib(F::Volume),
            R::Bib(F::Issue),
            R::Bib(F::Pages),
            R::Identifiers,
        ],
        K::MagazineArticle | K::NewspaperArticle => {
            &[R::Bib(F::Venue), R::Bib(F::Pages), R::Bib(F::Url)]
        }
        K::ConferencePaper => &[
            R::Bib(F::Venue),
            R::Bib(F::EventDate),
            R::Bib(F::Place),
            R::Bib(F::Pages),
            R::Identifiers,
        ],
        K::Preprint => &[R::Identifiers, R::Bib(F::Url)],
        K::Thesis => &[
            R::Bib(F::Institution),
            R::Bib(F::DocumentType),
            R::Bib(F::Place),
            R::Bib(F::Url),
        ],
        K::Report => &[
            R::Bib(F::Institution),
            R::Bib(F::DocumentType),
            R::Bib(F::Place),
            R::Identifiers,
            R::Bib(F::Url),
        ],
        K::Webpage => &[R::Bib(F::Venue), R::Bib(F::Url), R::Bib(F::Accessed)],
        K::BlogPost | K::PodcastEpisode => &[R::Parent, R::Bib(F::Url), R::Bib(F::Accessed)],
        K::Dataset => &[
            R::Bib(F::Publisher),
            R::Bib(F::Edition),
            R::Identifiers,
            R::Bib(F::Url),
            R::Bib(F::Accessed),
        ],
        K::ComputerProgram => &[
            R::Bib(F::Publisher),
            R::Bib(F::Edition),
            R::Bib(F::Url),
            R::Identifiers,
        ],
        K::Manuscript => &[
            R::Bib(F::Place),
            R::Bib(F::DocumentType),
            R::Bib(F::Institution),
            R::Bib(F::OriginalDate),
        ],
        K::Journal => &[
            R::Bib(F::Publisher),
            R::Bib(F::Place),
            R::Identifiers,
            R::Bib(F::Url),
        ],
        K::Issue => &[R::Parent, R::Bib(F::Volume), R::Bib(F::Issue)],
        K::Podcast | K::Blog => &[R::Bib(F::Publisher), R::Bib(F::Url)],
    };
    extra.contains(&field)
}

/// Whether the panel shows `field` for `kind`. A linked parent's title
/// overrides the venue (`resolve_container_title`), so an empty venue
/// stays hidden while a parent is set, even when revealed.
pub fn visible(
    kind: ResourceKind,
    field: FormField,
    has_value: bool,
    revealed: bool,
    parent_set: bool,
) -> bool {
    if has_value {
        return true;
    }
    if field == FormField::Bib(BibliographicField::Venue) && parent_set {
        return false;
    }
    is_main(kind, field) || revealed
}

#[cfg(test)]
mod tests {
    use super::{FormField, Section, is_main, visible};
    use crate::edit::BibliographicField;
    use ibex_core::ResourceKind;
    use std::collections::BTreeSet;

    #[test]
    fn every_kind_shows_kind_title_and_creators() {
        for kind in ResourceKind::ALL {
            for field in [FormField::Kind, FormField::Title, FormField::Creators] {
                assert!(is_main(kind, field), "{kind:?} {field:?}");
                assert!(visible(kind, field, false, false, true));
            }
        }
    }

    #[test]
    fn empty_venue_hides_while_a_parent_is_set() {
        let venue = FormField::Bib(BibliographicField::Venue);
        let kind = ResourceKind::JournalArticle;
        assert!(visible(kind, venue, false, false, false));
        assert!(!visible(kind, venue, false, false, true));
        assert!(!visible(kind, venue, false, true, true));
        assert!(visible(kind, venue, true, false, true));
    }

    #[test]
    fn a_field_with_a_value_is_always_visible() {
        for kind in ResourceKind::ALL {
            for field in FormField::ORDER {
                assert!(
                    visible(kind, *field, true, false, true),
                    "{kind:?} {field:?}"
                );
                assert!(
                    visible(kind, *field, true, false, false),
                    "{kind:?} {field:?}"
                );
            }
        }
        let volume = FormField::Bib(BibliographicField::Volume);
        assert!(!visible(ResourceKind::Book, volume, false, false, false));
        assert!(visible(ResourceKind::Book, volume, false, true, false));
    }

    #[test]
    fn labels_follow_the_kind() {
        use BibliographicField as F;
        let venue = FormField::Bib(F::Venue);
        assert_eq!(venue.label(ResourceKind::Webpage), "Website");
        assert_eq!(venue.label(ResourceKind::ConferencePaper), "Proceedings");
        assert_eq!(venue.label(ResourceKind::JournalArticle), "Published in");
        let edition = FormField::Bib(F::Edition);
        assert_eq!(edition.label(ResourceKind::Dataset), "Version");
        assert_eq!(edition.label(ResourceKind::Book), F::Edition.label());
        let institution = FormField::Bib(F::Institution);
        assert_eq!(institution.label(ResourceKind::Thesis), "University");
        assert_eq!(institution.label(ResourceKind::Manuscript), "Archive");
    }

    #[test]
    fn order_has_no_duplicates() {
        let unique: BTreeSet<_> = FormField::ORDER.iter().collect();
        assert_eq!(unique.len(), FormField::ORDER.len());
    }

    #[test]
    fn every_bibliographic_field_maps_to_a_listed_row() {
        for field in BibliographicField::ALL {
            let row = FormField::for_bib(*field);
            assert!(FormField::ORDER.contains(&row), "{field:?} -> {row:?}");
        }
        use BibliographicField as F;
        for hidden in [
            F::Doi,
            F::Eprint,
            F::Isbns,
            F::Issns,
            F::Isrns,
            F::PublishedDisplay,
            F::AccessedDisplay,
            F::OriginalDateDisplay,
            F::EventDateDisplay,
        ] {
            assert!(!FormField::ORDER.contains(&FormField::Bib(hidden)));
        }
    }

    #[test]
    fn sections_follow_section_order_along_the_panel() {
        let sections: Vec<Section> = FormField::ORDER.iter().map(|f| f.section()).collect();
        assert!(sections.windows(2).all(|pair| pair[0] <= pair[1]));
        for section in Section::ALL {
            assert!(sections.contains(&section), "{section:?} has rows");
        }
        assert_eq!(FormField::ORDER[0], FormField::Kind);
    }
}
