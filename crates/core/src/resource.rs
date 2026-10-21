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

    /// Creation-time millis the ULID itself encodes: the fallback
    /// `date_added` for rows persisted before record-keeping dates existed,
    /// so no backfill migration is needed. The 48-bit ULID timestamp always
    /// fits an i64.
    pub fn timestamp_ms(self) -> i64 {
        self.0.timestamp_ms() as i64
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
    /// Every kind, for shells that enumerate the whole set (dropdowns,
    /// round-trip tests).
    pub const ALL: [Self; 20] = [
        Self::Book,
        Self::JournalArticle,
        Self::BookSection,
        Self::ConferencePaper,
        Self::Preprint,
        Self::Thesis,
        Self::Report,
        Self::Webpage,
        Self::Dataset,
        Self::Manuscript,
        Self::MagazineArticle,
        Self::NewspaperArticle,
        Self::ComputerProgram,
        Self::DictionaryEntry,
        Self::EncyclopediaArticle,
        Self::BlogPost,
        Self::PodcastEpisode,
        Self::Journal,
        Self::Podcast,
        Self::Blog,
    ];

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

    /// Parse the slug [`ResourceKind::as_str`] emits. `None` on any other
    /// input, so callers own their error message.
    pub fn from_slug(s: &str) -> Option<Self> {
        Some(match s {
            "book" => Self::Book,
            "journalArticle" => Self::JournalArticle,
            "bookSection" => Self::BookSection,
            "conferencePaper" => Self::ConferencePaper,
            "preprint" => Self::Preprint,
            "thesis" => Self::Thesis,
            "report" => Self::Report,
            "webpage" => Self::Webpage,
            "dataset" => Self::Dataset,
            "manuscript" => Self::Manuscript,
            "magazineArticle" => Self::MagazineArticle,
            "newspaperArticle" => Self::NewspaperArticle,
            "computerProgram" => Self::ComputerProgram,
            "dictionaryEntry" => Self::DictionaryEntry,
            "encyclopediaArticle" => Self::EncyclopediaArticle,
            "blogPost" => Self::BlogPost,
            "podcastEpisode" => Self::PodcastEpisode,
            "journal" => Self::Journal,
            "podcast" => Self::Podcast,
            "blog" => Self::Blog,
            _ => return None,
        })
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

    /// The current instant, for record-keeping stamps (`date_added`,
    /// `date_edited`): the same UTC-millisecond resolution the type stores.
    pub fn now() -> Self {
        let now = time::OffsetDateTime::now_utc();
        // The millisecond half rounds down, so a stamp never lands in
        // the future.
        Self(
            now.unix_timestamp()
                .saturating_mul(1_000)
                .saturating_add(i64::from(now.millisecond())),
        )
    }

    /// The year's January 1, midnight UTC — the boundary a year-bracketed
    /// range keys on. `None` when the year falls outside the calendar.
    pub fn jan_1_utc(year: i64) -> Option<Self> {
        let date = time::Date::from_calendar_date(
            i32::try_from(year).ok()?,
            time::Month::January,
            1,
        )
        .ok()?;
        let midnight = time::PrimitiveDateTime::new(date, time::Time::MIDNIGHT);
        Some(Self::from_unix_ms(midnight.assume_utc().unix_timestamp() * 1_000))
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
    /// Record-keeping stamps, always present on a stored record; shells
    /// label them with [`format_recorded_date`].
    pub date_added: UtcMillis,
    pub date_edited: UtcMillis,
}

/// One newest-first page of summary rows from `Store::list_resources_page`.
///
/// `next_before` is the cursor for the following page: the id of this page's
/// last row when more rows may exist, `None` at the end of the library.
/// Feed it back verbatim as the next call's `before`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ResourcePage {
    pub rows: Vec<ResourceSummary>,
    pub next_before: Option<ResourceId>,
}

/// Field a sorted library listing orders by.
///
/// Order keys, shared by every adapter:
/// - [`ResourceSort::Title`] — case-insensitive text order.
/// - [`ResourceSort::PrimaryCreator`] — the first creator in bibliography
///   order keys on its family name ([`primary_creator_sort_key`]).
/// - [`ResourceSort::Published`] — the chronological instant, never the
///   display label ("Summer 397" has no order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ResourceSort {
    Title,
    PrimaryCreator,
    Published,
}

