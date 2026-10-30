//! Ibex application runtime.
//!
//! Closed commands, one `AppState::execute` entry point, and exactly-once wide events.

mod app;
pub mod classification;
pub mod command;
mod event;
pub mod import;
pub mod jev;
pub mod metadata;
pub mod seed;
#[cfg(feature = "testing")]
pub mod testing;

pub use app::{AppState, AppStateBuilder, Surface};
pub use command::{Call, Command, CommandError, ErrorKind, Outcome};
pub use event::{EventSink, ExecutionId, FieldValue, FinishedEvent, JsonLinesSink, Status};
pub use ibex_core::{Store, StoreError, StoreHealth};

pub(crate) use command::Handler;
