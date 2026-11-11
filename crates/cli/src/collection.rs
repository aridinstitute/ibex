use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use ibex_app::cluster::{
    DEFAULT_KEYWORD_TERMS, DEFAULT_LABEL_TERMS, DEFAULT_MIN_TOPIC_SIZE, SuggestClusters,
    TextSource, apply_clusters, suggest_clusters,
};
use ibex_app::command::{
    CreateCollection, DeleteCollection, ListCollectionForest, UpdateCollection,
};
use ibex_app::{AppState, Command, Outcome};
use ibex_core::{CollectionId, CollectionTreeNode, CollectionUpdate, NonEmptyStr};

#[derive(Debug, Args)]
pub struct CollectionCmd {
    #[command(subcommand)]
    pub action: CollectionAction,
}

#[derive(Debug, Subcommand)]
pub enum CollectionAction {
    /// List collections as a tree; multi-parent collections appear under each parent.
    List {
        /// Show each collection once without tree indentation.
        #[arg(long)]
        flat: bool,
    },
    /// Show a collection and all its parent links.
    Show { id: String },
    /// Create a root collection or a child of an existing collection.
    Add {
        name: String,
        #[arg(long)]
        parent: Option<String>,
    },
    /// Atomically update fields and parent links.
    Update {
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long = "add-parent")]
        add_parents: Vec<String>,
        #[arg(long = "remove-parent")]
        remove_parents: Vec<String>,
        #[arg(long, conflicts_with = "clear_keywords")]
        keyword: Vec<String>,
        #[arg(long)]
        clear_keywords: bool,
        #[arg(long, conflicts_with = "no_jev")]
        jev: bool,
        #[arg(long)]
        no_jev: bool,
    },
    /// Cluster resources into topic collections (BerTopic-style).
    ///
    /// Prefers extractable attachment text; otherwise uses title, subtitle,
    /// abstract, and keywords. Preview by default; pass --apply to create
    /// collections and attach members.
    Cluster {
        /// Create collections and attach resources.
        #[arg(long)]
        apply: bool,
        /// Minimum resources required to keep a topic.
        #[arg(long, default_value_t = DEFAULT_MIN_TOPIC_SIZE)]
        min_size: usize,
        /// Optional parent collection for newly created topics.
        #[arg(long)]
        parent: Option<String>,
    },
    /// Delete a collection using the library's cascade rules.
    Delete { id: String },
}