/// Direction of a [`ResourceSort`]. Descending is the exact key-reversal of
/// ascending: rows without the sort field move to the other end, and ties
/// stay newest-first in both directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl ResourceSort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::PrimaryCreator => "primaryCreator",
            Self::Published => "published",
        }
    }
}

impl SortDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ascending => "ascending",
            Self::Descending => "descending",
        }
    }
}

/// One library listing query: the order rows come back in, plus the
/// additive predicates every returned row must satisfy.
///
/// `sort: None` is the default listing order — newest-first by id, the
/// same order [`Store::list_resources_page`] walks by cursor — while
/// `Some((field, direction))` applies that field's order contract (see
/// [`ResourceSort`]). Because this seam pages by offset, every order
/// composes with every filter through one window contract.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResourceQuery {
    /// Field and direction to order by; `None` orders newest-first.
    pub sort: Option<(ResourceSort, SortDirection)>,
    /// Predicates every returned row must satisfy;
    /// [`ResourceFilter::none`] matches every resource.
    pub filter: ResourceFilter,
}

impl ResourceQuery {
    /// The default listing: newest-first, unfiltered.
    pub fn newest_first() -> Self {
        Self {
            sort: None,
            filter: ResourceFilter::none(),
        }
    }

    /// A sorted, unfiltered listing.
    pub fn sorted(field: ResourceSort, direction: SortDirection) -> Self {
        Self {
            sort: Some((field, direction)),
            filter: ResourceFilter::none(),
        }
    }
}

/// Additive predicates a library listing applies before ordering and
/// paging: every set field must hold (AND). [`ResourceFilter::none`]
/// matches every resource.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResourceFilter {
    /// Case-insensitive substring the title must contain.
    pub title: Option<NonEmptyStr>,
    /// A person with a creator edge to the resource, sole or shared.
    pub creator: Option<PersonId>,
    /// Window the published instant must fall in.
    pub published: Option<PublishedRange>,
    /// The exact kind a row must carry, so kind-scoped listings (a parent
    /// picker's candidates, for instance) narrow at the store instead of
    /// paging the whole library.
    pub kind: Option<ResourceKind>,
}

impl ResourceFilter {
    /// The filter that matches every resource.
    pub fn none() -> Self {
        Self {
            title: None,
            creator: None,
            published: None,
            kind: None,
        }
    }

    /// The filter matching every resource of one kind.
    pub fn by_kind(kind: ResourceKind) -> Self {
        Self {
            kind: Some(kind),
            ..Self::none()
        }
    }

    /// True when at least one predicate is set, so the listing narrows.
    pub fn is_active(&self) -> bool {
        self.title.is_some()
            || self.creator.is_some()
            || self.kind.is_some()
            || self
                .published
                .as_ref()
                .is_some_and(PublishedRange::is_active)
    }
}

/// An instant window over published dates. The `from` bound includes its
/// instant, the `to` bound excludes it, so whole years bracket as
/// [January 1, next January 1) and never miss the year's final day.
/// Either bound may be open; undated resources match only while the
/// window bounds nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PublishedRange {
    /// Earliest instant that still matches, if bounded below.
    pub from: Option<UtcMillis>,
    /// Instant after the last that matches, if bounded above.
    pub to: Option<UtcMillis>,
}

impl PublishedRange {
    /// True when either bound is set, so the window narrows.
    pub fn is_active(&self) -> bool {
        self.from.is_some() || self.to.is_some()
    }
}

/// One window of a queried library listing from
/// [`Store::list_resources_query_page`]: `limit` rows at a 0-based offset
/// into the ordered list. The database query owns the ordering.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SortedResourcePage {
    pub rows: Vec<ResourceSummary>,
    /// Offset of the following page when more rows may exist; `None` at the
    /// end of the ordered list.
    pub next_offset: Option<u64>,
}

/// Creators of one role for a [`ResourceDetail`]: every credited name,
/// bibliography order. Names stay a list; a shell stacks or joins them at
/// display time (summaries truncate with `"et al."`; details show them all).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DetailCreators {
    pub role: CreatorRole,
    pub names: Vec<String>,
}

