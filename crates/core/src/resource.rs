use std::fmt;

use crate::{NonEmptyStr, PersonId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ResourceId(ulid::Ulid);

impl ResourceId {
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

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceKind {
    Book,
    JournalArticle,
    BookSection,
    ConferencePaper,
    Preprint,
    Thesis,
    Report,
    Webpage,
    Dataset,
    Manuscript,
    MagazineArticle,
    NewspaperArticle,
    ComputerProgram,
    DictionaryEntry,
    EncyclopediaArticle,
    BlogPost,
    PodcastEpisode,
    Journal,
    Podcast,
    Blog,
}

/// Cardinality rule for `parent_id`. `expected` is the container kind for a later existence check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentRule {
    Forbidden,
    Optional { expected: ResourceKind },
    Required { expected: ResourceKind },
}

impl ResourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Book => "book",
            Self::JournalArticle => "journalArticle",
            Self::BookSection => "bookSection",
            Self::ConferencePaper => "conferencePaper",
            Self::Preprint => "preprint",
            Self::Thesis => "thesis",
            Self::Report => "report",
            Self::Webpage => "webpage",
            Self::Dataset => "dataset",
            Self::Manuscript => "manuscript",
            Self::MagazineArticle => "magazineArticle",
            Self::NewspaperArticle => "newspaperArticle",
            Self::ComputerProgram => "computerProgram",
            Self::DictionaryEntry => "dictionaryEntry",
            Self::EncyclopediaArticle => "encyclopediaArticle",
            Self::BlogPost => "blogPost",
            Self::PodcastEpisode => "podcastEpisode",
            Self::Journal => "journal",
            Self::Podcast => "podcast",
            Self::Blog => "blog",
        }
    }

    pub fn is_container(self) -> bool {
        matches!(self, Self::Journal | Self::Podcast | Self::Blog)
    }

    pub fn parent_rule(self) -> ParentRule {
        match self {
            Self::PodcastEpisode => ParentRule::Required {
                expected: Self::Podcast,
            },
            Self::BlogPost => ParentRule::Optional {
                expected: Self::Blog,
            },
            Self::JournalArticle => ParentRule::Optional {
                expected: Self::Journal,
            },
            Self::BookSection | Self::DictionaryEntry | Self::EncyclopediaArticle => {
                ParentRule::Optional {
                    expected: Self::Book,
                }
            }
            Self::Journal
            | Self::Podcast
            | Self::Blog
            | Self::Book
            | Self::ConferencePaper
            | Self::Preprint
            | Self::Thesis
            | Self::Report
            | Self::Webpage
            | Self::Dataset
            | Self::Manuscript
            | Self::MagazineArticle
            | Self::NewspaperArticle
            | Self::ComputerProgram => ParentRule::Forbidden,
        }
    }

    pub fn accept_parent(self, parent_kind: ResourceKind) -> Result<(), ResourceError> {
        match self.parent_rule() {
            ParentRule::Forbidden => Err(ResourceError::ParentKindMismatch),
            ParentRule::Optional { expected } | ParentRule::Required { expected } => {
                if parent_kind == expected {
                    Ok(())
                } else {
                    Err(ResourceError::ParentKindMismatch)
                }
            }
        }
    }
}

/// Unix UTC millis. Shells parse calendar strings into this at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct UtcMillis(i64);

impl UtcMillis {
    pub fn from_unix_ms(ms: i64) -> Self {
        Self(ms)
    }

    pub fn as_unix_ms(self) -> i64 {
        self.0
    }
}

/// Stored instant plus optional fuzzy display ("2020", "Summer 397").
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BibliographicDate {
    instant: UtcMillis,
    display: Option<NonEmptyStr>,
}

impl BibliographicDate {
    pub fn try_new(
        instant: UtcMillis,
        display: Option<NonEmptyStr>,
    ) -> Result<Self, ResourceError> {
        Ok(Self { instant, display })
    }

