use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand, ValueEnum};
use std::io::Write;
use std::path::PathBuf;

use ibex_app::command::{
    AddAttachment, CreateResource, GetResource, GetResourceDetail, ListNotes, ListResourceGridPage,
    ListResourcesPage, ListResourcesQueryPage, SetResourceFavorite, UpdateResource,
};
use ibex_app::{AppState, Command, Outcome};
use ibex_core::{
    BibliographicDate, CollectionId, CreatorRole, CreatorSpec, NewResource, NonEmptyStr, PersonId,
    PublishedRange, Resource, ResourceFilter, ResourceGridPage, ResourceGridParent,
    ResourceGridRow, ResourceId, ResourceKind, ResourceQuery, ResourceSort, ResourceSummary,
    SeriesId, SortDirection, TagId, UtcMillis, format_published_label, format_recorded_date,
};

use crate::note::NoteCmd;

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
    /// Show the complete, resolved metadata for one resource.
    Show { id: String },
    /// Create a resource with manually provided metadata.
    Add(AddArgs),
    /// Fetch ISBN/DOI metadata and store a new resource, or fill gaps on an existing one.
    Fetch {
        identifier: String,
        #[arg(long)]
        update: Option<String>,
    },
    /// Preview collection suggestions; --apply attaches them.
    Classify {
        id: String,
        #[arg(long, value_enum, default_value = "keywords")]
        engine: ClassifyEngine,
        #[arg(long)]
        apply: bool,
    },
    /// Update a resource by id. Only the fields whose flags are set change;
    /// the rest keep their stored values.
    Update(UpdateArgs),
    /// Manage Markdown notes linked to a resource.
    Note(NoteCmd),
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum ClassifyEngine {
    Keywords,
    Jev,
    Both,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum SortArg {
    Added,
    Edited,
    Title,
    Creator,
    Published,
    Publisher,
}

impl From<SortArg> for ResourceSort {
    fn from(value: SortArg) -> Self {
        match value {
            SortArg::Added => Self::DateAdded,
            SortArg::Edited => Self::DateEdited,
            SortArg::Title => Self::Title,
            SortArg::Creator => Self::PrimaryCreator,
            SortArg::Published => Self::Published,
            SortArg::Publisher => Self::Publisher,
        }
    }
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

    /// Search title and other indexed text.
    #[arg(long)]
    pub search: Option<String>,
    /// Filter by resource kind slug.
    #[arg(long)]
    pub kind: Option<String>,
    /// Only favorited resources.
    #[arg(long)]
    pub favorite: bool,
    /// Filter by tag id.
    #[arg(long)]
    pub tag: Option<String>,
    /// Filter by collection id.
    #[arg(long)]
    pub collection: Option<String>,

    /// Sort by a resource field (default direction: ascending).
    #[arg(long, value_enum)]
    pub sort: Option<SortArg>,
    /// Sort ascending; requires --sort.
    #[arg(long, requires = "sort", conflicts_with = "desc")]
    pub asc: bool,
    /// Sort descending; requires --sort.
    #[arg(long, requires = "sort", conflicts_with = "asc")]
    pub desc: bool,
    /// Display parent/child resource nesting.
    #[arg(long)]
    pub tree: bool,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    #[arg(long)]
    pub title: String,
    #[arg(long, default_value = "book")]
    pub kind: String,
    #[arg(long = "creator")]
    pub creators: Vec<String>,
    #[arg(long)]
    pub parent: Option<String>,
    #[arg(long)]
    pub publisher: Option<String>,
    #[arg(long)]
    pub published_year: Option<u16>,
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    #[arg(long = "collection")]
    pub collections: Vec<String>,
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

    /// Add this resource to favorites.
    #[arg(long, conflicts_with = "unfavorite")]
    pub favorite: bool,

    /// Remove this resource from favorites.
    #[arg(long, conflicts_with = "favorite")]
    pub unfavorite: bool,

    /// Attach a tag by id; repeatable.
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// Detach a tag by id; repeatable.
    #[arg(long = "remove-tag")]
    pub remove_tags: Vec<String>,
    /// Add a collection membership by id; repeatable.
    #[arg(long = "collection")]
    pub collections: Vec<String>,
    /// Remove a collection membership by id; repeatable.
    #[arg(long = "remove-collection")]
    pub remove_collections: Vec<String>,
    /// Attach a local file; repeatable.
    #[arg(long = "attach")]
    pub attachments: Vec<PathBuf>,
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
            || !self.tags.is_empty()
            || !self.remove_tags.is_empty()
            || !self.collections.is_empty()
            || !self.remove_collections.is_empty()
    }
}

