//! Shared bibliography import: format parsers and save orchestration.
//!
//! Parsers produce [`oeuvre_core::ImportDraft`] values with no store access.
//! [`save_draft`] owns entity matching and persistence for every format.

mod biblatex;
mod save;

pub use biblatex::{ParseError, parse_biblatex};
pub use save::save_draft;
pub(crate) use save::save_draft_inner;
