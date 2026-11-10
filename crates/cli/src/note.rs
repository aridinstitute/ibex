//! Resource-linked notes: list/add/update/delete under `resource note`.

use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};

use ibex_app::command::{CreateNote, DeleteNote, ListNotes, UpdateNote};
use ibex_app::{AppState, Command, Outcome};
use ibex_core::{CitationLocation, LocationKind, Note, NoteContent, NoteId, NoteKind, ResourceId};

#[derive(Debug, Args)]
pub struct NoteCmd {
    #[command(subcommand)]
    pub action: NoteAction,
}

#[derive(Debug, Subcommand)]
pub enum NoteAction {
    /// List notes for a resource, oldest first.
    List { resource: String },
    /// Create a note on a resource.
    Add(AddNoteArgs),
    /// Update a note; omitted flags keep the stored values.
    Update(UpdateNoteArgs),
    /// Delete a note by id.
    Delete { id: String },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum NoteKindArg {
    Note,
    Summary,
    Paraphrase,
    Quote,
}

impl From<NoteKindArg> for NoteKind {
    fn from(value: NoteKindArg) -> Self {
        match value {
            NoteKindArg::Note => Self::Note,
            NoteKindArg::Summary => Self::Summary,
            NoteKindArg::Paraphrase => Self::Paraphrase,
            NoteKindArg::Quote => Self::Quote,
        }
    }
}

#[derive(Debug, Args)]
pub struct AddNoteArgs {
    /// Resource that owns the note.
    pub resource: String,
    /// Note type (default: note).
    #[arg(long, value_enum, default_value = "note")]
    pub kind: NoteKindArg,
    /// Markdown body. Conflicts with --body-file; when neither is set, reads stdin.
    #[arg(long, conflicts_with = "body_file")]
    pub body: Option<String>,
    /// Read the Markdown body from a file.
    #[arg(long, conflicts_with = "body")]
    pub body_file: Option<PathBuf>,
    /// Citation location as `kind=value` (page, chapter, section, timestamp); repeatable.
    #[arg(long = "location")]
    pub locations: Vec<String>,
}

#[derive(Debug, Args)]
pub struct UpdateNoteArgs {
    /// Resource that owns the note (needed to load the current content).
    pub resource: String,
    /// Note id.
    pub id: String,
    /// New note type.
    #[arg(long, value_enum)]
    pub kind: Option<NoteKindArg>,
    /// Replace the Markdown body. Conflicts with --body-file.
    #[arg(long, conflicts_with = "body_file")]
    pub body: Option<String>,
    /// Replace the Markdown body from a file.
    #[arg(long, conflicts_with = "body")]
    pub body_file: Option<PathBuf>,
    /// Replace citation locations with these `kind=value` entries; repeatable.
    #[arg(long = "location", conflicts_with = "clear_locations")]
    pub locations: Vec<String>,
    /// Remove all citation locations.
    #[arg(long)]
    pub clear_locations: bool,
}

impl UpdateNoteArgs {
    fn has_changes(&self) -> bool {
        self.kind.is_some()
            || self.body.is_some()
            || self.body_file.is_some()
            || !self.locations.is_empty()
            || self.clear_locations
    }
}

pub async fn run(state: &AppState, cmd: NoteCmd) -> Result<()> {
    match cmd.action {
        NoteAction::List { resource } => list(state, &resource).await,
        NoteAction::Add(args) => add(state, args).await,
        NoteAction::Update(args) => update(state, args).await,
        NoteAction::Delete { id } => delete(state, &id).await,
    }
}

async fn list(state: &AppState, resource: &str) -> Result<()> {
    let resource_id = ResourceId::parse(resource).context("invalid resource id")?;
    let notes = list_notes(state, resource_id).await?;
    if notes.is_empty() {
        println!("(none)");
        return Ok(());
    }
    for note in notes {
        print_note_summary(&note);
    }
    Ok(())
}

async fn add(state: &AppState, args: AddNoteArgs) -> Result<()> {
    let resource_id = ResourceId::parse(&args.resource).context("invalid resource id")?;
    let body = read_body(args.body, args.body_file, true)?;
    let locations = parse_locations(&args.locations)?;
    let content = NoteContent {
        kind: args.kind.into(),
        body_markdown: body,
        locations,
    };
    let Outcome::CreateNote(note) = state
        .execute(Command::CreateNote(CreateNote {
            resource_id,
            content,
        }))
        .await?
    else {
        bail!("unexpected outcome from create_note");
    };
    print_note_summary(&note);
    Ok(())
}

async fn update(state: &AppState, args: UpdateNoteArgs) -> Result<()> {
    if !args.has_changes() {
        bail!("pass at least one of --kind, --body, --body-file, --location, or --clear-locations");
    }
    let resource_id = ResourceId::parse(&args.resource).context("invalid resource id")?;
    let note_id = NoteId::parse(&args.id).context("invalid note id")?;
    let notes = list_notes(state, resource_id).await?;
    let existing = notes
        .into_iter()
        .find(|note| note.id == note_id)
        .ok_or_else(|| ibex_app::CommandError::NotFound(note_id.to_string()))?;
    let mut content = existing.content;
    if let Some(kind) = args.kind {
        content.kind = kind.into();
    }
    if args.body.is_some() || args.body_file.is_some() {
        content.body_markdown = read_body(args.body, args.body_file, false)?;
    }
    if args.clear_locations {
        content.locations.clear();
    } else if !args.locations.is_empty() {
        content.locations = parse_locations(&args.locations)?;
    }
    let Outcome::UpdateNote(note) = state
        .execute(Command::UpdateNote(UpdateNote {
            id: note_id,
            content,
        }))
        .await?
    else {
        bail!("unexpected outcome from update_note");
    };
    print_note_summary(&note);
    Ok(())
}

async fn delete(state: &AppState, id: &str) -> Result<()> {
    let id = NoteId::parse(id).context("invalid note id")?;
    let Outcome::DeleteNote(()) = state
        .execute(Command::DeleteNote(DeleteNote { id }))
        .await?
    else {
        bail!("unexpected outcome from delete_note");
    };
    println!("deleted {id}");
    Ok(())
}

async fn list_notes(state: &AppState, resource_id: ResourceId) -> Result<Vec<Note>> {
    let Outcome::ListNotes(notes) = state
        .execute(Command::ListNotes(ListNotes { resource_id }))
        .await?
    else {
        bail!("unexpected outcome from list_notes");
    };
    Ok(notes)
}

fn read_body(body: Option<String>, body_file: Option<PathBuf>, required: bool) -> Result<String> {
    if let Some(text) = body {
        return Ok(text);
    }
    if let Some(path) = body_file {
        return std::fs::read_to_string(&path)
            .with_context(|| format!("read note body from {}", path.display()));
    }
    if !io::stdin().is_terminal() {
        let mut text = String::new();
        io::stdin().read_to_string(&mut text)?;
        return Ok(text);
    }
    if required {
        bail!("pass --body, --body-file, or pipe Markdown on stdin");
    }
    bail!("pass --body or --body-file to change the note body");
}

fn parse_locations(raw: &[String]) -> Result<Vec<CitationLocation>> {
    raw.iter().map(|entry| parse_location(entry)).collect()
}

fn parse_location(entry: &str) -> Result<CitationLocation> {
    let (kind, value) = entry
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("--location must be kind=value, got '{entry}'"))?;
    let kind = match kind.trim().to_ascii_lowercase().as_str() {
        "page" => LocationKind::Page,
        "chapter" => LocationKind::Chapter,
        "section" => LocationKind::Section,
        "timestamp" => LocationKind::Timestamp,
        other => bail!("unknown --location kind '{other}' (page, chapter, section, timestamp)"),
    };
    let value = value.trim();
    if value.is_empty() {
        bail!("--location value must not be empty");
    }
    Ok(CitationLocation {
        kind,
        value: value.to_owned(),
    })
}

