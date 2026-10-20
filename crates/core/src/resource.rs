use std::fmt;

use crate::{Name, NonEmptyStr, PersonId, SeriesId};

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

/// Compact library-index row. Not a substitute for [`Resource`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ResourceSummary {
    pub id: ResourceId,
    pub kind: ResourceKind,
    pub title: NonEmptyStr,
    /// Empty when no creators; else one name, `"A; B"`, or `"A et al."`.
    pub creators_label: String,
    pub published_label: Option<NonEmptyStr>,
    pub publisher: Option<String>,
    /// Parent resource title when a parent link is set.
    pub container_title: Option<NonEmptyStr>,
}

/// Creator slot label: `credited_as` when set, else [`Name::display`].
pub fn creator_label_name(credited_as: Option<&NonEmptyStr>, name: &Name) -> String {
    match credited_as {
        Some(credited) => credited.as_str().to_owned(),
        None => name.display(),
    }
}

/// Format creator display names for a summary row.
///
/// - `[]` → `""`
/// - `[a]` → `a`
/// - `[a, b]` → `"a; b"`
/// - three or more → `"a et al."`
pub fn format_creators_label<S: AsRef<str>>(names: &[S]) -> String {
    match names.len() {
        0 => String::new(),
        1 => names[0].as_ref().to_owned(),
        2 => format!("{}; {}", names[0].as_ref(), names[1].as_ref()),
        _ => format!("{} et al.", names[0].as_ref()),
    }
}

/// Prefer fuzzy [`BibliographicDate::display`]; else UTC calendar year of the instant.
pub fn format_published_label(published: &BibliographicDate) -> NonEmptyStr {
    if let Some(display) = published.display() {
        return NonEmptyStr::try_new(display).expect("display already NonEmptyStr");
    }
    let year = utc_year_from_millis(published.instant().as_unix_ms());
    NonEmptyStr::try_new(year.to_string()).expect("year digits non-empty")
}

pub fn published_label_from(published: Option<&BibliographicDate>) -> Option<NonEmptyStr> {
    published.map(format_published_label)
}

/// Civil UTC year from Unix millis (Howard Hinnant `civil_from_days`, year only).
fn utc_year_from_millis(ms: i64) -> i32 {
    let days = ms.div_euclid(86_400_000);
    // Days since civil 0000-03-01.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        (y + 1) as i32
    } else {
        y as i32
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

/// Write-side creator link for [`NewResource`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CreatorSpec {
    pub person_id: PersonId,
    pub role: CreatorRole,
    pub credited_as: Option<NonEmptyStr>,
}

/// Credited person on a [`Resource`], with bibliography order.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Creator {
    pub person_id: PersonId,
    pub role: CreatorRole,
    pub order: u32,
    pub credited_as: Option<NonEmptyStr>,
}

