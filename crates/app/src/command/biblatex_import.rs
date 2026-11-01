//! Compatibility commands: format parse + shared import save.
//!
//! Parsing lives in [`crate::import`]; persistence in [`crate::import::save_draft`].
//! Preview and choice-aware save are available for review flows (Task 06+).

use ibex_core::{ImportChoices, ImportPreview, ImportReport};

use crate::event::Event;
use crate::import::{
    ImportHooks, parse_biblatex, parse_csl_json, preview_draft, save_draft_inner,
    save_draft_with_choices,
};
use crate::{AppState, CommandError, Handler, Outcome};

pub use ibex_core::{ImportIssue, ImportedEntry};

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
        let draft =
            parse_biblatex(&self.source_text).map_err(|e| CommandError::Invalid(e.message))?;
        let report =
            save_draft_inner(state, draft, self.choices, Some(ev), ImportHooks::default()).await?;
        Ok(Imported(report))
    }
}

/// Import a CSL-JSON bibliography through the shared save path.
#[derive(Debug, Clone)]
pub struct ImportCslJson {
    pub source_text: String,
    pub choices: ImportChoices,
}

pub(crate) struct ImportedCsl(pub ImportReport);

impl From<ImportedCsl> for Outcome {
    fn from(ImportedCsl(report): ImportedCsl) -> Self {
        Outcome::ImportCslJson(report)
    }
}

#[async_trait::async_trait]
impl Handler for ImportCslJson {
    const NAME: &'static str = "import_csl_json";
    type Output = ImportedCsl;

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<ImportedCsl, CommandError> {
        let draft =
            parse_csl_json(&self.source_text).map_err(|e| CommandError::Invalid(e.message))?;
        let report =
            save_draft_inner(state, draft, self.choices, Some(ev), ImportHooks::default()).await?;
        Ok(ImportedCsl(report))
    }
}

/// Preview a CSL-JSON source without writing.
#[derive(Debug, Clone)]
pub struct PreviewCslJson {
    pub source_text: String,
}

pub(crate) struct PreviewedCsl(pub ImportPreview);

impl From<PreviewedCsl> for Outcome {
    fn from(PreviewedCsl(preview): PreviewedCsl) -> Self {
        Outcome::PreviewCslJson(preview)
    }
}

#[async_trait::async_trait]
impl Handler for PreviewCslJson {
    const NAME: &'static str = "preview_csl_json";
    type Output = PreviewedCsl;

    async fn run(self, state: &AppState, _ev: &mut Event) -> Result<PreviewedCsl, CommandError> {
        let draft =
            parse_csl_json(&self.source_text).map_err(|e| CommandError::Invalid(e.message))?;
        let preview = preview_draft(state, draft).await?;
        Ok(PreviewedCsl(preview))
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
        let draft =
            parse_biblatex(&self.source_text).map_err(|e| CommandError::Invalid(e.message))?;
        let preview = preview_draft(state, draft).await?;
        Ok(Previewed(preview))
    }
}

/// Save an already-parsed draft with review choices (shared by formats).
#[derive(Debug, Clone)]
pub struct SaveImportDraft {
    pub draft: ibex_core::ImportDraft,
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