/// Fully-resolved resource for detail views.
///
/// Like [`ResourceSummary`] this is a read-time projection, but complete:
/// creator `person_id`s are replaced with credited names grouped by role, and
/// the parent / series ids with their titles. Dates stay raw — shells label
/// the bibliographic pair with [`format_published_label`] and the
/// record-keeping pair with [`format_recorded_date`]. The command layer
/// composes it from [`Resource`] plus lookups; the store does not build it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ResourceDetail {
    pub id: ResourceId,
    pub kind: ResourceKind,
    pub title: NonEmptyStr,
    /// One [`DetailCreators`] per role, in first-appearance order.
    pub creators: Vec<DetailCreators>,
    pub publisher: Option<String>,
    pub published: Option<BibliographicDate>,
    pub accessed: Option<BibliographicDate>,
    /// Parent resource's title when a parent link is set.
    pub container_title: Option<NonEmptyStr>,
    pub series_title: Option<NonEmptyStr>,
    pub series_number: Option<NonEmptyStr>,
    /// Record-keeping stamps, always present on a stored record.
    pub date_added: UtcMillis,
    pub date_edited: UtcMillis,
}

/// Group creators into one [`DetailCreators`] per role.
///
/// `label` resolves one creator to its credited display name (see
/// [`creator_label_name`]). Roles appear in first-appearance order; names
/// inside a role keep bibliography order.
pub fn detail_creator_groups<F>(creators: &[Creator], label: F) -> Vec<DetailCreators>
where
    F: Fn(&Creator) -> String,
{
    // Accumulate names per role first so the join happens once per group.
    let mut groups: Vec<(CreatorRole, Vec<String>)> = Vec::new();
    for creator in creators {
        match groups.iter_mut().find(|(role, _)| *role == creator.role) {
            Some((_, names)) => names.push(label(creator)),
            None => groups.push((creator.role, vec![label(creator)])),
        }
    }
    groups
        .into_iter()
        .map(|(role, names)| DetailCreators { role, names })
        .collect()
}

/// Creator slot label: `credited_as` when set, else [`Name::display`].
pub fn creator_label_name(credited_as: Option<&NonEmptyStr>, name: &Name) -> String {
    match credited_as {
        Some(credited) => credited.as_str().to_owned(),
        None => name.display(),
    }
}