/// Rows per query-page fetch while walking a filtered listing.
const QUERY_PAGE: u32 = 500;

pub async fn run(
    state: &AppState,
    cmd: ResourceCmd,
    config: &ibex_app::config::ConfigFile,
) -> Result<()> {
    match cmd.action {
        ResourceAction::List(args) => list(state, args).await,
        ResourceAction::Show { id } => show(state, &id).await,
        ResourceAction::Add(args) => add(state, args).await,
        ResourceAction::Fetch { identifier, update } => {
            fetch(state, &identifier, update.as_deref()).await
        }
        ResourceAction::Update(args) => update(state, args).await,
        ResourceAction::Classify { id, engine, apply } => {
            classify(state, config, &id, engine, apply).await
        }
        ResourceAction::Note(cmd) => crate::note::run(state, cmd).await,
    }
}

async fn classify(
    state: &AppState,
    config: &ibex_app::config::ConfigFile,
    id: &str,
    engine: ClassifyEngine,
    apply: bool,
) -> Result<()> {
    let id = ResourceId::parse(id).context("invalid resource id")?;
    let mut suggestions: Vec<(ibex_core::Collection, Option<f32>)> = Vec::new();
    if engine != ClassifyEngine::Keywords {
        let config = config.load()?;
        let key = config.get("openrouter_api_key").and_then(toml::Value::as_str)
            .filter(|key| !key.trim().is_empty())
            .context("Set an OpenRouter API key with `ibex config set openrouter-api-key` before Jev classification")?;
        let model = config
            .get("jev_model")
            .and_then(toml::Value::as_str)
            .unwrap_or(ibex_app::jev::DEFAULT_JEV_MODEL);
        let endpoint = std::env::var("IBEX_JEV_URL")
            .unwrap_or_else(|_| ibex_app::jev::DECISIONS_URL.to_owned());
        validate_jev_endpoint(&endpoint)?;
        let picks = ibex_app::classification::suggest_jev(state, id, key, model, &endpoint)
            .await
            .map_err(|message| anyhow!(message))?;
        suggestions.extend(
            picks
                .into_iter()
                .map(|(collection, probability)| (collection, Some(probability))),
        );
    }
    if engine != ClassifyEngine::Jev {
        for collection in ibex_app::classification::suggest_keywords(state, id).await? {
            if !suggestions
                .iter()
                .any(|(existing, _)| existing.id == collection.id)
            {
                suggestions.push((collection, None));
            }
        }
    }
    if suggestions.is_empty() {
        println!("(none)");
        return Ok(());
    }
    if apply {
        let collections = suggestions
            .iter()
            .map(|(collection, _)| collection.clone())
            .collect::<Vec<_>>();
        ibex_app::classification::apply_suggestions(state, id, &collections).await?;
    }
    for (collection, probability) in suggestions {
        if let Some(probability) = probability {
            println!(
                "{}  {}  {}%",
                collection.id,
                collection.name.as_str(),
                (probability * 100.).round() as u32
            );
        } else {
            println!("{}  {}", collection.id, collection.name.as_str());
        }
    }
    Ok(())
}

fn validate_jev_endpoint(endpoint: &str) -> Result<()> {
    // Keep the HTTP exception deliberately narrow: URL normalization must not
    // turn userinfo, escaped hosts, or backslashes into a remote destination.
    let safe = !endpoint
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
        && (endpoint.starts_with("https://")
            || endpoint.strip_prefix("http://").is_some_and(|rest| {
                let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
                if let Ok(address) = authority.parse::<std::net::SocketAddr>() {
                    return address.ip().is_loopback();
                }
                let host = match authority.rsplit_once(':') {
                    Some((host, port)) if port.parse::<u16>().is_ok() => host,
                    _ => authority,
                };
                host.eq_ignore_ascii_case("localhost")
                    || host == "[::1]"
                    || host
                        .parse::<std::net::Ipv4Addr>()
                        .is_ok_and(|address| address.is_loopback())
            }));
    if !safe {
        return Err(ibex_app::CommandError::Invalid(
            "IBEX_JEV_URL must use HTTPS, or HTTP with a loopback host (127.0.0.1, ::1, localhost)"
                .to_owned(),
        )
        .into());
    }
    Ok(())
}

