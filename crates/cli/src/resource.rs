use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use oeuvre_app::command::{GetResource, ListResources, ListResourcesQueryPage, UpdateResource};
use oeuvre_app::{AppState, Command, Outcome};
use oeuvre_core::{
    BibliographicDate, CreatorRole, CreatorSpec, NewResource, NonEmptyStr, PersonId,
    PublishedRange, Resource, ResourceFilter, ResourceId, ResourceKind, ResourceQuery,
    ResourceSummary, SeriesId, UtcMillis,
};

#[derive(Debug, Args)]
pub struct ResourceCmd {
    #[command(subcommand)]
    pub action: ResourceAction,
}

#[derive(Debug, Subcommand)]
pub enum ResourceAction {
    /// List resources as summary rows, newest first; every set filter
    /// flag narrows the same listing additively.
    List(ListArgs),
    /// Update a resource by id. Only the fields whose flags are set change;
    /// the rest keep their stored values.
    Update(UpdateArgs),
}

/// Filter flags for `resource list`.
#[derive(Debug, Args)]
pub struct ListArgs {
    /// Keep rows whose title contains this text, case-insensitively.
    #[arg(long)]
    pub title: Option<String>,

    /// Keep rows created by this person id.
    #[arg(long)]
    pub creator: Option<String>,

    /// Keep rows published in this year or later.
    #[arg(long)]
    pub published_from: Option<u16>,

    /// Keep rows published in this year or earlier.
    #[arg(long)]
    pub published_to: Option<u16>,
}

/// Field flags for `resource update`: each set flag replaces that field,
/// the rest keep their stored values. A `--clear-*` flag empties its field.
#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Resource id.
    pub id: String,

    /// New kind as its slug, e.g. `book`, `journalArticle`.
    #[arg(long)]
    pub kind: Option<String>,

    /// New title.
    #[arg(long)]
    pub title: Option<String>,

    /// New publisher.
    #[arg(long)]
    pub publisher: Option<String>,

    /// Set publisher to empty.
    #[arg(long, conflicts_with = "publisher")]
    pub clear_publisher: bool,

    /// New published year; keys the year's January 1 UTC instant.
    #[arg(long)]
    pub published_year: Option<u16>,

    /// Set published to empty.
    #[arg(long, conflicts_with = "published_year")]
    pub clear_published: bool,

    /// New accessed year; keys the year's January 1 UTC instant.
    #[arg(long)]
    pub accessed_year: Option<u16>,

    /// Set accessed to empty.
    #[arg(long, conflicts_with = "accessed_year")]
    pub clear_accessed: bool,

    /// New parent resource id.
    #[arg(long)]
    pub parent: Option<String>,

    /// Remove the parent link.
    #[arg(long, conflicts_with = "parent")]
    pub clear_parent: bool,

    /// New series id.
    #[arg(long)]
    pub series: Option<String>,

    /// Remove the series link (also pass --clear-series-number when one is
    /// stored, or the update is rejected).
    #[arg(long, conflicts_with = "series")]
    pub clear_series: bool,

    /// New series number.
    #[arg(long)]
    pub series_number: Option<String>,

    /// Set series number to empty.
    #[arg(long, conflicts_with = "series_number")]
    pub clear_series_number: bool,

    /// Replace the creator list, one per creator: `personId:role`, with an
    /// optional `="Credited as"` display. Roles: author, editor, translator,
    /// contributor, host, director.
    #[arg(long = "creator")]
    pub creators: Vec<String>,

    /// Set the creator list to empty. Containers accept that; other kinds
    /// are rejected by the update itself.
    #[arg(long, conflicts_with = "creators")]
    pub clear_creators: bool,
}

impl UpdateArgs {
    /// True when any field flag is set, so the update has work to do.
    pub fn has_changes(&self) -> bool {
        self.kind.is_some()
            || self.title.is_some()
            || self.publisher.is_some()
            || self.clear_publisher
            || self.published_year.is_some()
            || self.clear_published
            || self.accessed_year.is_some()
            || self.clear_accessed
            || self.parent.is_some()
            || self.clear_parent
            || self.series.is_some()
            || self.clear_series
            || self.series_number.is_some()
            || self.clear_series_number
            || !self.creators.is_empty()
            || self.clear_creators
    }
}

