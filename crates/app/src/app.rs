use std::sync::Arc;
use std::time::Instant;

use ibex_core::Store;
use tracing::Instrument;

use crate::command::{self, Call, CommandError, Outcome};
use crate::event::{self, Event, EventSink, ExecutionId, JsonLinesSink, Status};

/// Which shell is driving. Stamped on every wide event as `surface`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    Cli,
    Gui,
    Mcp,
    Test,
}

impl Surface {
    pub fn as_str(self) -> &'static str {
        match self {
            Surface::Cli => "cli",
            Surface::Gui => "gui",
            Surface::Mcp => "mcp",
            Surface::Test => "test",
        }
    }
}

/// Process-wide application state. Cheap to clone; share freely across tasks.
///
/// Immutable after `build()`. Concurrent `execute` calls share no mutable state
/// (the sink serializes its own writes).
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    store: Arc<dyn Store>,
    sink: Arc<dyn EventSink>,
    surface: Surface,
    started_at: Instant,
    version: &'static str,
}

pub struct AppStateBuilder {
    store: Arc<dyn Store>,
    sink: Option<Arc<dyn EventSink>>,
    surface: Surface,
}

impl AppState {
    /// Store is required — there is no hidden default backend.
    pub fn builder(store: impl Store) -> AppStateBuilder {
        AppStateBuilder {
            store: Arc::new(store),
            sink: None,
            surface: Surface::Cli,
        }
    }

    /// The single entry point. Every command runs through here.
    pub async fn execute(&self, call: impl Into<Call>) -> Result<Outcome, CommandError> {
        let call = call.into();
        let mut guard = EmitGuard::open(&self.inner, call.command.name(), call.request_id);
        let span = tracing::info_span!(
            "command",
            command = guard.name(),
            execution_id = %guard.id()
        );
        let result = command::dispatch(self, call.command, guard.event_mut())
            .instrument(span)
            .await;
        guard.finish(result.as_ref().map(|_| ()));
        result
    }

    pub(crate) fn store(&self) -> &dyn Store {
        &*self.inner.store
    }

    pub(crate) fn uptime(&self) -> std::time::Duration {
        self.inner.started_at.elapsed()
    }

    pub(crate) fn version(&self) -> &'static str {
        self.inner.version
    }
}

impl AppStateBuilder {
    pub fn surface(mut self, s: Surface) -> Self {
        self.surface = s;
        self
    }

    pub fn sink(mut self, sink: impl EventSink) -> Self {
        self.sink = Some(Arc::new(sink));
        self
    }

    /// Infallible: nothing here does I/O. Opening the library is the adapter's job.
    pub fn build(self) -> AppState {
        let sink = self
            .sink
            .unwrap_or_else(|| Arc::new(JsonLinesSink::stderr()));
        AppState {
            inner: Arc::new(Inner {
                store: self.store,
                sink,
                surface: self.surface,
                started_at: Instant::now(),
                version: env!("CARGO_PKG_VERSION"),
            }),
        }
    }
}

/// Owns the in-flight event for one execution. Emits exactly once.
struct EmitGuard<'a> {
    inner: &'a Inner,
    event: Option<Event>,
    started: Instant,
    command: &'static str,
}

impl<'a> EmitGuard<'a> {
    fn open(inner: &'a Inner, command: &'static str, request_id: Option<String>) -> Self {
        let mut event = Event::new(ExecutionId::generate(), command);
        event.set(event::SURFACE, inner.surface);
        event.set(event::APP_VERSION, inner.version);
        if let Some(id) = request_id {
            event.set(event::REQUEST_ID, id);
        }
        Self {
            inner,
            event: Some(event),
            started: Instant::now(),
            command,
        }
    }

    fn id(&self) -> ExecutionId {
        self.event
            .as_ref()
            .expect("EmitGuard event present before emit")
            .id()
    }

    fn name(&self) -> &'static str {
        self.command
    }

    fn event_mut(&mut self) -> &mut Event {
        self.event
            .as_mut()
            .expect("EmitGuard event present before emit")
    }

    fn finish(mut self, result: Result<(), &CommandError>) {
        let status = match result {
            Ok(()) => Status::Ok,
            Err(e) => {
                if let Some(ev) = self.event.as_mut() {
                    ev.set(event::ERROR_KIND, e.kind().as_str());
                    ev.set(event::ERROR_MESSAGE, e.to_string());
                }
                Status::Error
            }
        };
        self.emit(status);
    }

    fn emit(&mut self, status: Status) {
        if let Some(ev) = self.event.take() {
            self.inner
                .sink
                .emit(&ev.finish(status, self.started.elapsed()));
        }
    }
}

impl Drop for EmitGuard<'_> {
    fn drop(&mut self) {
        let status = if std::thread::panicking() {
            Status::Panicked
        } else {
            Status::Cancelled
        };
        self.emit(status);
    }
}
