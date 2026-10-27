//! Compatibility command: BibLaTeX parse + shared import save.
//!
//! Parsing lives in [`crate::import::parse_biblatex`]; persistence in
//! [`crate::import::save_draft`]. Callers that already use this command keep
//! working while new formats feed [`crate::import::save_draft`] directly.

use oeuvre_core::ImportReport;

use crate::event::Event;
use crate::import::{parse_biblatex, save_draft_inner};
use crate::{AppState, CommandError, Handler, Outcome};

pub use oeuvre_core::{ImportIssue, ImportedEntry};

#[derive(Debug, Clone)]
pub struct ImportBiblatex {
    pub source_text: String,
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
        let report = save_draft_inner(state, draft, Some(ev)).await?;
        Ok(Imported(report))
    }
}