    pub fn instant(&self) -> UtcMillis {
        self.instant
    }

    pub fn display(&self) -> Option<&str> {
        self.display.as_ref().map(NonEmptyStr::as_str)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CreatorRole {
    Author,
    Editor,
    Translator,
    Contributor,
    Host,
    Director,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Creator {
    pub person_id: PersonId,
    pub role: CreatorRole,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NewResource {
    pub kind: ResourceKind,
    pub title: NonEmptyStr,
    pub creators: Vec<Creator>,
    pub published: Option<BibliographicDate>,
    pub accessed: Option<BibliographicDate>,
    pub parent_id: Option<ResourceId>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResourceError {
    #[error("resource requires at least one creator")]
    EmptyCreators,
    #[error("duplicate creator person_id and role")]
    DuplicateCreator,
    #[error("parent_id is forbidden for this kind")]
    ParentForbidden,
    #[error("parent_id is required for this kind")]
    ParentRequired,
    #[error("parent kind does not match expected container")]
    ParentKindMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Resource {
    pub id: ResourceId,
    pub kind: ResourceKind,
    pub title: NonEmptyStr,
    pub creators: Vec<Creator>,
    pub published: Option<BibliographicDate>,
    pub accessed: Option<BibliographicDate>,
    pub parent_id: Option<ResourceId>,
}

impl Resource {
    /// Does not check that creator PersonIds or parent_id exist.
    pub fn try_new(id: ResourceId, new: NewResource) -> Result<Self, ResourceError> {
        if new.creators.is_empty() && !new.kind.is_container() {
            return Err(ResourceError::EmptyCreators);
        }
        for (i, creator) in new.creators.iter().enumerate() {
            if new.creators[..i]
                .iter()
                .any(|c| c.person_id == creator.person_id && c.role == creator.role)
            {
                return Err(ResourceError::DuplicateCreator);
            }
        }
        match (new.kind.parent_rule(), new.parent_id.is_some()) {
            (ParentRule::Forbidden, true) => return Err(ResourceError::ParentForbidden),
            (ParentRule::Required { .. }, false) => return Err(ResourceError::ParentRequired),
            _ => {}
        }
        Ok(Self {
            id,
            kind: new.kind,
            title: new.title,
            creators: new.creators,
            published: new.published,
            accessed: new.accessed,
            parent_id: new.parent_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(s: &str) -> NonEmptyStr {
        NonEmptyStr::try_new(s).unwrap()
    }

    fn one_creator() -> Vec<Creator> {
        vec![Creator {
            person_id: PersonId::generate(),
            role: CreatorRole::Author,
        }]
    }

    fn new_book(creators: Vec<Creator>, parent_id: Option<ResourceId>) -> NewResource {
        NewResource {
            kind: ResourceKind::Book,
            title: title("Confessions"),
            creators,
            published: None,
            accessed: None,
            parent_id,
        }
    }

    #[test]
    fn resource_id_parse_round_trip() {
        let id = ResourceId::generate();
        let parsed = ResourceId::parse(&id.to_string()).unwrap();
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_ulid(), id.as_ulid());
    }

    #[test]
    fn try_new_rejects_empty_creators() {
        let err = Resource::try_new(ResourceId::generate(), new_book(vec![], None)).unwrap_err();
        assert_eq!(err, ResourceError::EmptyCreators);
    }

    #[test]
    fn try_new_allows_empty_creators_on_containers() {
        for kind in [
            ResourceKind::Journal,
            ResourceKind::Podcast,
            ResourceKind::Blog,
        ] {
            let resource = Resource::try_new(
                ResourceId::generate(),
                NewResource {
                    kind,
                    title: title("Container"),
                    creators: vec![],
                    published: None,
                    accessed: None,
                    parent_id: None,
                },
            )
            .unwrap();
            assert!(resource.creators.is_empty());
        }
    }

    #[test]
    fn try_new_rejects_duplicate_person_and_role() {
        let person_id = PersonId::generate();
        let creators = vec![
            Creator {
                person_id,
                role: CreatorRole::Author,
            },
            Creator {
                person_id,
                role: CreatorRole::Author,
            },
        ];
        let err = Resource::try_new(ResourceId::generate(), new_book(creators, None)).unwrap_err();
        assert_eq!(err, ResourceError::DuplicateCreator);
    }

    #[test]
    fn try_new_allows_same_person_different_roles() {
        let person_id = PersonId::generate();
        let creators = vec![
            Creator {
                person_id,
                role: CreatorRole::Author,
            },
            Creator {
                person_id,
                role: CreatorRole::Editor,
            },
        ];
        let resource =
            Resource::try_new(ResourceId::generate(), new_book(creators, None)).unwrap();
        assert_eq!(resource.creators.len(), 2);
    }

    #[test]
    fn try_new_rejects_parent_when_forbidden() {
        let err = Resource::try_new(
            ResourceId::generate(),
            new_book(one_creator(), Some(ResourceId::generate())),
        )
        .unwrap_err();
        assert_eq!(err, ResourceError::ParentForbidden);
    }

    #[test]
    fn try_new_requires_parent_for_podcast_episode() {
        let err = Resource::try_new(
            ResourceId::generate(),
            NewResource {
                kind: ResourceKind::PodcastEpisode,
                title: title("Ep 1"),
                creators: one_creator(),
                published: None,
                accessed: None,
                parent_id: None,
            },
        )
        .unwrap_err();
        assert_eq!(err, ResourceError::ParentRequired);
    }

    #[test]
    fn try_new_allows_optional_parent_absent_or_present() {
        let without = Resource::try_new(
            ResourceId::generate(),
            NewResource {
                kind: ResourceKind::JournalArticle,
                title: title("Paper"),
                creators: one_creator(),
                published: None,
                accessed: None,
                parent_id: None,
            },
        )
        .unwrap();
        assert!(without.parent_id.is_none());

        let parent = ResourceId::generate();
        let with = Resource::try_new(
            ResourceId::generate(),
            NewResource {
                kind: ResourceKind::JournalArticle,
                title: title("Paper"),
                creators: one_creator(),
                published: None,
                accessed: None,
                parent_id: Some(parent),
            },
        )
        .unwrap();
        assert_eq!(with.parent_id, Some(parent));
    }

    #[test]
    fn parent_rule_podcast_episode_required_podcast() {
        assert_eq!(
            ResourceKind::PodcastEpisode.parent_rule(),
            ParentRule::Required {
                expected: ResourceKind::Podcast
            }
        );
    }

    #[test]
    fn parent_rule_containers_forbidden() {
        for kind in [
            ResourceKind::Journal,
            ResourceKind::Podcast,
            ResourceKind::Blog,
            ResourceKind::Book,
        ] {
            assert_eq!(kind.parent_rule(), ParentRule::Forbidden);
        }
    }

    #[test]
    fn accept_parent_checks_expected_kind() {
        assert!(ResourceKind::PodcastEpisode
            .accept_parent(ResourceKind::Podcast)
            .is_ok());
        assert_eq!(
            ResourceKind::PodcastEpisode.accept_parent(ResourceKind::Blog),
            Err(ResourceError::ParentKindMismatch)
        );
        assert_eq!(
            ResourceKind::Book.accept_parent(ResourceKind::Book),
            Err(ResourceError::ParentKindMismatch)
        );
    }

    #[test]
    fn bibliographic_date_keeps_instant_and_display() {
        let date = BibliographicDate::try_new(
            UtcMillis::from_unix_ms(0),
            Some(title("397")),
        )
        .unwrap();
        assert_eq!(date.instant().as_unix_ms(), 0);
        assert_eq!(date.display(), Some("397"));
    }
}