/// Rows per query-page fetch while walking a filtered listing.
const QUERY_PAGE: u32 = 500;

pub async fn run(state: &AppState, cmd: ResourceCmd) -> Result<()> {
    match cmd.action {
        ResourceAction::List(args) => list(state, args).await,
        ResourceAction::Update(args) => update(state, args).await,
    }
}

/// The listing the flags compose to: no flag keeps the plain newest-first
/// listing, any set flag narrows it additively through the query seam.
fn listing_query(args: &ListArgs) -> Result<Option<ResourceQuery>> {
    let title = match args.title.as_deref() {
        Some(text) => {
            Some(NonEmptyStr::try_new(text).map_err(|_| anyhow!("--title must not be empty"))?)
        }
        None => None,
    };
    let creator = match args.creator.as_deref() {
        Some(text) => Some(PersonId::parse(text).context("--creator must be a person id")?),
        None => None,
    };
    // Years bracket whole years: the from year's January 1 is inclusive,
    // the to year's next January 1 exclusive.
    let published = match (args.published_from, args.published_to) {
        (None, None) => None,
        (from, to) => Some(PublishedRange {
            from: year_bound(from, 0)?,
            to: year_bound(to, 1)?,
        }),
    };
    if title.is_none() && creator.is_none() && published.is_none() {
        return Ok(None);
    }
    Ok(Some(ResourceQuery {
        sort: None,
        filter: ResourceFilter {
            search: None,
            title,
            creator,
            published,
            kind: None,
            favorite: None,
            tag: None,
            collection: None,
        },
    }))
}

/// One end of a published window; `shift` turns the inclusive to-year into
/// its exclusive next-January-1 bound.
fn year_bound(year: Option<u16>, shift: i64) -> Result<Option<UtcMillis>> {
    year.map(|year| UtcMillis::jan_1_utc(i64::from(year) + shift))
        .map(|instant| instant.context("year outside the calendar"))
        .transpose()
}

async fn list(state: &AppState, args: ListArgs) -> Result<()> {
    let rows = match listing_query(&args)? {
        // Unfiltered keeps the plain listing: the same rows through the
        // command the library already had.
        None => {
            let Outcome::ListResources(rows) =
                state.execute(Command::ListResources(ListResources)).await?
            else {
                bail!("unexpected outcome from list_resources");
            };
            rows
        }
        // The query seam pages by offset; the CLI lists every match, so
        // walk the pages until the window ends.
        Some(query) => {
            let mut rows = Vec::new();
            let mut offset = 0u64;
            loop {
                let Outcome::ListResourcesQueryPage(page) = state
                    .execute(Command::ListResourcesQueryPage(ListResourcesQueryPage {
                        query: query.clone(),
                        offset,
                        limit: QUERY_PAGE,
                    }))
                    .await?
                else {
                    bail!("unexpected outcome from list_resources_query_page");
                };
                rows.extend(page.rows);
                match page.next_offset {
                    Some(next) => offset = next,
                    None => break,
                }
            }
            rows
        }
    };

    if rows.is_empty() {
        println!("(none)");
        return Ok(());
    }
    for row in &rows {
        println!("{}", format_summary_row(row));
    }
    Ok(())
}

/// Full-replace edit: fetch the record, merge the set flags into its
/// write-side twin, send it back. Unchanged fields ride through untouched,
/// the same merge `person update` performs.
async fn update(state: &AppState, args: UpdateArgs) -> Result<()> {
    let id = ResourceId::parse(&args.id)
        .with_context(|| format!("invalid resource id '{}'", args.id))?;

    let Outcome::GetResource(existing) = state
        .execute(Command::GetResource(GetResource { id }))
        .await?
    else {
        bail!("unexpected outcome from get_resource");
    };

    let new = merge_update(&existing, &args)?;

    let Outcome::UpdateResource(updated) = state
        .execute(Command::UpdateResource(UpdateResource { id, new }))
        .await?
    else {
        bail!("unexpected outcome from update_resource");
    };
    println!(
        "{}  {}  {}",
        updated.id,
        updated.kind.as_str(),
        updated.title.as_str()
    );
    Ok(())
}