pub async fn run(state: &AppState, cmd: CollectionCmd) -> Result<()> {
    if let CollectionAction::Cluster {
        apply,
        min_size,
        parent,
    } = &cmd.action
    {
        return cluster(state, *apply, *min_size, parent.as_deref()).await;
    }

    let Outcome::ListCollectionForest(forest) = state
        .execute(Command::ListCollectionForest(ListCollectionForest))
        .await?
    else {
        bail!("unexpected outcome from list_collection_forest");
    };
    match cmd.action {
        CollectionAction::Cluster { .. } => unreachable!("handled above"),
        CollectionAction::Delete { id } => {
            let id = CollectionId::parse(&id).context("invalid collection id")?;
            let Outcome::DeleteCollection(_) = state
                .execute(Command::DeleteCollection(DeleteCollection { id }))
                .await?
            else {
                bail!("unexpected outcome from delete_collection");
            };
            println!("deleted {id}");
        }
        CollectionAction::Add { name, parent } => {
            let name = NonEmptyStr::try_new(&name)?;
            let parent_id = parent
                .as_deref()
                .map(CollectionId::parse)
                .transpose()
                .context("invalid parent collection id")?;
            let Outcome::CreateCollection(collection) = state
                .execute(Command::CreateCollection(CreateCollection {
                    name,
                    parent_id,
                }))
                .await?
            else {
                bail!("unexpected outcome from create_collection");
            };
            println!("{}  {}", collection.id, collection.name.as_str());
        }
        CollectionAction::Update {
            id,
            name,
            add_parents,
            remove_parents,
            keyword,
            clear_keywords,
            jev,
            no_jev,
        } => {
            let id = CollectionId::parse(&id).context("invalid collection id")?;
            let parse_parents = |parents: Vec<String>| -> Result<Vec<CollectionId>> {
                parents
                    .iter()
                    .map(|s| CollectionId::parse(s).context("invalid parent collection id"))
                    .collect()
            };
            let update = CollectionUpdate {
                id,
                name: name.map(|name| NonEmptyStr::try_new(&name)).transpose()?,
                keywords: if clear_keywords {
                    Some(vec![])
                } else if keyword.is_empty() {
                    None
                } else {
                    Some(keyword)
                },
                jev_enabled: if jev {
                    Some(true)
                } else if no_jev {
                    Some(false)
                } else {
                    None
                },
                add_parents: parse_parents(add_parents)?,
                remove_parents: parse_parents(remove_parents)?,
            };
            let Outcome::UpdateCollection(collection) = state
                .execute(Command::UpdateCollection(UpdateCollection(update)))
                .await?
            else {
                bail!("unexpected outcome from update_collection");
            };
            println!("{}  {}", collection.id, collection.name.as_str());
        }
        CollectionAction::List { flat } => {
            if forest.is_empty() {
                println!("(none)");
            }
            let mut seen = HashSet::new();
            for root in &forest {
                print_node(root, 0, flat, &mut seen);
            }
        }
        CollectionAction::Show { id } => {
            let id = CollectionId::parse(&id).context("invalid collection id")?;
            let mut found = None;
            let mut parents = Vec::new();
            for root in &forest {
                find_with_parents(root, id, None, &mut found, &mut parents);
            }
            let Some(collection) = found else {
                bail!("collection {id} not found");
            };
            println!("id: {}", collection.id);
            println!("name: {}", collection.name.as_str());
            println!(
                "parents: {}",
                parents
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!("keywords: {}", collection.keywords.join(", "));
            println!("jev_enabled: {}", collection.jev_enabled);
        }
    }
    Ok(())
}

async fn cluster(
    state: &AppState,
    apply: bool,
    min_size: usize,
    parent: Option<&str>,
) -> Result<()> {
    let parent_id = parent
        .map(CollectionId::parse)
        .transpose()
        .context("invalid parent collection id")?;
    let suggestion = suggest_clusters(
        state,
        SuggestClusters {
            min_topic_size: min_size,
            label_terms: DEFAULT_LABEL_TERMS,
            keyword_terms: DEFAULT_KEYWORD_TERMS,
        },
    )
    .await?;

    if suggestion.report.topics.is_empty() && suggestion.report.outliers.is_empty() {
        println!("(none)");
        return Ok(());
    }

    if apply {
        let applied = apply_clusters(state, &suggestion, parent_id).await?;
        for (topic, created) in suggestion.report.topics.iter().zip(applied.iter()) {
            println!(
                "{}  {}  ({} resources)",
                created.collection.id,
                created.collection.name.as_str(),
                created.resource_ids.len()
            );
            for member in &topic.members {
                println!(
                    "  {}  {}  [{}]",
                    member.resource_id,
                    member.title,
                    source_label(&member.source)
                );
            }
        }
    } else {
        for (i, topic) in suggestion.report.topics.iter().enumerate() {
            println!(
                "topic {}  {}  ({} resources)",
                i + 1,
                topic.label,
                topic.members.len()
            );
            for member in &topic.members {
                println!(
                    "  {}  {}  [{}]",
                    member.resource_id,
                    member.title,
                    source_label(&member.source)
                );
            }
        }
    }

    if !suggestion.report.outliers.is_empty() {
        println!("outliers  ({} resources)", suggestion.report.outliers.len());
        for member in &suggestion.report.outliers {
            println!(
                "  {}  {}  [{}]",
                member.resource_id,
                member.title,
                source_label(&member.source)
            );
        }
    }

    if !apply && !suggestion.report.topics.is_empty() {
        eprintln!("preview only; pass --apply to create collections");
    }
    Ok(())
}

fn source_label(source: &TextSource) -> &'static str {
    match source {
        TextSource::Attachment => "attachment",
        TextSource::Metadata => "metadata",
    }
}

fn print_node(
    node: &CollectionTreeNode,
    depth: usize,
    flat: bool,
    seen: &mut HashSet<CollectionId>,
) {
    if !flat || seen.insert(node.collection.id) {
        let indent = if flat { 0 } else { depth * 2 };
        println!(
            "{}{}  {}",
            " ".repeat(indent),
            node.collection.id,
            node.collection.name.as_str()
        );
    }
    for child in &node.children {
        print_node(child, depth + 1, flat, seen);
    }
}

fn find_with_parents(
    node: &CollectionTreeNode,
    id: CollectionId,
    parent: Option<CollectionId>,
    found: &mut Option<ibex_core::Collection>,
    parents: &mut Vec<CollectionId>,
) {
    if node.collection.id == id {
        *found = Some(node.collection.clone());
        if let Some(parent) = parent
            && !parents.contains(&parent)
        {
            parents.push(parent);
        }
    }
    for child in &node.children {
        find_with_parents(child, id, Some(node.collection.id), found, parents);
    }
}
