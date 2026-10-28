//! Shared bibliography import: format parsers and save orchestration.
//!
//! Parsers produce [`oeuvre_core::ImportDraft`] values with no store access.
//! [`preview_draft`] / [`save_draft`] own matching and persistence for every format.

mod biblatex;
mod csl_json;
mod link;
mod preview;
mod save;

pub use biblatex::{ParseError, parse_biblatex};
pub use csl_json::parse_csl_json;
pub use preview::preview_draft;
pub use save::{save_draft, save_draft_with_choices, save_draft_with_hooks, ImportHooks};
pub(crate) use save::save_draft_inner;