/// Merge the set flags over the stored record's write-side twin: each set
/// flag replaces its field, each `--clear-*` empties it, and untouched
/// fields ride through as stored. Validation of the merged record belongs
/// to the command (reference existence, parent rules, series invariants).
fn merge_update(existing: &Resource, args: &UpdateArgs) -> Result<NewResource> {
    if !args.has_changes() {
        bail!("nothing to update; pass a field flag");
    }
    let mut edit = existing.to_new();

    if let Some(kind) = &args.kind {
        edit.kind =
            ResourceKind::from_slug(kind).with_context(|| format!("unknown --kind '{kind}'"))?;
    }
    if let Some(title) = &args.title {
        edit.title = NonEmptyStr::try_new(title.clone())
            .map_err(|_| anyhow!("--title must not be empty"))?;
    }
    if let Some(publisher) = &args.publisher {
        edit.publisher = Some(publisher.clone());
    }
    if args.clear_publisher {
        edit.publisher = None;
    }
    if let Some(year) = args.published_year {
        edit.published = Some(year_date("--published-year", year)?);
    }
    if args.clear_published {
        edit.published = None;
    }
    if let Some(year) = args.accessed_year {
        edit.accessed = Some(year_date("--accessed-year", year)?);
    }
    if args.clear_accessed {
        edit.accessed = None;
    }
    if let Some(parent) = &args.parent {
        edit.parent_id = Some(
            ResourceId::parse(parent)
                .with_context(|| format!("--parent must be a resource id, got '{parent}'"))?,
        );
    }
    if args.clear_parent {
        edit.parent_id = None;
    }
    if let Some(series) = &args.series {
        edit.series_id = Some(
            SeriesId::parse(series)
                .with_context(|| format!("--series must be a series id, got '{series}'"))?,
        );
    }
    if args.clear_series {
        edit.series_id = None;
    }
    if let Some(number) = &args.series_number {
        edit.series_number = Some(
            NonEmptyStr::try_new(number.clone())
                .map_err(|_| anyhow!("--series-number must not be empty"))?,
        );
    }
    if args.clear_series_number {
        edit.series_number = None;
    }
    if !args.creators.is_empty() {
        edit.creators = parse_creator_specs(&args.creators)?;
    }
    if args.clear_creators {
        edit.creators = Vec::new();
    }

    Ok(edit)
}

/// Parse `--creator` values into the replacement list. Each value is
/// `personId:role`, optionally followed by `="Credited as"`; flag order is
/// the bibliography order.
fn parse_creator_specs(values: &[String]) -> Result<Vec<CreatorSpec>> {
    let mut specs = Vec::with_capacity(values.len());
    for value in values {
        // The optional credited display splits off first, so an '=' meant
        // for the credit can never be eaten by the id or role.
        let (slot, credited) = match value.split_once('=') {
            Some((slot, credited)) => (slot, Some(credited)),
            None => (value.as_str(), None),
        };
        let (id, role) = slot
            .split_once(':')
            .with_context(|| format!("--creator must be 'personId:role', got '{value}'"))?;
        let person_id = PersonId::parse(id)
            .with_context(|| format!("--creator person id '{id}' is not a person id"))?;
        let role = CreatorRole::from_slug(role)
            .with_context(|| format!("--creator role '{role}' is not a known role"))?;
        let credited_as = credited
            .map(|c| {
                // An optional surrounding quote pair keeps
                // `id:role="Credited as"` shell-friendly; a bare credit
                // works the same without the quotes.
                let c = if c.len() >= 2 && c.starts_with('"') && c.ends_with('"') {
                    &c[1..c.len() - 1]
                } else {
                    c
                };
                NonEmptyStr::try_new(c)
            })
            .transpose()
            .map_err(|_| anyhow!("--creator credited name must not be empty"))?;
        specs.push(CreatorSpec {
            person_id,
            role,
            credited_as,
        });
    }
    Ok(specs)
}

