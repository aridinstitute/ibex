use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use oeuvre_app::command::{
    GetResource, ListResources, ListResourcesQueryPage, UpdateResource,
};
use oeuvre_app::{AppState, Command, Outcome};
use oeuvre_core::{
    NonEmptyStr, PersonId, PublishedRange, ResourceFilter, ResourceId, ResourceQuery,
    ResourceSummary, UtcMillis,
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
/// the rest keep their stored values.
#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Resource id.
    pub id: String,

    /// New title.
    #[arg(long)]
    pub title: Option<String>,
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
            title,
            creator,
            published,
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
            let Outcome::ListResources(rows) = state
                .execute(Command::ListResources(ListResources))
                .await?
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

    let Some(title) = args.title else {
        bail!("nothing to update; pass --title");
    };
    let mut edit = existing.to_new();
    edit.title = NonEmptyStr::try_new(title).map_err(|_| anyhow!("--title must not be empty"))?;

    let Outcome::UpdateResource(updated) = state
        .execute(Command::UpdateResource(UpdateResource { id, new: edit }))
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
        NonEmptyStr, PersonId, ResourceId, ResourceKind, UtcMillis,
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
        assert!(listing_query(&list_args(None, None, None, None))
            .expect("no flags parse")
            .is_none());
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
}
