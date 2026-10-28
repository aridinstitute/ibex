mod biblatex_import;
mod collection;
mod health;
mod person;
pub(crate) mod resource;
mod series;
mod tag;

pub use biblatex_import::{
    ImportBiblatex, ImportCslJson, ImportIssue, ImportedEntry, PreviewBiblatex, PreviewCslJson,
    SaveImportDraft,
};
pub use oeuvre_core::{ImportChoices, ImportPreview, ImportReport};
pub use collection::{
    AddCollectionParent, CreateCollection, DeleteCollection, DeletedCollection,
    ListCollectionForest, RemoveCollectionParent, RenameCollection,
};
pub use health::{Health, HealthReport};
pub use person::{
    CreatePerson, DeletePerson, DeletedPerson, FindPersonsByName, GetPerson, ListPersons,
    UpdatePerson,
};
pub use resource::{
    CountResources, CreateResource, GetResource, GetResourceDetail, ListResourceGridPage,
    ListResources, ListResourcesPage, ListResourcesQueryPage, SetResourceFavorite, UpdateResource,
};
pub use series::CreateSeries;
pub use tag::{CreateTag, FindTagByText, GetTag, ListTags};

use oeuvre_core::{
    Collection, CollectionTreeNode, Person, Resource, ResourceCounts, ResourceDetail,
    ResourceGridPage, ResourcePage, ResourceSummary, Series, SortedResourcePage, StoreError, Tag,
};

use crate::AppState;
use crate::event::Event;

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
    CreateSeries(CreateSeries),
    CreateTag(CreateTag),
    GetTag(GetTag),
    FindTagByText(FindTagByText),
    ListTags(ListTags),
    CreateCollection(CreateCollection),
    RenameCollection(RenameCollection),
    ListCollectionForest(ListCollectionForest),
    AddCollectionParent(AddCollectionParent),
    RemoveCollectionParent(RemoveCollectionParent),
    DeleteCollection(DeleteCollection),
    CreateResource(CreateResource),
    ImportBiblatex(ImportBiblatex),
    ImportCslJson(ImportCslJson),
    PreviewBiblatex(PreviewBiblatex),
    PreviewCslJson(PreviewCslJson),
    SaveImportDraft(SaveImportDraft),
    UpdateResource(UpdateResource),
    SetResourceFavorite(SetResourceFavorite),
    GetResource(GetResource),
    GetResourceDetail(GetResourceDetail),
    ListResources(ListResources),
    ListResourcesPage(ListResourcesPage),
    ListResourcesQueryPage(ListResourcesQueryPage),
    ListResourceGridPage(ListResourceGridPage),
    CountResources(CountResources),
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
    CreateSeries(Series),
    CreateTag(Tag),
    GetTag(Tag),
    FindTagByText(Option<Tag>),
    ListTags(Vec<Tag>),
    CreateCollection(Collection),
    RenameCollection(Collection),
    ListCollectionForest(Vec<CollectionTreeNode>),
    AddCollectionParent(()),
    RemoveCollectionParent(()),
    DeleteCollection(DeletedCollection),
    CreateResource(Resource),
    ImportBiblatex(ImportReport),
    ImportCslJson(ImportReport),
    PreviewBiblatex(ImportPreview),
    PreviewCslJson(ImportPreview),
    SaveImportDraft(ImportReport),
    UpdateResource(Resource),
    SetResourceFavorite(Resource),
    GetResource(Resource),
    GetResourceDetail(ResourceDetail),
    ListResources(Vec<ResourceSummary>),
    ListResourcesPage(ResourcePage),
    ListResourcesQueryPage(SortedResourcePage),
    ResourceGridPage(ResourceGridPage),
    CountResources(ResourceCounts),
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
            Command::CreateSeries(_) => CreateSeries::NAME,
            Command::CreateTag(_) => CreateTag::NAME,
            Command::GetTag(_) => GetTag::NAME,
            Command::FindTagByText(_) => FindTagByText::NAME,
            Command::ListTags(_) => ListTags::NAME,
            Command::CreateCollection(_) => CreateCollection::NAME,
            Command::RenameCollection(_) => RenameCollection::NAME,
            Command::ListCollectionForest(_) => ListCollectionForest::NAME,
            Command::AddCollectionParent(_) => AddCollectionParent::NAME,
            Command::RemoveCollectionParent(_) => RemoveCollectionParent::NAME,
            Command::DeleteCollection(_) => DeleteCollection::NAME,
            Command::CreateResource(_) => CreateResource::NAME,
            Command::ImportBiblatex(_) => ImportBiblatex::NAME,
            Command::ImportCslJson(_) => ImportCslJson::NAME,
            Command::PreviewBiblatex(_) => PreviewBiblatex::NAME,
            Command::PreviewCslJson(_) => PreviewCslJson::NAME,
            Command::SaveImportDraft(_) => SaveImportDraft::NAME,
            Command::UpdateResource(_) => UpdateResource::NAME,
            Command::SetResourceFavorite(_) => SetResourceFavorite::NAME,
            Command::GetResource(_) => GetResource::NAME,
            Command::GetResourceDetail(_) => GetResourceDetail::NAME,
            Command::ListResources(_) => ListResources::NAME,
            Command::ListResourcesPage(_) => ListResourcesPage::NAME,
            Command::ListResourcesQueryPage(_) => ListResourcesQueryPage::NAME,
            Command::ListResourceGridPage(_) => ListResourceGridPage::NAME,
            Command::CountResources(_) => CountResources::NAME,
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

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Self::Output, CommandError>;
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
        Command::CreateSeries(c) => drive(c, state, ev).await,
        Command::CreateTag(c) => drive(c, state, ev).await,
        Command::GetTag(c) => drive(c, state, ev).await,
        Command::FindTagByText(c) => drive(c, state, ev).await,
        Command::ListTags(c) => drive(c, state, ev).await,
        Command::CreateCollection(c) => drive(c, state, ev).await,
        Command::RenameCollection(c) => drive(c, state, ev).await,
        Command::ListCollectionForest(c) => drive(c, state, ev).await,
        Command::AddCollectionParent(c) => drive(c, state, ev).await,
        Command::RemoveCollectionParent(c) => drive(c, state, ev).await,
        Command::DeleteCollection(c) => drive(c, state, ev).await,
        Command::CreateResource(c) => drive(c, state, ev).await,
        Command::ImportBiblatex(c) => drive(c, state, ev).await,
        Command::ImportCslJson(c) => drive(c, state, ev).await,
        Command::PreviewBiblatex(c) => drive(c, state, ev).await,
        Command::PreviewCslJson(c) => drive(c, state, ev).await,
        Command::SaveImportDraft(c) => drive(c, state, ev).await,
        Command::UpdateResource(c) => drive(c, state, ev).await,
        Command::SetResourceFavorite(c) => drive(c, state, ev).await,
        Command::GetResource(c) => drive(c, state, ev).await,
        Command::GetResourceDetail(c) => drive(c, state, ev).await,
        Command::ListResources(c) => drive(c, state, ev).await,
        Command::ListResourcesPage(c) => drive(c, state, ev).await,
        Command::ListResourcesQueryPage(c) => drive(c, state, ev).await,
        Command::ListResourceGridPage(c) => drive(c, state, ev).await,
        Command::CountResources(c) => drive(c, state, ev).await,
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