/// A year flag's date: the year's January 1 UTC instant, no fuzzy display.
fn year_date(flag: &str, year: u16) -> Result<BibliographicDate> {
    let instant = UtcMillis::jan_1_utc(i64::from(year))
        .with_context(|| format!("{flag} outside the calendar"))?;
    BibliographicDate::try_new(instant, None).map_err(|_| anyhow!("invalid date from {flag}"))
}

/// One line per summary: `id  kind  title  creators  (year)  in container`,
/// omitting fields the row does not carry. Publisher stays out of the row
/// to keep lines readable; `resource get` will surface it later.
fn format_summary_row(row: &ResourceSummary) -> String {
    let mut out = format!("{}  {}  {}", row.id, row.kind.as_str(), row.title.as_str());
    if !row.creators_label.is_empty() {
        out.push_str("  ");
        out.push_str(&row.creators_label);
    }
    if let Some(year) = &row.published_label {
        out.push_str("  (");
        out.push_str(year.as_str());
        out.push(')');
    }
    if let Some(container) = &row.container_title {
        out.push_str("  in ");
        out.push_str(container.as_str());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oeuvre_core::{
        CreatorRole, CreatorSpec, NonEmptyStr, PersonId, ResourceId, ResourceKind, UtcMillis,
    };

    fn summary(
        title: &str,
        creators: &str,
        published: Option<&str>,
        container: Option<&str>,
    ) -> ResourceSummary {
        ResourceSummary {
            id: ResourceId::generate(),
            kind: ResourceKind::JournalArticle,
            title: NonEmptyStr::try_new(title).expect("title non-empty"),
            creators_label: creators.to_owned(),
            published_label: published
                .map(|p| NonEmptyStr::try_new(p).expect("published non-empty")),
            publisher: None,
            container_title: container
                .map(|c| NonEmptyStr::try_new(c).expect("container non-empty")),
            date_added: UtcMillis::from_unix_ms(0),
            date_edited: UtcMillis::from_unix_ms(0),
        }
    }

    #[test]
    fn formats_full_row() {
        let row = summary(
            "Quantum bibliometrics",
            "Ada; Grace",
            Some("2024"),
            Some("Oeuvre Journal"),
        );
        assert_eq!(
            format_summary_row(&row),
            format!(
                "{}  journalArticle  Quantum bibliometrics  Ada; Grace  (2024)  in Oeuvre Journal",
                row.id
            )
        );
    }

    #[test]
    fn omits_absent_fields() {
        let row = summary("A Lone Title", "", None, None);
        assert_eq!(
            format_summary_row(&row),
            format!("{}  journalArticle  A Lone Title", row.id)
        );
    }

    #[test]
    fn omits_empty_creators_but_keeps_year() {
        let row = summary("Solo Work", "", Some("2020"), None);
        assert_eq!(
            format_summary_row(&row),
            format!("{}  journalArticle  Solo Work  (2020)", row.id)
        );
    }

    fn list_args(
        title: Option<&str>,
        creator: Option<&str>,
        published_from: Option<u16>,
        published_to: Option<u16>,
    ) -> ListArgs {
        ListArgs {
            title: title.map(str::to_owned),
            creator: creator.map(str::to_owned),
            published_from,
            published_to,
        }
    }

    #[test]
    fn no_flags_keep_the_plain_listing() {
        assert!(
            listing_query(&list_args(None, None, None, None))
                .expect("no flags parse")
                .is_none()
        );
    }

    #[test]
    fn title_and_creator_flags_narrow_additively() {
        let id = PersonId::generate();
        let query = listing_query(&list_args(
            Some("  MOD "),
            Some(&id.to_string()),
            None,
            None,
        ))
        .expect("flags parse")
        .expect("some filter set");
        // The trimmed needle rides the store's case-insensitive contract.
        assert_eq!(
            query.filter.title.as_ref().map(|needle| needle.as_str()),
            Some("MOD")
        );
        assert_eq!(query.filter.creator, Some(id));
        assert_eq!(query.filter.published, None);
        assert_eq!(query.sort, None);
    }

    #[test]
    fn year_flags_bracket_whole_years() {
        let query = listing_query(&list_args(None, None, Some(1962), Some(1963)))
            .expect("years parse")
            .expect("some filter set");
        let range = query.filter.published.expect("range set");
        assert_eq!(range.from, UtcMillis::jan_1_utc(1962));
        // The to year lands whole: its next January 1 is exclusive.
        assert_eq!(range.to, UtcMillis::jan_1_utc(1964));

        // Either year alone still bounds the listing.
        let from_only = listing_query(&list_args(None, None, Some(1984), None))
            .expect("year parses")
            .expect("some filter set");
        let from_range = from_only.filter.published.expect("range set");
        assert_eq!(from_range.from, UtcMillis::jan_1_utc(1984));
        assert_eq!(from_range.to, None);
    }

    #[test]
    fn empty_title_and_unknown_creator_reject() {
        let err = listing_query(&list_args(Some("   "), None, None, None))
            .expect_err("blank title rejects");
        assert!(err.to_string().contains("--title"));
        let err = listing_query(&list_args(None, Some("not-an-id"), None, None))
            .expect_err("bad creator id rejects");
        assert!(err.to_string().contains("--creator"));
    }

    fn update_args() -> UpdateArgs {
        UpdateArgs {
            id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_owned(),
            kind: None,
            title: None,
            publisher: None,
            clear_publisher: false,
            published_year: None,
            clear_published: false,
            accessed_year: None,
            clear_accessed: false,
            parent: None,
            clear_parent: false,
            series: None,
            clear_series: false,
            series_number: None,
            clear_series_number: false,
            creators: Vec::new(),
            clear_creators: false,
        }
    }

    fn existing_book() -> Resource {
        let author = PersonId::generate();
        Resource::try_new(
            ResourceId::generate(),
            NewResource {
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new("Confessions").expect("title non-empty"),
                creators: vec![CreatorSpec {
                    person_id: author,
                    role: CreatorRole::Author,
                    credited_as: None,
                }],
                publisher: Some("Harvard".into()),
                published: Some(
                    BibliographicDate::try_new(
                        UtcMillis::jan_1_utc(1991).expect("year in calendar"),
                        None,
                    )
                    .expect("date valid"),
                ),
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
                tag_ids: vec![],
                collection_ids: vec![],
                metadata: Default::default(),
            },
        )
        .expect("book valid")
    }

    #[test]
    fn update_with_no_flags_rejects() {
        let err = merge_update(&existing_book(), &update_args()).expect_err("no flags reject");
        assert!(err.to_string().contains("nothing to update"));
    }

    #[test]
    fn update_title_flag_replaces_only_title() {
        let mut args = update_args();
        args.title = Some("Retitled".into());
        let merged = merge_update(&existing_book(), &args).expect("title merges");
        assert_eq!(merged.title.as_str(), "Retitled");
        assert_eq!(merged.kind, ResourceKind::Book);
        assert_eq!(merged.publisher.as_deref(), Some("Harvard"));
        assert_eq!(
            merged.published.expect("published rides through").instant(),
            UtcMillis::jan_1_utc(1991).expect("year in calendar")
        );
        assert_eq!(merged.creators.len(), 1);
    }

    #[test]
    fn update_kind_flag_parses_slug_and_unknown_rejects() {
        let mut args = update_args();
        args.kind = Some("journalArticle".into());
        let merged = merge_update(&existing_book(), &args).expect("kind merges");
        assert_eq!(merged.kind, ResourceKind::JournalArticle);

        args.kind = Some("novel".into());
        let err = merge_update(&existing_book(), &args).expect_err("unknown kind rejects");
        assert!(err.to_string().contains("unknown --kind 'novel'"));
    }

    #[test]
    fn update_clear_flags_empty_their_fields() {
        let mut args = update_args();
        args.clear_publisher = true;
        args.clear_published = true;
        let merged = merge_update(&existing_book(), &args).expect("clears merge");
        assert_eq!(merged.publisher, None);
        assert_eq!(merged.published, None);
        // Untouched fields ride through.
        assert_eq!(merged.title.as_str(), "Confessions");
    }

    #[test]
    fn update_year_flags_key_january_first() {
        let mut args = update_args();
        args.published_year = Some(2021);
        args.accessed_year = Some(2024);
        let merged = merge_update(&existing_book(), &args).expect("years merge");
        assert_eq!(
            merged.published.expect("published set").instant(),
            UtcMillis::jan_1_utc(2021).expect("year in calendar")
        );
        assert_eq!(
            merged.accessed.expect("accessed set").instant(),
            UtcMillis::jan_1_utc(2024).expect("year in calendar")
        );
    }

    #[test]
    fn update_id_flags_parse_and_bad_ids_reject() {
        let mut args = update_args();
        args.parent = Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".into());
        args.series = Some("01ARZ3NDEKTSV4RRFFQ69G5FAW".into());
        let merged = merge_update(&existing_book(), &args).expect("ids merge");
        assert_eq!(
            merged.parent_id,
            ResourceId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").ok()
        );
        assert_eq!(
            merged.series_id,
            SeriesId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAW").ok()
        );

        args.parent = Some("nope".into());
        let err = merge_update(&existing_book(), &args).expect_err("bad parent rejects");
        assert!(err.to_string().contains("--parent"));

        args.parent = None;
        args.series = Some("nope".into());
        let err = merge_update(&existing_book(), &args).expect_err("bad series rejects");
        assert!(err.to_string().contains("--series"));
    }

    #[test]
    fn update_series_number_merge_keeps_series_id_absent() {
        let mut args = update_args();
        args.series_number = Some("LCL 26".into());
        let merged = merge_update(&existing_book(), &args).expect("number merges");
        assert_eq!(
            merged.series_number.as_ref().map(|n| n.as_str()),
            Some("LCL 26")
        );
        // A number without a series is the command's call to reject, not the
        // merge's; the payload carries it through for that rejection.
        assert_eq!(merged.series_id, None);
    }

    #[test]
    fn update_creator_flags_build_the_replacement_list() {
        let author = PersonId::generate().to_string();
        let editor = PersonId::generate().to_string();
        let mut args = update_args();
        args.creators = vec![
            format!("{author}:author"),
            format!("{editor}:editor=\"H. Chadwick\""),
        ];
        let merged = merge_update(&existing_book(), &args).expect("creators merge");
        assert_eq!(merged.creators.len(), 2);
        assert_eq!(merged.creators[0].person_id.to_string(), author);
        assert_eq!(merged.creators[0].role, CreatorRole::Author);
        assert_eq!(merged.creators[0].credited_as, None);
        assert_eq!(merged.creators[1].person_id.to_string(), editor);
        assert_eq!(merged.creators[1].role, CreatorRole::Editor);
        assert_eq!(
            merged.creators[1].credited_as.as_ref().map(|c| c.as_str()),
            Some("H. Chadwick")
        );
    }

    #[test]
    fn update_creator_flags_reject_bad_values() {
        let author = PersonId::generate().to_string();
        let mut args = update_args();

        args.creators = vec!["nope:author".into()];
        let err = merge_update(&existing_book(), &args).expect_err("bad person id rejects");
        assert!(err.to_string().contains("--creator person id"));

        args.creators = vec![format!("{author}:publisher")];
        let err = merge_update(&existing_book(), &args).expect_err("bad role rejects");
        assert!(err.to_string().contains("--creator role"));

        args.creators = vec![author.clone()];
        let err = merge_update(&existing_book(), &args).expect_err("missing role rejects");
        assert!(err.to_string().contains("--creator must be"));

        args.creators = vec![format!("{author}:author=")];
        let err = merge_update(&existing_book(), &args).expect_err("empty credit rejects");
        assert!(err.to_string().contains("credited"));
    }

    #[test]
    fn update_clear_creators_empties_the_list() {
        let mut args = update_args();
        args.clear_creators = true;
        let merged = merge_update(&existing_book(), &args).expect("clear merges");
        assert!(merged.creators.is_empty());
    }
}