/// Sort key for a creator: the family name a library listing orders by.
///
/// `credited_as` keys on its last word ("A. Lovelace" → "lovelace");
/// otherwise the person's `last` name — mononyms live there — else `first`.
/// Lowercased, so display case never reorders rows.
pub fn primary_creator_sort_key(credited_as: Option<&NonEmptyStr>, name: &Name) -> String {
    match credited_as {
        Some(credited) => credited
            .as_str()
            .split_whitespace()
            .next_back()
            .unwrap_or(credited.as_str())
            .to_lowercase(),
        None => name
            .last()
            .or_else(|| name.first())
            .expect("a Name carries a first or last name")
            .to_lowercase(),
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

/// English month abbreviations in calendar order, the word each
/// [`format_recorded_date`] label leads with.
const RECORDED_MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Record-keeping stamp as `Mon D, YYYY`, the label shells show for a
/// resource's `date_added` / `date_edited`. UTC deliberately: a library
/// table reads the same day whatever timezone it opens in, and tests stay
/// deterministic.
pub fn format_recorded_date(instant: UtcMillis) -> String {
    let stamp =
        time::OffsetDateTime::from_unix_timestamp(instant.as_unix_ms().div_euclid(1_000))
            .expect("recorded stamp within OffsetDateTime range");
    format!(
        "{} {}, {}",
        RECORDED_MONTHS[(stamp.month() as usize) - 1],
        stamp.day(),
        stamp.year()
    )
}

pub fn utc_year_from_millis(ms: i64) -> i32 {
    time::OffsetDateTime::from_unix_timestamp(ms.div_euclid(1_000))
        .expect("bibliographic instant within OffsetDateTime range")
        .year()
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

impl CreatorRole {
    /// Parse the slug the CLI's `--creator` flag takes. `None` on any other
    /// input, so callers own their error message.
    pub fn from_slug(s: &str) -> Option<Self> {
        Some(match s {
            "author" => Self::Author,
            "editor" => Self::Editor,
            "translator" => Self::Translator,
            "contributor" => Self::Contributor,
            "host" => Self::Host,
            "director" => Self::Director,
            _ => return None,
        })
    }
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

    /// Write-side twin of this slot. Drops `order`: the update re-derives it
    /// from list position, the same rule create applies.
    pub fn to_spec(&self) -> CreatorSpec {
        CreatorSpec {
            person_id: self.person_id,
            role: self.role,
            credited_as: self.credited_as.clone(),
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
    /// When the record joined the library: a record-keeping stamp, never
    /// the bibliographic `published`.
    pub date_added: UtcMillis,
    /// When the record last changed; every create and update restamps it.
    pub date_edited: UtcMillis,
}

fn validate_creators(kind: ResourceKind, creators: &[Creator]) -> Result<(), ResourceError> {
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
        // One instant stamps both record-keeping dates: the record joined
        // the library now and has not changed since.
        let now = UtcMillis::now();
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
            date_added: now,
            date_edited: now,
        })
    }

    /// Rebuild for [`Store::update_resource`](crate::store::Store::update_resource):
    /// the same shape validation as [`Resource::try_new`], carrying the
    /// record's original `date_added` — an edit moves `date_edited` to now
    /// but never the day the record joined the library.
    pub fn try_new_updated(
        id: ResourceId,
        date_added: UtcMillis,
        new: NewResource,
    ) -> Result<Self, ResourceError> {
        let mut resource = Self::try_new(id, new)?;
        resource.date_added = date_added;
        Ok(resource)
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
        date_added: UtcMillis,
        date_edited: UtcMillis,
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
            date_added,
            date_edited,
        })
    }

    /// Write-side twin of the stored record for
    /// [`Store::update_resource`](crate::store::Store::update_resource):
    /// shells merge field edits into it and send it back as the full
    /// replacement. Creator order drops out; position is the order, the
    /// same rule create applies.
    pub fn to_new(&self) -> NewResource {
        NewResource {
            kind: self.kind,
            title: self.title.clone(),
            creators: self.creators.iter().map(Creator::to_spec).collect(),
            publisher: self.publisher.clone(),
            published: self.published.clone(),
            accessed: self.accessed.clone(),
            parent_id: self.parent_id,
            series_id: self.series_id,
            series_number: self.series_number.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jan_1_utc_lands_on_new_year_midnights() {
        // The seed's former single anchor doubles as the oracle.
        assert_eq!(
            UtcMillis::jan_1_utc(2021),
            Some(UtcMillis::from_unix_ms(1_609_459_200_000))
        );
        // One calendar year apart: the 2021 span is common, the 2020 span
        // leap.
        assert_eq!(
            UtcMillis::jan_1_utc(2022).map(|t| t.as_unix_ms() - 1_609_459_200_000),
            Some(31_536_000_000)
        );
        assert_eq!(
            UtcMillis::jan_1_utc(2020).map(|t| 1_609_459_200_000 - t.as_unix_ms()),
            Some(31_622_400_000)
        );
        // Years outside the calendar reject rather than wrap.
        assert_eq!(UtcMillis::jan_1_utc(3_000_000_000), None);
    }

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
    fn resource_id_timestamp_ms_matches_the_ulid() {
        // The ULID's encoded millis are the oracle; the accessor reads
        // them for the legacy-row date_added backfill.
        let id = ResourceId(ulid::Ulid::from_parts(1_609_459_200_000, 0));
        assert_eq!(id.timestamp_ms(), 1_609_459_200_000);
    }

    #[test]
    fn try_new_stamps_both_record_dates_with_one_instant() {
        let before = UtcMillis::now().as_unix_ms();
        let resource = Resource::try_new(ResourceId::generate(), new_book(one_creator(), None))
            .expect("book shape valid");
        let after = UtcMillis::now().as_unix_ms();
        // Create stamps both record-keeping dates with the same instant:
        // the record joined the library now and has not changed since.
        assert_eq!(resource.date_added, resource.date_edited);
        assert!(resource.date_added.as_unix_ms() >= before);
        assert!(resource.date_added.as_unix_ms() <= after);
    }

    #[test]
    fn try_new_updated_keeps_date_added_and_restamps_date_edited() {
        let added = UtcMillis::from_unix_ms(1_500_000_000_000);
        let resource =
            Resource::try_new_updated(ResourceId::generate(), added, new_book(one_creator(), None))
                .expect("book shape valid");
        // An edit moves the edited stamp on from the added stamp, which is
        // the record's history, never the edit's clock.
        assert_eq!(resource.date_added, added);
        assert!(resource.date_edited.as_unix_ms() >= added.as_unix_ms());
    }

    #[test]
    fn from_stored_keeps_the_stored_record_dates() {
        let added = UtcMillis::from_unix_ms(1_500_000_000_000);
        let edited = UtcMillis::from_unix_ms(1_600_000_000_000);
        let resource = Resource::from_stored(
            ResourceId::generate(),
            ResourceKind::Journal,
            title("Oeuvre Journal"),
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            added,
            edited,
        )
        .expect("containers carry no creators");
        assert_eq!((resource.date_added, resource.date_edited), (added, edited));
    }

    #[test]
    fn format_recorded_date_renders_month_day_year() {
        // UTC midnight into January 1 lands on that day, and the epoch's
        // single-digit day prints unpadded.
        assert_eq!(
            format_recorded_date(UtcMillis::from_unix_ms(1_609_459_200_000)),
            "Jan 1, 2021"
        );
        assert_eq!(format_recorded_date(UtcMillis::from_unix_ms(0)), "Jan 1, 1970");
        // The millisecond half floors, so the label never rolls to the
        // next day on sub-second remainders.
        assert_eq!(
            format_recorded_date(UtcMillis::from_unix_ms(1_609_459_199_999)),
            "Dec 31, 2020"
        );
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
            resource.creators[1]
                .credited_as
                .as_ref()
                .map(NonEmptyStr::as_str),
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
        let resource = Resource::try_new(ResourceId::generate(), new_book(creators, None)).unwrap();
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
        assert!(
            ResourceKind::PodcastEpisode
                .accept_parent(ResourceKind::Podcast)
                .is_ok()
        );
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
            UtcMillis::from_unix_ms(0),
            UtcMillis::from_unix_ms(0),
        )
        .unwrap_err();
        assert_eq!(err, ResourceError::EmptyCreators);
    }

    #[test]
    fn to_new_round_trips_every_field() {
        let author = PersonId::generate();
        let editor = PersonId::generate();
        let new = NewResource {
            kind: ResourceKind::BookSection,
            title: title("Nested editing"),
            creators: vec![
                CreatorSpec {
                    person_id: author,
                    role: CreatorRole::Author,
                    credited_as: Some(title("A. Author")),
                },
                CreatorSpec {
                    person_id: editor,
                    role: CreatorRole::Editor,
                    credited_as: None,
                },
            ],
            publisher: Some("MIT".into()),
            published: Some(
                BibliographicDate::try_new(
                    UtcMillis::jan_1_utc(2021).expect("2021 in calendar"),
                    Some(title("Spring 2021")),
                )
                .expect("valid date"),
            ),
            accessed: None,
            parent_id: Some(ResourceId::generate()),
            series_id: Some(SeriesId::generate()),
            series_number: Some(title("3")),
        };
        let stored = Resource::try_new(ResourceId::generate(), new.clone()).unwrap();
        assert_eq!(stored.to_new(), new);
    }

    #[test]
    fn from_slug_round_trips_every_kind() {
        let kinds = [
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
        for kind in kinds {
            assert_eq!(ResourceKind::from_slug(kind.as_str()), Some(kind));
        }
        assert_eq!(ResourceKind::from_slug("novel"), None);
        // Slugs are exact: display case never parses.
        assert_eq!(ResourceKind::from_slug("Book"), None);
    }

    #[test]
    fn creator_role_from_slug_parses_every_role() {
        let pairs = [
            ("author", CreatorRole::Author),
            ("editor", CreatorRole::Editor),
            ("translator", CreatorRole::Translator),
            ("contributor", CreatorRole::Contributor),
            ("host", CreatorRole::Host),
            ("director", CreatorRole::Director),
        ];
        for (slug, role) in pairs {
            assert_eq!(CreatorRole::from_slug(slug), Some(role));
        }
        assert_eq!(CreatorRole::from_slug("publisher"), None);
    }

    #[test]
    fn format_creators_label_zero_one_two_et_al() {
        assert_eq!(format_creators_label(&[] as &[&str]), "");
        assert_eq!(format_creators_label(&["Augustine"]), "Augustine");
        assert_eq!(
            format_creators_label(&["Augustine", "Chadwick"]),
            "Augustine; Chadwick"
        );
        assert_eq!(format_creators_label(&["A", "B", "C"]), "A et al.");
        assert_eq!(format_creators_label(&["A", "B", "C", "D"]), "A et al.");
    }

    #[test]
    fn detail_creator_groups_one_group_per_role_in_appearance_order() {
        let creator = |order: u32, role: CreatorRole| Creator {
            person_id: PersonId::generate(),
            role,
            order,
            credited_as: None,
        };
        let creators = vec![
            creator(0, CreatorRole::Author),
            creator(1, CreatorRole::Editor),
            creator(2, CreatorRole::Author),
            creator(3, CreatorRole::Editor),
        ];
        let groups = detail_creator_groups(&creators, |c| format!("name-{}", c.order));
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].role, CreatorRole::Author);
        assert_eq!(groups[0].names, vec!["name-0", "name-2"]);
        assert_eq!(groups[1].role, CreatorRole::Editor);
        assert_eq!(groups[1].names, vec!["name-1", "name-3"]);
        assert!(detail_creator_groups(&[], |_: &Creator| String::new()).is_empty());
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
    fn primary_creator_sort_key_keys_on_family_name() {
        let lovelace = Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap();
        assert_eq!(primary_creator_sort_key(None, &lovelace), "lovelace");
        // Mononyms live in `last`, the primary sort slot.
        let augustine = Name::try_new(None::<&str>, Some("Augustine"), None::<&str>).unwrap();
        assert_eq!(primary_creator_sort_key(None, &augustine), "augustine");
        // A first-only name keys on the given name.
        let first_only = Name::try_new(Some("Cher"), None::<&str>, None::<&str>).unwrap();
        assert_eq!(primary_creator_sort_key(None, &first_only), "cher");
        // Display case never reorders rows, and a suffix is not a family name.
        let suffixed = Name::try_new(Some("GRACE"), Some("HOPPER"), Some("Jr.")).unwrap();
        assert_eq!(primary_creator_sort_key(None, &suffixed), "hopper");
    }

    #[test]
    fn primary_creator_sort_key_credited_string_keys_on_its_last_word() {
        let lovelace = Name::try_new(Some("Ada"), Some("Lovelace"), None::<&str>).unwrap();
        assert_eq!(
            primary_creator_sort_key(Some(&title("A. Lovelace")), &lovelace),
            "lovelace"
        );
        // The credited string wins even when it disagrees with the name.
        assert_eq!(
            primary_creator_sort_key(Some(&title("Countess of Lovelace")), &lovelace),
            "lovelace"
        );
    }

    #[test]
    fn format_published_label_prefers_display() {
        let date =
            BibliographicDate::try_new(UtcMillis::from_unix_ms(0), Some(title("Summer 397")))
                .unwrap();
        assert_eq!(format_published_label(&date).as_str(), "Summer 397");
    }

    const UNIX_MS_2021_01_01_UTC: i64 = 1_609_459_200_000;

    #[test]
    fn format_published_label_falls_back_to_utc_year() {
        let date =
            BibliographicDate::try_new(UtcMillis::from_unix_ms(UNIX_MS_2021_01_01_UTC), None)
                .unwrap();
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

    #[test]
    fn by_kind_sets_only_kind_and_narrows_the_listing() {
        let filter = ResourceFilter::by_kind(ResourceKind::Journal);
        assert_eq!(filter.title, None);
        assert_eq!(filter.creator, None);
        assert_eq!(filter.published, None);
        assert_eq!(filter.kind, Some(ResourceKind::Journal));
        assert!(filter.is_active());
        assert!(!ResourceFilter::none().is_active());
    }
}