pub(crate) fn print_note_summary(note: &Note) {
    let locations = if note.content.locations.is_empty() {
        String::new()
    } else {
        let joined = note
            .content
            .locations
            .iter()
            .map(|location| {
                format!(
                    "{}={}",
                    location.kind.label().to_ascii_lowercase(),
                    location.value
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("  [{joined}]")
    };
    let preview = first_line_preview(&note.content.body_markdown);
    println!(
        "{}  {}{}  {}",
        note.id,
        note.content.kind.label().to_ascii_lowercase(),
        locations,
        preview
    );
}

fn first_line_preview(body: &str) -> String {
    let line = body.lines().next().unwrap_or("").trim();
    const MAX: usize = 80;
    if line.chars().count() <= MAX {
        return line.to_owned();
    }
    let truncated: String = line.chars().take(MAX.saturating_sub(1)).collect();
    format!("{truncated}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_location_kinds() {
        let location = parse_location("page=12–15").expect("page");
        assert_eq!(location.kind, LocationKind::Page);
        assert_eq!(location.value, "12–15");
        let location = parse_location("Section=2.1").expect("section");
        assert_eq!(location.kind, LocationKind::Section);
        assert!(parse_location("folio=1").is_err());
        assert!(parse_location("page=").is_err());
        assert!(parse_location("page").is_err());
    }

    #[test]
    fn preview_truncates_long_first_line() {
        let long = "x".repeat(100);
        let preview = first_line_preview(&long);
        assert!(preview.ends_with('…'));
        assert_eq!(preview.chars().count(), 80);
    }
}