async fn fetch(state: &AppState, identifier: &str, update: Option<&str>) -> Result<()> {
    let target = update
        .map(|id| ResourceId::parse(id).context("invalid --update resource id"))
        .transpose()?;
    let resource = ibex_app::metadata::fetch(state, identifier, target)
        .await
        .map_err(|message| anyhow!(message))?;
    println!(
        "{}  {}  {}",
        resource.id,
        resource.kind.as_str(),
        resource.title.as_str()
    );
    Ok(())
}

async fn add(state: &AppState, args: AddArgs) -> Result<()> {
    let title =
        NonEmptyStr::try_new(&args.title).map_err(|_| anyhow!("--title must not be empty"))?;
    let kind = ResourceKind::from_slug(&args.kind)
        .with_context(|| format!("unknown --kind '{}'", args.kind))?;
    let creators = parse_creator_specs(&args.creators)?;
    let parent_id = args
        .parent
        .as_deref()
        .map(ResourceId::parse)
        .transpose()
        .context("invalid --parent id")?;
    let tag_ids = args
        .tags
        .iter()
        .map(|id| TagId::parse(id).context("invalid --tag id"))
        .collect::<Result<Vec<_>>>()?;
    let collection_ids = args
        .collections
        .iter()
        .map(|id| CollectionId::parse(id).context("invalid --collection id"))
        .collect::<Result<Vec<_>>>()?;
    let published = args
        .published_year
        .map(|year| year_date("--published-year", year))
        .transpose()?;
    let new = NewResource {
        kind,
        title,
        creators,
        publisher: args.publisher,
        published,
        accessed: None,
        parent_id,
        series_id: None,
        series_number: None,
        tag_ids,
        collection_ids,
        metadata: Default::default(),
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
    };
    let Outcome::CreateResource(resource) = state
        .execute(Command::CreateResource(CreateResource { new }))
        .await?
    else {
        bail!("unexpected outcome from create_resource");
    };
    println!(
        "{}  {}  {}",
        resource.id,
        resource.kind.as_str(),
        resource.title.as_str()
    );
    Ok(())
}

