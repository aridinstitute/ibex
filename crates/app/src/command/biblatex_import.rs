//! Compatibility command: BibLaTeX parse + shared import save.
//!
//! Parsing lives in [`crate::import::parse_biblatex`]; persistence in
//! [`crate::import::save_draft`]. Preview and choice-aware save are available
//! for review flows (Task 06).

use oeuvre_core::{ImportChoices, ImportPreview, ImportReport};

use crate::event::Event;
use crate::import::{parse_biblatex, preview_draft, save_draft_inner, save_draft_with_choices};
use crate::{AppState, CommandError, Handler, Outcome};

pub use oeuvre_core::{ImportIssue, ImportedEntry};

#[derive(Debug, Clone)]
pub struct ImportBiblatex {
    pub source_text: String,
    /// Optional review decisions; empty means automatic policy.
    pub choices: ImportChoices,
}

impl ImportBiblatex {
    pub fn new(source_text: impl Into<String>) -> Self {
        Self {
            source_text: source_text.into(),
            choices: ImportChoices::default(),
        }
    }
}

pub(crate) struct Imported(pub ImportReport);

impl From<Imported> for Outcome {
    fn from(Imported(report): Imported) -> Self {
        Outcome::ImportBiblatex(report)
    }
}

#[async_trait::async_trait]
impl Handler for ImportBiblatex {
    const NAME: &'static str = "import_biblatex";
    type Output = Imported;

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Imported, CommandError> {
        let draft = parse_biblatex(&self.source_text).map_err(|e| CommandError::Invalid(e.message))?;
        let report = save_draft_inner(state, draft, self.choices, Some(ev)).await?;
        Ok(Imported(report))
    }
}

/// Preview a BibLaTeX source without writing.
#[derive(Debug, Clone)]
pub struct PreviewBiblatex {
    pub source_text: String,
}

pub(crate) struct Previewed(pub ImportPreview);

impl From<Previewed> for Outcome {
    fn from(Previewed(preview): Previewed) -> Self {
        Outcome::PreviewBiblatex(preview)
    }
}

#[async_trait::async_trait]
impl Handler for PreviewBiblatex {
    const NAME: &'static str = "preview_biblatex";
    type Output = Previewed;

    async fn run(self, state: &AppState, _ev: &mut Event) -> Result<Previewed, CommandError> {
        let draft = parse_biblatex(&self.source_text).map_err(|e| CommandError::Invalid(e.message))?;
        let preview = preview_draft(state, draft).await?;
        Ok(Previewed(preview))
    }
}

/// Save an already-parsed draft with review choices (shared by formats).
#[derive(Debug, Clone)]
pub struct SaveImportDraft {
    pub draft: oeuvre_core::ImportDraft,
    pub choices: ImportChoices,
}

pub(crate) struct SavedImport(pub ImportReport);

impl From<SavedImport> for Outcome {
    fn from(SavedImport(report): SavedImport) -> Self {
        Outcome::SaveImportDraft(report)
    }
}

#[async_trait::async_trait]
impl Handler for SaveImportDraft {
    const NAME: &'static str = "save_import_draft";
    type Output = SavedImport;

    async fn run(self, state: &AppState, _ev: &mut Event) -> Result<SavedImport, CommandError> {
        let report = save_draft_with_choices(state, self.draft, self.choices).await?;
        Ok(SavedImport(report))
    }
}