impl Creator {
    pub fn from_spec(order: u32, spec: CreatorSpec) -> Self {
        Self {
            person_id: spec.person_id,
            role: spec.role,
            order,
            credited_as: spec.credited_as,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NewResource {
    pub kind: ResourceKind,
    pub title: NonEmptyStr,
    pub creators: Vec<CreatorSpec>,
    pub publisher: Option<String>,
    pub published: Option<BibliographicDate>,
    pub accessed: Option<BibliographicDate>,
    pub parent_id: Option<ResourceId>,
    pub series_id: Option<SeriesId>,
    pub series_number: Option<NonEmptyStr>,
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
    #[error("series_number requires series_id")]
    SeriesNumberWithoutSeries,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Resource {
    pub id: ResourceId,
    pub kind: ResourceKind,
    pub title: NonEmptyStr,
    pub creators: Vec<Creator>,
    pub publisher: Option<String>,
    pub published: Option<BibliographicDate>,
    pub accessed: Option<BibliographicDate>,
    pub parent_id: Option<ResourceId>,
    pub series_id: Option<SeriesId>,
    pub series_number: Option<NonEmptyStr>,
}

fn validate_creators(
    kind: ResourceKind,
    creators: &[Creator],
) -> Result<(), ResourceError> {
    if creators.is_empty() && !kind.is_container() {
        return Err(ResourceError::EmptyCreators);
    }
    for (i, creator) in creators.iter().enumerate() {
        if creators[..i]
            .iter()
            .any(|c| c.person_id == creator.person_id && c.role == creator.role)
        {
            return Err(ResourceError::DuplicateCreator);
        }
    }
    Ok(())
}

fn validate_parent_presence(
    kind: ResourceKind,
    parent_id: Option<ResourceId>,
) -> Result<(), ResourceError> {
    match (kind.parent_rule(), parent_id.is_some()) {
        (ParentRule::Forbidden, true) => Err(ResourceError::ParentForbidden),
        (ParentRule::Required { .. }, false) => Err(ResourceError::ParentRequired),
        _ => Ok(()),
    }
}

fn validate_series(
    series_id: Option<SeriesId>,
    series_number: Option<&NonEmptyStr>,
) -> Result<(), ResourceError> {
    if series_number.is_some() && series_id.is_none() {
        Err(ResourceError::SeriesNumberWithoutSeries)
    } else {
        Ok(())
    }
}

impl Resource {
    /// Domain shape only. Does not check person/parent/series existence.
    /// Assigns `Creator.order` from `creators` enumeration (0..n).
    /// Does **not** call `accept_parent` (needs loaded parent kind — command boundary).
    pub fn try_new(id: ResourceId, new: NewResource) -> Result<Self, ResourceError> {
        let creators: Vec<Creator> = new
            .creators
            .into_iter()
            .enumerate()
            .map(|(i, spec)| Creator::from_spec(i as u32, spec))
            .collect();
        validate_creators(new.kind, &creators)?;
        validate_parent_presence(new.kind, new.parent_id)?;
        validate_series(new.series_id, new.series_number.as_ref())?;
        Ok(Self {
            id,
            kind: new.kind,
            title: new.title,
            creators,
            publisher: new.publisher,
            published: new.published,
            accessed: new.accessed,
            parent_id: new.parent_id,
            series_id: new.series_id,
            series_number: new.series_number,
        })
    }

    /// Reassemble after loading a row + ordered edge projection.
    /// Assumes `creators` already sorted by `order`.
    pub fn from_stored(
        id: ResourceId,
        kind: ResourceKind,
        title: NonEmptyStr,
        creators: Vec<Creator>,
        publisher: Option<String>,
        published: Option<BibliographicDate>,
        accessed: Option<BibliographicDate>,
        parent_id: Option<ResourceId>,
        series_id: Option<SeriesId>,
        series_number: Option<NonEmptyStr>,
    ) -> Result<Self, ResourceError> {
        validate_creators(kind, &creators)?;
        validate_parent_presence(kind, parent_id)?;
        validate_series(series_id, series_number.as_ref())?;
        Ok(Self {
            id,
            kind,
            title,
            creators,
            publisher,
            published,
            accessed,
            parent_id,
            series_id,
            series_number,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(s: &str) -> NonEmptyStr {
        NonEmptyStr::try_new(s).unwrap()
    }

    fn one_creator() -> Vec<CreatorSpec> {
        vec![CreatorSpec {
            person_id: PersonId::generate(),
            role: CreatorRole::Author,
            credited_as: None,
        }]
    }

    fn new_book(creators: Vec<CreatorSpec>, parent_id: Option<ResourceId>) -> NewResource {
        NewResource {
            kind: ResourceKind::Book,
            title: title("Confessions"),
            creators,
            publisher: None,
            published: None,
            accessed: None,
            parent_id,
            series_id: None,
            series_number: None,
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
                    publisher: None,
                    published: None,
                    accessed: None,
                    parent_id: None,
                    series_id: None,
                    series_number: None,
                },
            )
            .unwrap();
            assert!(resource.creators.is_empty());
        }
    }

    #[test]
    fn try_new_assigns_creator_order_from_index() {
        let a = PersonId::generate();
        let b = PersonId::generate();
        let resource = Resource::try_new(
            ResourceId::generate(),
            new_book(
                vec![
                    CreatorSpec {
                        person_id: a,
                        role: CreatorRole::Author,
                        credited_as: None,
                    },
                    CreatorSpec {
                        person_id: b,
                        role: CreatorRole::Editor,
                        credited_as: Some(title("ed.")),
                    },
                ],
                None,
            ),
        )
        .unwrap();
        assert_eq!(resource.creators[0].order, 0);
        assert_eq!(resource.creators[0].person_id, a);
        assert_eq!(resource.creators[1].order, 1);
        assert_eq!(
            resource.creators[1].credited_as.as_ref().map(NonEmptyStr::as_str),
            Some("ed.")
        );
    }

    #[test]
    fn try_new_rejects_duplicate_person_and_role() {
        let person_id = PersonId::generate();
        let creators = vec![
            CreatorSpec {
                person_id,
                role: CreatorRole::Author,
                credited_as: None,
            },
            CreatorSpec {
                person_id,
                role: CreatorRole::Author,
                credited_as: None,
            },
        ];
        let err = Resource::try_new(ResourceId::generate(), new_book(creators, None)).unwrap_err();
        assert_eq!(err, ResourceError::DuplicateCreator);
    }

    #[test]
    fn try_new_allows_same_person_different_roles() {
        let person_id = PersonId::generate();
        let creators = vec![
            CreatorSpec {
                person_id,
                role: CreatorRole::Author,
                credited_as: None,
            },
            CreatorSpec {
                person_id,
                role: CreatorRole::Editor,
                credited_as: None,
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
                publisher: None,
                published: None,
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
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
                publisher: None,
                published: None,
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
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
                publisher: None,
                published: None,
                accessed: None,
                parent_id: Some(parent),
                series_id: None,
                series_number: None,
            },
        )
        .unwrap();
        assert_eq!(with.parent_id, Some(parent));
    }

    #[test]
    fn try_new_rejects_series_number_without_series() {
        let err = Resource::try_new(
            ResourceId::generate(),
            NewResource {
                kind: ResourceKind::Book,
                title: title("Confessions"),
                creators: one_creator(),
                publisher: None,
                published: None,
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: Some(title("LCL 26")),
            },
        )
        .unwrap_err();
        assert_eq!(err, ResourceError::SeriesNumberWithoutSeries);
    }

    #[test]
    fn try_new_keeps_publisher_and_series() {
        let series_id = SeriesId::generate();
        let resource = Resource::try_new(
            ResourceId::generate(),
            NewResource {
                kind: ResourceKind::Book,
                title: title("Confessions"),
                creators: one_creator(),
                publisher: Some("Harvard University Press".into()),
                published: None,
                accessed: None,
                parent_id: None,
                series_id: Some(series_id),
                series_number: Some(title("LCL 26")),
            },
        )
        .unwrap();
        assert_eq!(
            resource.publisher.as_deref(),
            Some("Harvard University Press")
        );
        assert_eq!(resource.series_id, Some(series_id));
        assert_eq!(
            resource.series_number.as_ref().map(NonEmptyStr::as_str),
            Some("LCL 26")
        );
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
        let date =
            BibliographicDate::try_new(UtcMillis::from_unix_ms(0), Some(title("397"))).unwrap();
        assert_eq!(date.instant().as_unix_ms(), 0);
        assert_eq!(date.display(), Some("397"));
    }

    #[test]
    fn from_stored_rejects_corrupt_empty_creators() {
        let err = Resource::from_stored(
            ResourceId::generate(),
            ResourceKind::Book,
            title("Orphan"),
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap_err();
        assert_eq!(err, ResourceError::EmptyCreators);
    }

    #[test]
    fn format_creators_label_zero_one_two_et_al() {
        assert_eq!(format_creators_label(&[] as &[&str]), "");
        assert_eq!(format_creators_label(&["Augustine"]), "Augustine");
        assert_eq!(
            format_creators_label(&["Augustine", "Chadwick"]),
            "Augustine; Chadwick"
        );
        assert_eq!(
            format_creators_label(&["A", "B", "C"]),
            "A et al."
        );
        assert_eq!(
            format_creators_label(&["A", "B", "C", "D"]),
            "A et al."
        );
    }

    #[test]
    fn creator_label_name_prefers_credited_as() {
        let name = Name::try_new(None::<&str>, Some("Augustine"), None::<&str>).unwrap();
        assert_eq!(creator_label_name(None, &name), "Augustine");
        assert_eq!(
            creator_label_name(Some(&title("St. Augustine")), &name),
            "St. Augustine"
        );
    }

    #[test]
    fn format_published_label_prefers_display() {
        let date =
            BibliographicDate::try_new(UtcMillis::from_unix_ms(0), Some(title("Summer 397")))
                .unwrap();
        assert_eq!(format_published_label(&date).as_str(), "Summer 397");
    }

    #[test]
    fn format_published_label_falls_back_to_utc_year() {
        // 2021-01-01T00:00:00Z
        let date =
            BibliographicDate::try_new(UtcMillis::from_unix_ms(1_609_459_200_000), None).unwrap();
        assert_eq!(format_published_label(&date).as_str(), "2021");
        assert_eq!(
            published_label_from(Some(&date))
                .as_ref()
                .map(NonEmptyStr::as_str),
            Some("2021")
        );
        assert_eq!(published_label_from(None), None);

        assert_eq!(utc_year_from_millis(0), 1970);
        assert_eq!(utc_year_from_millis(-1), 1969);
    }
}
