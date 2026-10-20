mod health;
mod person;
mod resource;

pub use health::{Health, HealthReport};
pub use person::{
    CreatePerson, DeletePerson, DeletedPerson, FindPersonsByName, GetPerson, ListPersons,
    UpdatePerson,
};
pub use resource::CreateResource;

use oeuvre_core::{Person, Resource, StoreError};

use crate::event::Event;
use crate::AppState;

/// Every operation Oeuvre supports. Closed set; shells enumerate this.
/// Variants carry already-validated domain inputs — parsing happens in the shell.
#[derive(Debug, Clone)]
pub enum Command {
    Health(Health),
    CreatePerson(CreatePerson),
    GetPerson(GetPerson),
    UpdatePerson(UpdatePerson),
    DeletePerson(DeletePerson),
    FindPersonsByName(FindPersonsByName),
    ListPersons(ListPersons),
    CreateResource(CreateResource),
}

/// The result of a successful command. One variant per [`Command`] variant.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "command", content = "result", rename_all = "snake_case")]
pub enum Outcome {
    Health(HealthReport),
    CreatePerson(Person),
    GetPerson(Person),
    UpdatePerson(Person),
    DeletePerson(DeletedPerson),
    FindPersonsByName(Vec<Person>),
    ListPersons(Vec<Person>),
    CreateResource(Resource),
}

/// A command plus per-call transport context.
#[derive(Debug, Clone)]
pub struct Call {
    pub(crate) command: Command,
    pub(crate) request_id: Option<String>,
}

impl Call {
    pub fn new(command: Command) -> Self {
        Self {
            command,
            request_id: None,
        }
    }

    pub fn request_id(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(id.into());
        self
    }
}

impl From<Command> for Call {
    fn from(c: Command) -> Self {
        Call::new(c)
    }
}

impl Command {
    /// Stable snake_case name; used for the `command` log field and MCP tool names.
    pub fn name(&self) -> &'static str {
        match self {
            Command::Health(_) => Health::NAME,
            Command::CreatePerson(_) => CreatePerson::NAME,
            Command::GetPerson(_) => GetPerson::NAME,
            Command::UpdatePerson(_) => UpdatePerson::NAME,
            Command::DeletePerson(_) => DeletePerson::NAME,
            Command::FindPersonsByName(_) => FindPersonsByName::NAME,
            Command::ListPersons(_) => ListPersons::NAME,
            Command::CreateResource(_) => CreateResource::NAME,
        }
    }
}

/// Crate-private: external crates can't add commands or call handlers directly.
#[async_trait::async_trait]
pub(crate) trait Handler: Sized + Send {
    const NAME: &'static str;
    type Output: Into<Outcome>;

    /// Record input dimensions before `run`, so they land even if `run` fails early.
    fn annotate(&self, _ev: &mut Event) {}

    async fn run(
        self,
        state: &AppState,
        ev: &mut Event,
    ) -> Result<Self::Output, CommandError>;
}

/// Exhaustive: a new `Command` variant without an arm fails to compile.
pub(crate) async fn dispatch(
    state: &AppState,
    cmd: Command,
    ev: &mut Event,
) -> Result<Outcome, CommandError> {
    match cmd {
        Command::Health(c) => drive(c, state, ev).await,
        Command::CreatePerson(c) => drive(c, state, ev).await,
        Command::GetPerson(c) => drive(c, state, ev).await,
        Command::UpdatePerson(c) => drive(c, state, ev).await,
        Command::DeletePerson(c) => drive(c, state, ev).await,
        Command::FindPersonsByName(c) => drive(c, state, ev).await,
        Command::ListPersons(c) => drive(c, state, ev).await,
        Command::CreateResource(c) => drive(c, state, ev).await,
    }
}

async fn drive<H: Handler>(
    h: H,
    state: &AppState,
    ev: &mut Event,
) -> Result<Outcome, CommandError> {
    h.annotate(ev);
    h.run(state, ev).await.map(Into::into)
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("internal: {0}")]
    Internal(String),
}

/// Low-cardinality error class for log fields, CLI exit codes, and MCP error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Invalid,
    NotFound,
    Store,
    Internal,
}

impl CommandError {
    pub fn kind(&self) -> ErrorKind {
        match self {
            CommandError::Invalid(_) => ErrorKind::Invalid,
            CommandError::NotFound(_) => ErrorKind::NotFound,
            CommandError::Store(_) => ErrorKind::Store,
            CommandError::Internal(_) => ErrorKind::Internal,
        }
    }
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Invalid => "invalid",
            ErrorKind::NotFound => "not_found",
            ErrorKind::Store => "store",
            ErrorKind::Internal => "internal",
        }
    }
}
