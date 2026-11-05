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

    pub fn label(self, _kind: ResourceKind) -> &'static str {
        match self {
            Self::Kind => "Kind",
            Self::Title => "Title",
            Self::Creators => "Creators",
            Self::Parent => "Parent",
            Self::Series => "Series",
            Self::Identifiers => "Identifiers",
            Self::Tags => "Tags",
            Self::Collections => "Collections",
            Self::Bib(field) => field.label(),
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

#[cfg(test)]
mod tests {
    use super::{FormField, Section};
    use crate::edit::BibliographicField;
    use std::collections::BTreeSet;

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
