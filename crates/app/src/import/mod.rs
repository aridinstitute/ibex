//! Shared bibliography import: format parsers and save orchestration.
//!
//! Parsers produce [`oeuvre_core::ImportDraft`] values with no store access.
//! [`preview_draft`] / [`save_draft`] own matching and persistence for every format.

mod biblatex;
mod link;
mod preview;
mod save;

pub use biblatex::{ParseError, parse_biblatex};
pub use preview::preview_draft;
pub use save::{save_draft, save_draft_with_choices};
pub(crate) use save::save_draft_inner;