async fn show(state: &AppState, id: &str) -> Result<()> {
    let id = ResourceId::parse(id).context("invalid resource id")?;
    let Outcome::GetResourceDetail(detail) = state
        .execute(Command::GetResourceDetail(GetResourceDetail { id }))
        .await?
    else {
        bail!("unexpected outcome from get_resource_detail");
    };
    let value = serde_json::to_value(&detail)?;
    let Some(fields) = value.as_object() else {
        bail!("unexpected resource detail shape");
    };
    for (field, value) in fields {
        let date = match field.as_str() {
            "published" => detail
                .published
                .as_ref()
                .map(|date| format_published_label(date).as_str().to_owned()),
            "accessed" => detail
                .accessed
                .as_ref()
                .map(|date| format_published_label(date).as_str().to_owned()),
            "original_date" => detail
                .original_date
                .as_ref()
                .map(|date| format_published_label(date).as_str().to_owned()),
            "event_date" => detail
                .event_date
                .as_ref()
                .map(|date| format_published_label(date).as_str().to_owned()),
            "date_added" => Some(format_recorded_date(detail.date_added)),
            "date_edited" => Some(format_recorded_date(detail.date_edited)),
            _ => None,
        };
        if let Some(date) = date {
            println!("{field}: {date}");
            continue;
        }
        if !value.is_null() && !value.as_array().is_some_and(Vec::is_empty) {
            println!(
                "{field}: {}",
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string())
            );
        }
    }
    let Outcome::ListNotes(notes) = state
        .execute(Command::ListNotes(ListNotes { resource_id: id }))
        .await?
    else {
        bail!("unexpected outcome from list_notes");
    };
    if !notes.is_empty() {
        println!("notes:");
        for note in notes {
            print!("  ");
            crate::note::print_note_summary(&note);
        }
    }
    Ok(())
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
    let search = args
        .search
        .as_deref()
        .map(|text| NonEmptyStr::try_new(text).map_err(|_| anyhow!("--search must not be empty")))
        .transpose()?;
    let kind = args
        .kind
        .as_deref()
        .map(|text| {
            ResourceKind::from_slug(text).with_context(|| format!("unknown --kind '{text}'"))
        })
        .transpose()?;
    let tag = args
        .tag
        .as_deref()
        .map(|id| TagId::parse(id).context("invalid --tag id"))
        .transpose()?;
    let collection = args
        .collection
        .as_deref()
        .map(|id| CollectionId::parse(id).context("invalid --collection id"))
        .transpose()?;
    if title.is_none()
        && creator.is_none()
        && published.is_none()
        && search.is_none()
        && kind.is_none()
        && !args.favorite
        && tag.is_none()
        && collection.is_none()
        && args.sort.is_none()
    {
        return Ok(None);
    }
    Ok(Some(ResourceQuery {
        sort: args.sort.map(|sort| {
            (
                sort.into(),
                if args.desc {
                    SortDirection::Descending
                } else {
                    SortDirection::Ascending
                },
            )
        }),
        filter: ResourceFilter {
            search,
            title,
            creator,
            published,
            kind,
            favorite: args.favorite.then_some(true),
            tag,
            collection,
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
    if args.tree {
        return list_tree(state, &args).await;
    }
    let mut out = std::io::BufWriter::new(std::io::stdout());
    let mut printed = false;
    match listing_query(&args)? {
        // Unfiltered walks the newest-first id cursor, so each page costs
        // the same no matter how deep into the library it starts.
        None => {
            let mut before = None;
            loop {
                let Outcome::ListResourcesPage(page) = state
                    .execute(Command::ListResourcesPage(ListResourcesPage {
                        before,
                        limit: QUERY_PAGE,
                    }))
                    .await?
                else {
                    bail!("unexpected outcome from list_resources_page");
                };
                for row in &page.rows {
                    writeln!(out, "{}", format_summary_row(row))?;
                    printed = true;
                }
                out.flush()?;
                match page.next_before {
                    Some(next) => before = Some(next),
                    None => break,
                }
            }
        }
        // Filtered and sorted listings only page by offset; rows still
        // print page by page instead of accumulating.
        Some(query) => {
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
                for row in &page.rows {
                    writeln!(out, "{}", format_summary_row(row))?;
                    printed = true;
                }
                out.flush()?;
                match page.next_offset {
                    Some(next) => offset = next,
                    None => break,
                }
            }
        }
    }
    if !printed {
        writeln!(out, "(none)")?;
    }
    out.flush()?;
    Ok(())
}

async fn grid_page(
    state: &AppState,
    parent: ResourceGridParent,
    query: &ResourceQuery,
    offset: u64,
) -> Result<ResourceGridPage> {
    let Outcome::ResourceGridPage(page) = state
        .execute(Command::ListResourceGridPage(ListResourceGridPage {
            parent,
            query: query.clone(),
            offset,
            limit: QUERY_PAGE,
        }))
        .await?
    else {
        bail!("unexpected outcome from list_resource_grid_page");
    };
    Ok(page)
}

async fn grid_rows(
    state: &AppState,
    parent: ResourceGridParent,
    query: &ResourceQuery,
) -> Result<Vec<ResourceGridRow>> {
    let mut rows = Vec::new();
    let mut offset = 0;
    loop {
        let page = grid_page(state, parent, query, offset).await?;
        rows.extend(page.rows);
        match page.next_offset {
            Some(next) => offset = next,
            None => break,
        }
    }
    Ok(rows)
}

/// Roots stream one page at a time, each subtree printed before the next
/// root. The grid seam lists one parent's children per call, so every row
/// with children still costs its own query.
async fn list_tree(state: &AppState, args: &ListArgs) -> Result<()> {
    let query = listing_query(args)?.unwrap_or_else(ResourceQuery::newest_first);
    let mut out = std::io::BufWriter::new(std::io::stdout());
    let mut printed = false;
    let mut seen = std::collections::HashSet::new();
    let mut offset = 0;
    loop {
        let page = grid_page(state, ResourceGridParent::Roots, &query, offset).await?;
        for root in page.rows {
            let mut stack = vec![(root, 0usize)];
            while let Some((row, depth)) = stack.pop() {
                if !seen.insert(row.summary.id) {
                    continue;
                }
                writeln!(
                    out,
                    "{}{}",
                    "  ".repeat(depth),
                    format_summary_row(&row.summary)
                )?;
                printed = true;
                if row.has_children {
                    let children =
                        grid_rows(state, ResourceGridParent::Children(row.summary.id), &query)
                            .await?;
                    stack.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
                }
            }
        }
        out.flush()?;
        match page.next_offset {
            Some(next) => offset = next,
            None => break,
        }
    }
    if !printed {
        writeln!(out, "(none)")?;
    }
    out.flush()?;
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

    if !args.has_changes() && !args.favorite && !args.unfavorite && args.attachments.is_empty() {
        bail!("nothing to update; pass a field flag");
    }
    let mut updated = existing;
    if args.has_changes() {
        let new = merge_update(&updated, &args)?;
        let Outcome::UpdateResource(resource) = state
            .execute(Command::UpdateResource(UpdateResource { id, new }))
            .await?
        else {
            bail!("unexpected outcome from update_resource");
        };
        updated = resource;
    }
    if args.favorite || args.unfavorite {
        let Outcome::SetResourceFavorite(resource) = state
            .execute(Command::SetResourceFavorite(SetResourceFavorite {
                id,
                favorite: args.favorite,
            }))
            .await?
        else {
            bail!("unexpected outcome from set_resource_favorite");
        };
        updated = resource;
    }
    for path in &args.attachments {
        let Outcome::AddAttachment(attachment) = state
            .execute(Command::AddAttachment(AddAttachment {
                resource_id: id,
                file_path: path.clone(),
            }))
            .await?
        else {
            bail!("unexpected outcome from add_attachment");
        };
        println!(
            "attachment  {}  {}  {}",
            attachment.id,
            attachment.attachment_type,
            attachment.file_path.display()
        );
    }
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
    let tags = args
        .tags
        .iter()
        .map(|id| TagId::parse(id).context("invalid --tag id"))
        .collect::<Result<Vec<_>>>()?;
    let removed_tags = args
        .remove_tags
        .iter()
        .map(|id| TagId::parse(id).context("invalid --remove-tag id"))
        .collect::<Result<Vec<_>>>()?;
    let collections = args
        .collections
        .iter()
        .map(|id| CollectionId::parse(id).context("invalid --collection id"))
        .collect::<Result<Vec<_>>>()?;
    let removed_collections = args
        .remove_collections
        .iter()
        .map(|id| CollectionId::parse(id).context("invalid --remove-collection id"))
        .collect::<Result<Vec<_>>>()?;
    if tags.iter().any(|id| removed_tags.contains(id))
        || collections
            .iter()
            .any(|id| removed_collections.contains(id))
    {
        bail!("cannot add and remove the same membership in one update");
    }
    edit.tag_ids.retain(|id| !removed_tags.contains(id));
    for id in tags {
        if !edit.tag_ids.contains(&id) {
            edit.tag_ids.push(id);
        }
    }
    edit.collection_ids
        .retain(|id| !removed_collections.contains(id));
    for id in collections {
        if !edit.collection_ids.contains(&id) {
            edit.collection_ids.push(id);
        }
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
    use ibex_core::{
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
            Some("Ibex Journal"),
        );
        assert_eq!(
            format_summary_row(&row),
            format!(
                "{}  journalArticle  Quantum bibliometrics  Ada; Grace  (2024)  in Ibex Journal",
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
            search: None,
            kind: None,
            favorite: false,
            tag: None,
            collection: None,
            sort: None,
            asc: false,
            desc: false,
            tree: false,
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

    #[test]
    fn jev_endpoint_accepts_https_and_loopback_http_only() {
        for endpoint in [
            "https://openrouter.ai/api/alpha/decisions",
            "http://127.0.0.1:8080/api/alpha/decisions",
            "http://localhost/api",
            "http://LOCALHOST:9/api",
            "http://[::1]:8080/api",
        ] {
            assert!(validate_jev_endpoint(endpoint).is_ok(), "{endpoint}");
        }
        for endpoint in [
            "http://example.com/api",
            "http://127.0.0.1.example.com/api",
            "http://localhost@example.com/api",
            "http://localhost.example.com/api",
            "http://10.0.0.1/api",
            "ftp://127.0.0.1/api",
            "http://127.0.0.1\\@example.com/api",
        ] {
            let err = validate_jev_endpoint(endpoint).expect_err(endpoint);
            assert!(
                err.downcast_ref::<ibex_app::CommandError>()
                    .is_some_and(|cmd| cmd.kind() == ibex_app::ErrorKind::Invalid),
                "{endpoint}"
            );
        }
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
            tags: vec![],
            remove_tags: vec![],
            collections: vec![],
            remove_collections: vec![],
            attachments: vec![],
            favorite: false,
            unfavorite: false,
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
