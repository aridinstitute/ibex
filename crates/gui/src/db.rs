//! Store access for the UI: one OS thread owns a tokio runtime and serves
//! page requests over channels, so gpui's executor never runs Surreal I/O.

use oeuvre_app::command::ListResourceGridPage;
#[cfg(test)]
use oeuvre_app::command::ListResourcesPage;
use oeuvre_app::command::{
    AddCollectionParent, CountResources, CreateCollection, CreatePerson, CreateResource, CreateTag,
    DeleteCollection, FindTagByText, GetResource, GetResourceDetail, ListCollectionForest,
    ListPersons, ListResourcesQueryPage, ListTags, RemoveCollectionParent, RenameCollection,
    SetResourceFavorite, UpdateResource,
};
use oeuvre_app::command::{ImportBiblatex, ImportReport, PreviewBiblatex};
use oeuvre_app::{AppState, Command, Outcome};
#[cfg(test)]
use oeuvre_core::ResourcePage;
use oeuvre_core::{
    Collection, CollectionId, CollectionTreeNode, ImportChoices, ImportPreview, Name, NewResource,
    NonEmptyStr, Person, Resource, ResourceCounts, ResourceDetail, ResourceId, ResourceQuery,
    SortedResourcePage, Tag,
};
use oeuvre_core::{ResourceGridPage, ResourceGridParent};
use std::{io::Read as _, path::PathBuf};
use tokio::sync::oneshot;

/// File contents plus preview for the GUI review step.
#[derive(Debug, Clone)]
pub struct BiblatexImportPlan {
    pub source_text: String,
    pub preview: ImportPreview,
}

pub enum DbRequest {
    PreviewBiblatex {
        path: PathBuf,
        reply: oneshot::Sender<Result<BiblatexImportPlan, String>>,
    },
    ImportBiblatex {
        source_text: String,
        choices: ImportChoices,
        reply: oneshot::Sender<Result<ImportReport, String>>,
    },
    #[cfg(test)]
    ListPage {
        before: Option<ResourceId>,
        limit: u32,
        reply: oneshot::Sender<Result<ResourcePage, String>>,
    },
    ListQueryPage {
        query: ResourceQuery,
        offset: u64,
        limit: u32,
        reply: oneshot::Sender<Result<SortedResourcePage, String>>,
    },
    ListGridPage {
        parent: ResourceGridParent,
        query: ResourceQuery,
        offset: u64,
        limit: u32,
        reply: oneshot::Sender<Result<ResourceGridPage, String>>,
    },
    CountResources {
        reply: oneshot::Sender<Result<ResourceCounts, String>>,
    },
    GetDetail {
        id: ResourceId,
        reply: oneshot::Sender<Result<ResourceDetail, String>>,
    },
    GetResource {
        id: ResourceId,
        reply: oneshot::Sender<Result<Resource, String>>,
    },
    UpdateResource {
        id: ResourceId,
        new: NewResource,
        reply: oneshot::Sender<Result<Resource, String>>,
    },
    SetResourceFavorite {
        id: ResourceId,
        favorite: bool,
        reply: oneshot::Sender<Result<Resource, String>>,
    },
    CreateResource {
        new: NewResource,
        reply: oneshot::Sender<Result<Resource, String>>,
    },
    ListPersons {
        reply: oneshot::Sender<Result<Vec<Person>, String>>,
    },
    CreatePerson {
        name: Name,
        reply: oneshot::Sender<Result<Person, String>>,
    },
    ListTags {
        reply: oneshot::Sender<Result<Vec<Tag>, String>>,
    },
    CreateTag {
        text: NonEmptyStr,
        reply: oneshot::Sender<Result<Tag, String>>,
    },
    FindTagByText {
        text: NonEmptyStr,
        reply: oneshot::Sender<Result<Option<Tag>, String>>,
    },
    ListCollectionForest {
        reply: oneshot::Sender<Result<Vec<CollectionTreeNode>, String>>,
    },
    CreateCollection {
        name: NonEmptyStr,
        parent_id: Option<CollectionId>,
        reply: oneshot::Sender<Result<Collection, String>>,
    },
    DeleteCollection {
        id: CollectionId,
        reply: oneshot::Sender<Result<(), String>>,
    },
    RenameCollection {
        id: CollectionId,
        name: NonEmptyStr,
        reply: oneshot::Sender<Result<Collection, String>>,
    },
    AddCollectionParent {
        child_id: CollectionId,
        parent_id: CollectionId,
        reply: oneshot::Sender<Result<(), String>>,
    },
    RemoveCollectionParent {
        child_id: CollectionId,
        parent_id: CollectionId,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

#[derive(Clone)]
pub struct DbWorker {
    tx: std::sync::mpsc::Sender<DbRequest>,
}

impl DbWorker {
    /// Read a bibliography and build a shared preview (no writes).
    pub async fn preview_biblatex(&self, path: PathBuf) -> Result<BiblatexImportPlan, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::PreviewBiblatex { path, reply: tx })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    /// Persist a previously previewed bibliography with review choices.
    pub async fn import_biblatex(
        &self,
        source_text: String,
        choices: ImportChoices,
    ) -> Result<ImportReport, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::ImportBiblatex {
                source_text,
                choices,
                reply: tx,
            })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }
    /// Spawn the worker thread. The thread owns the tokio runtime that runs
    /// every command against `state`, mirroring the CLI's runtime so Surreal
    /// never runs on gpui's executor.
    pub fn spawn(state: AppState) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<DbRequest>();
        std::thread::Builder::new()
            .name("oeuvre-db".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("db worker runtime");
                rt.block_on(async move {
                    for req in rx {
                        match req {
                            DbRequest::PreviewBiblatex { path, reply } => {
                                let result = async {
                                    let source_text = read_bibliography_file(&path)?;
                                    match state
                                        .execute(Command::PreviewBiblatex(PreviewBiblatex {
                                            source_text: source_text.clone(),
                                        }))
                                        .await
                                        .map_err(|e| e.to_string())?
                                    {
                                        Outcome::PreviewBiblatex(preview) => {
                                            Ok(BiblatexImportPlan {
                                                source_text,
                                                preview,
                                            })
                                        }
                                        _ => Err("unexpected outcome from preview_biblatex".into()),
                                    }
                                }
                                .await;
                                let _ = reply.send(result);
                            }
                            DbRequest::ImportBiblatex {
                                source_text,
                                choices,
                                reply,
                            } => {
                                let result = async {
                                    match state
                                        .execute(Command::ImportBiblatex(ImportBiblatex {
                                            source_text,
                                            choices,
                                        }))
                                        .await
                                        .map_err(|e| e.to_string())?
                                    {
                                        Outcome::ImportBiblatex(report) => Ok(report),
                                        _ => Err("unexpected outcome from import_biblatex".into()),
                                    }
                                }
                                .await;
                                let _ = reply.send(result);
                            }
                            #[cfg(test)]
                            DbRequest::ListPage {
                                before,
                                limit,
                                reply,
                            } => {
                                let result =
                                    state
                                        .execute(Command::ListResourcesPage(ListResourcesPage {
                                            before,
                                            limit,
                                        }))
                                        .await
                                        .map_err(|e| e.to_string())
                                        .and_then(|outcome| match outcome {
                                            Outcome::ListResourcesPage(page) => Ok(page),
                                            _ => Err("unexpected outcome from list_resources_page"
                                                .into()),
                                        });
                                let _ = reply.send(result);
                            }
                            DbRequest::ListQueryPage {
                                query,
                                offset,
                                limit,
                                reply,
                            } => {
                                let result = state
                                    .execute(Command::ListResourcesQueryPage(
                                        ListResourcesQueryPage {
                                            query,
                                            offset,
                                            limit,
                                        },
                                    ))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::ListResourcesQueryPage(page) => Ok(page),
                                        _ => {
                                            Err("unexpected outcome from list_resources_query_page"
                                                .into())
                                        }
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::ListGridPage {
                                parent,
                                query,
                                offset,
                                limit,
                                reply,
                            } => {
                                let result = state
                                    .execute(Command::ListResourceGridPage(ListResourceGridPage {
                                        parent,
                                        query,
                                        offset,
                                        limit,
                                    }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::ResourceGridPage(page) => Ok(page),
                                        _ => Err("unexpected outcome from list_resource_grid_page"
                                            .into()),
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::CountResources { reply } => {
                                let result = state
                                    .execute(Command::CountResources(CountResources))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::CountResources(counts) => Ok(counts),
                                        _ => Err("unexpected outcome from count_resources".into()),
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::GetDetail { id, reply } => {
                                let result = state
                                    .execute(Command::GetResourceDetail(GetResourceDetail { id }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::GetResourceDetail(detail) => Ok(detail),
                                        _ => {
                                            Err("unexpected outcome from get_resource_detail"
                                                .into())
                                        }
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::GetResource { id, reply } => {
                                let result = state
                                    .execute(Command::GetResource(GetResource { id }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::GetResource(resource) => Ok(resource),
                                        _ => Err("unexpected outcome from get_resource".into()),
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::UpdateResource { id, new, reply } => {
                                let result = state
                                    .execute(Command::UpdateResource(UpdateResource { id, new }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::UpdateResource(resource) => Ok(resource),
                                        _ => Err("unexpected outcome from update_resource".into()),
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::SetResourceFavorite {
                                id,
                                favorite,
                                reply,
                            } => {
                                let result = state
                                    .execute(Command::SetResourceFavorite(SetResourceFavorite {
                                        id,
                                        favorite,
                                    }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::SetResourceFavorite(resource) => Ok(resource),
                                        _ => {
                                            Err("unexpected outcome from set_resource_favorite"
                                                .into())
                                        }
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::CreateResource { new, reply } => {
                                let result = state
                                    .execute(Command::CreateResource(CreateResource { new }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::CreateResource(resource) => Ok(resource),
                                        _ => Err("unexpected outcome from create_resource".into()),
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::ListPersons { reply } => {
                                let result = state
                                    .execute(Command::ListPersons(ListPersons))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::ListPersons(people) => Ok(people),
                                        _ => Err("unexpected outcome from list_persons".into()),
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::CreatePerson { name, reply } => {
                                let result = state
                                    .execute(Command::CreatePerson(CreatePerson {
                                        name,
                                        aliases: vec![],
                                    }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::CreatePerson(person) => Ok(person),
                                        _ => Err("unexpected outcome from create_person".into()),
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::ListTags { reply } => {
                                let result = state
                                    .execute(Command::ListTags(ListTags))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::ListTags(tags) => Ok(tags),
                                        _ => Err("unexpected outcome from list_tags".into()),
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::CreateTag { text, reply } => {
                                let result = state
                                    .execute(Command::CreateTag(CreateTag { text }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::CreateTag(tag) => Ok(tag),
                                        _ => Err("unexpected outcome from create_tag".into()),
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::FindTagByText { text, reply } => {
                                let result = state
                                    .execute(Command::FindTagByText(FindTagByText { text }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::FindTagByText(tag) => Ok(tag),
                                        _ => Err("unexpected outcome from find_tag_by_text".into()),
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::ListCollectionForest { reply } => {
                                let result = state
                                    .execute(Command::ListCollectionForest(ListCollectionForest))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::ListCollectionForest(forest) => Ok(forest),
                                        _ => {
                                            Err("unexpected outcome from list_collection_forest"
                                                .into())
                                        }
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::CreateCollection {
                                name,
                                parent_id,
                                reply,
                            } => {
                                let result = state
                                    .execute(Command::CreateCollection(CreateCollection {
                                        name,
                                        parent_id,
                                    }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::CreateCollection(collection) => Ok(collection),
                                        _ => {
                                            Err("unexpected outcome from create_collection".into())
                                        }
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::DeleteCollection { id, reply } => {
                                let result = state
                                    .execute(Command::DeleteCollection(DeleteCollection { id }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::DeleteCollection(_) => Ok(()),
                                        _ => {
                                            Err("unexpected outcome from delete_collection".into())
                                        }
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::RenameCollection { id, name, reply } => {
                                let result = state
                                    .execute(Command::RenameCollection(RenameCollection {
                                        id,
                                        name,
                                    }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::RenameCollection(collection) => Ok(collection),
                                        _ => {
                                            Err("unexpected outcome from rename_collection".into())
                                        }
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::AddCollectionParent {
                                child_id,
                                parent_id,
                                reply,
                            } => {
                                let result = state
                                    .execute(Command::AddCollectionParent(AddCollectionParent {
                                        child_id,
                                        parent_id,
                                    }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::AddCollectionParent(()) => Ok(()),
                                        _ => {
                                            Err("unexpected outcome from add_collection_parent"
                                                .into())
                                        }
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::RemoveCollectionParent {
                                child_id,
                                parent_id,
                                reply,
                            } => {
                                let result = state
                                    .execute(Command::RemoveCollectionParent(
                                        RemoveCollectionParent {
                                            child_id,
                                            parent_id,
                                        },
                                    ))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::RemoveCollectionParent(()) => Ok(()),
                                        _ => {
                                            Err("unexpected outcome from remove_collection_parent"
                                                .into())
                                        }
                                    });
                                let _ = reply.send(result);
                            }
                        }
                    }
                });
            })
            .expect("spawn db worker thread");
        Self { tx }
    }

    /// Request a page and wait for the reply off the UI thread.
    #[cfg(test)]
    pub async fn list_page(
        &self,
        before: Option<ResourceId>,
        limit: u32,
    ) -> Result<ResourcePage, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::ListPage {
                before,
                limit,
                reply: tx,
            })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    /// Request one offset window of a queried listing and wait for the
    /// reply off the UI thread.
    pub async fn list_query_page(
        &self,
        query: ResourceQuery,
        offset: u64,
        limit: u32,
    ) -> Result<SortedResourcePage, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::ListQueryPage {
                query,
                offset,
                limit,
                reply: tx,
            })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    /// Request one tree level through the app command boundary.
    pub async fn list_grid_page(
        &self,
        parent: ResourceGridParent,
        query: ResourceQuery,
        offset: u64,
        limit: u32,
    ) -> Result<ResourceGridPage, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::ListGridPage {
                parent,
                query,
                offset,
                limit,
                reply: tx,
            })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    /// Bootstrap a tree level before the UI's async context exists.
    pub fn list_grid_page_blocking(
        &self,
        parent: ResourceGridParent,
        query: ResourceQuery,
        offset: u64,
        limit: u32,
    ) -> Result<ResourceGridPage, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::ListGridPage {
                parent,
                query,
                offset,
                limit,
                reply: tx,
            })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.blocking_recv()
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    pub async fn resource_counts(&self) -> Result<ResourceCounts, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::CountResources { reply: tx })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    /// Fetch every person for the creator filter's dropdown.
    pub async fn list_persons(&self) -> Result<Vec<Person>, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::ListPersons { reply: tx })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    pub async fn create_person(&self, name: Name) -> Result<Person, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::CreatePerson { name, reply: tx })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    pub async fn list_tags(&self) -> Result<Vec<Tag>, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::ListTags { reply: tx })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    pub async fn create_tag(&self, text: NonEmptyStr) -> Result<Tag, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::CreateTag { text, reply: tx })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    pub async fn find_tag_by_text(&self, text: NonEmptyStr) -> Result<Option<Tag>, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::FindTagByText { text, reply: tx })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    pub async fn list_collection_forest(&self) -> Result<Vec<CollectionTreeNode>, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::ListCollectionForest { reply: tx })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    pub async fn create_collection(
        &self,
        name: NonEmptyStr,
        parent_id: Option<CollectionId>,
    ) -> Result<Collection, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::CreateCollection {
                name,
                parent_id,
                reply: tx,
            })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    pub async fn delete_collection(&self, id: CollectionId) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::DeleteCollection { id, reply: tx })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    pub async fn rename_collection(
        &self,
        id: CollectionId,
        name: NonEmptyStr,
    ) -> Result<Collection, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::RenameCollection {
                id,
                name,
                reply: tx,
            })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    pub async fn add_collection_parent(
        &self,
        child_id: CollectionId,
        parent_id: CollectionId,
    ) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::AddCollectionParent {
                child_id,
                parent_id,
                reply: tx,
            })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    pub async fn remove_collection_parent(
        &self,
        child_id: CollectionId,
        parent_id: CollectionId,
    ) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::RemoveCollectionParent {
                child_id,
                parent_id,
                reply: tx,
            })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    /// Fetch the fully-resolved detail for one resource, for the panel.
    pub async fn resource_detail(&self, id: ResourceId) -> Result<ResourceDetail, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::GetDetail { id, reply: tx })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    /// Fetch the raw stored record for one resource, for the edit form:
    /// the detail projection drops ids the form needs (creator person ids,
    /// parent, series).
    pub async fn resource(&self, id: ResourceId) -> Result<Resource, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::GetResource { id, reply: tx })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    /// Replace one resource's stored fields with `new`, handing back the
    /// stored record.
    pub async fn update_resource(
        &self,
        id: ResourceId,
        new: NewResource,
    ) -> Result<Resource, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::UpdateResource { id, new, reply: tx })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    /// Toggle only the favorite flag on `id`.
    pub async fn set_resource_favorite(
        &self,
        id: ResourceId,
        favorite: bool,
    ) -> Result<Resource, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::SetResourceFavorite {
                id,
                favorite,
                reply: tx,
            })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    /// Insert a new resource from `new`, handing back the stored record.
    pub async fn create_resource(&self, new: NewResource) -> Result<Resource, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::CreateResource { new, reply: tx })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.await
            .map_err(|_| "db worker dropped the request".to_string())?
    }

    /// A synchronous listing/barrier for worker and GUI tests.
    #[cfg(test)]
    pub fn list_page_blocking(
        &self,
        before: Option<ResourceId>,
        limit: u32,
    ) -> Result<ResourcePage, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::ListPage {
                before,
                limit,
                reply: tx,
            })
            .map_err(|_| "db worker stopped".to_string())?;
        rx.blocking_recv()
            .map_err(|_| "db worker dropped the request".to_string())?
    }
}

fn read_bibliography_file(path: &PathBuf) -> Result<String, String> {
    // File I/O stays on the db worker, never GPUI's executor.
    let file =
        std::fs::File::open(path).map_err(|e| format!("Unable to open bibliography: {e}"))?;
    let mut bytes = Vec::new();
    const MAX_BYTES: u64 = 16 * 1024 * 1024;
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Unable to read bibliography: {e}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Bibliography exceeds the 16 MiB import limit".into());
    }
    String::from_utf8(bytes).map_err(|_| "Bibliography must be UTF-8 text".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oeuvre_app::Surface;
    use oeuvre_app::testing::MemoryStore;
    use oeuvre_core::published_label_from;

    #[tokio::test]
    async fn worker_lists_grid_roots_and_children_with_metadata() {
        let state = AppState::builder(MemoryStore::new())
            .surface(Surface::Test)
            .build();
        let worker = DbWorker::spawn(state);
        let book = NewResource {
            kind: oeuvre_core::ResourceKind::Book,
            title: oeuvre_core::NonEmptyStr::try_new("Grid book").expect("title non-empty"),
            creators: vec![],
            publisher: None,
            published: None,
            accessed: None,
            parent_id: None,
            series_id: None,
            series_number: None,
            tag_ids: vec![],
            collection_ids: vec![],
            metadata: Default::default(),
            subtitle: None,
            title_addon: None,
            short_title: None,
            place: None,
            volume: None,
            issue: None,
            pages: None,
            total_pages: None,
            article_number: None,
            edition: None,
            doi: None,
            url: None,
            eprint: None,
            isbns: vec![],
            issns: vec![],
            isrns: vec![],
            original_date: None,
            event_date: None,
            language: None,
            document_type: None,
            publication_method: None,
            institution: None,
            organization: None,
            school: None,
            abstract_text: None,
            keywords: vec![],
        };
        let chapter = NewResource {
            kind: oeuvre_core::ResourceKind::BookSection,
            title: NonEmptyStr::try_new("Grid chapter").unwrap(),
            ..book.clone()
        };
        let book = worker.create_resource(book).await.unwrap();
        let chapter = worker
            .create_resource(NewResource {
                parent_id: Some(book.id),
                ..chapter
            })
            .await
            .unwrap();

        let roots = worker
            .list_grid_page(
                oeuvre_core::ResourceGridParent::Roots,
                ResourceQuery::newest_first(),
                0,
                10,
            )
            .await
            .unwrap();
        assert_eq!(roots.rows.len(), 1);
        assert_eq!(roots.rows[0].summary.id, book.id);
        assert_eq!(roots.rows[0].summary.title.as_str(), "Grid book");
        assert!(roots.rows[0].has_children);
        assert_eq!(roots.next_offset, None);

        let children = std::thread::spawn(move || {
            worker.list_grid_page_blocking(
                oeuvre_core::ResourceGridParent::Children(book.id),
                ResourceQuery::newest_first(),
                0,
                10,
            )
        })
        .join()
        .unwrap()
        .unwrap();
        assert_eq!(children.rows.len(), 1);
        assert_eq!(children.rows[0].summary.id, chapter.id);
        assert_eq!(children.rows[0].summary.title.as_str(), "Grid chapter");
        assert!(!children.rows[0].has_children);
        assert_eq!(children.next_offset, None);
    }

    #[tokio::test]
    async fn worker_imports_a_file_and_returns_report_with_persisted_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.bib");
        std::fs::write(&path, "@book{one,title={Imported book},doi={10.1000/book}}\n@article{two,title={Imported paper},journaltitle={Journal}}\n@patent{three,title={Skipped}}").unwrap();
        let worker = DbWorker::spawn(
            AppState::builder(MemoryStore::new())
                .surface(Surface::Test)
                .build(),
        );
        let plan = worker.preview_biblatex(path).await.unwrap();
        let report = worker
            .import_biblatex(plan.source_text, ImportChoices::default())
            .await
            .unwrap();
        assert_eq!(report.imported.len(), 2);
        assert_eq!(report.skipped.len(), 1);
        // The article also materializes its linked journal container.
        assert_eq!(worker.resource_counts().await.unwrap().all_items, 3);
        let book = worker
            .resource(report.imported[0].resource_id)
            .await
            .unwrap();
        assert_eq!(book.doi.as_deref(), Some("10.1000/book"));
        assert!(
            worker
                .preview_biblatex(dir.path().join("missing.bib"))
                .await
                .is_err()
        );
        assert_eq!(worker.resource_counts().await.unwrap().all_items, 3);
    }

    #[tokio::test]
    async fn worker_creates_person_available_for_author_selection() {
        let state = AppState::builder(MemoryStore::new())
            .surface(Surface::Test)
            .build();
        let worker = DbWorker::spawn(state);
        let name = Name::try_new(Some("Grace"), Some("Hopper"), None::<&str>).unwrap();
        let created = worker.create_person(name).await.unwrap();
        assert_eq!(created.name.display(), "Grace Hopper");
        assert!(
            worker
                .list_persons()
                .await
                .unwrap()
                .iter()
                .any(|p| p.id == created.id)
        );
    }

    #[tokio::test]
    async fn worker_walks_pages() {
        let store = MemoryStore::new();
        oeuvre_app::seed::seed_demo_library(&store, 5)
            .await
            .unwrap();
        let state = AppState::builder(store).surface(Surface::Test).build();
        let worker = DbWorker::spawn(state);

        // 5 demo rows plus 3 containers: pages of 3 must tile 8 exactly.
        let mut before = None;
        let mut walked = 0usize;
        let mut pages = 0;
        loop {
            let page = worker.list_page(before, 3).await.unwrap();
            assert!(!page.rows.is_empty() || page.next_before.is_none());
            assert!(page.rows.len() <= 3);
            walked += page.rows.len();
            pages += 1;
            match page.next_before {
                Some(next) => before = Some(next),
                None => break,
            }
        }
        assert_eq!(walked, 8);
        assert!(pages >= 3);
    }

    #[tokio::test]
    async fn worker_counts_the_full_library_not_just_the_first_page() {
        let store = MemoryStore::new();
        oeuvre_app::seed::seed_demo_library(&store, 5)
            .await
            .unwrap();
        let worker = DbWorker::spawn(AppState::builder(store).surface(Surface::Test).build());
        assert_eq!(worker.list_page(None, 3).await.unwrap().rows.len(), 3);
        let counts = worker.resource_counts().await.unwrap();
        assert_eq!(counts.all_items, 8);
        assert_eq!(counts.by_kind.values().sum::<u64>(), 8);
    }

    #[tokio::test]
    async fn worker_fetches_resource_detail() {
        let store = MemoryStore::new();
        oeuvre_app::seed::seed_demo_library(&store, 5)
            .await
            .unwrap();
        let state = AppState::builder(store).surface(Surface::Test).build();
        let worker = DbWorker::spawn(state);

        let page = worker.list_page(None, 8).await.unwrap();
        // Containers can sort anywhere within the same ULID millisecond, so
        // pick a seeded item; those always have authors.
        let row = page
            .rows
            .iter()
            .find(|row| !row.kind.is_container())
            .expect("seeded library has non-container items");
        let detail = worker.resource_detail(row.id).await.unwrap();
        assert_eq!(detail.id, row.id);
        assert_eq!(detail.kind, row.kind);
        assert_eq!(detail.title.as_str(), row.title.as_str());
        // Summary and detail are different projections of one resource, so
        // the resolved detail must agree with the summary on shared fields.
        assert!(!detail.creators.is_empty());
        assert_eq!(
            published_label_from(detail.published.as_ref()),
            row.published_label
        );
    }
    #[tokio::test]
    async fn worker_walks_query_pages() {
        let store = MemoryStore::new();
        oeuvre_app::seed::seed_demo_library(&store, 5)
            .await
            .unwrap();
        let state = AppState::builder(store).surface(Surface::Test).build();
        let worker = DbWorker::spawn(state);

        // 5 demo rows plus 3 containers: pages of 3 must tile 8 exactly,
        // and the database query must hand back title order across pages.
        let mut offset = 0u64;
        let mut walked: Vec<oeuvre_core::ResourceSummary> = Vec::new();
        loop {
            let page = worker
                .list_query_page(
                    oeuvre_core::ResourceQuery::sorted(
                        oeuvre_core::ResourceSort::Title,
                        oeuvre_core::SortDirection::Ascending,
                    ),
                    offset,
                    3,
                )
                .await
                .unwrap();
            assert!(page.rows.len() <= 3);
            walked.extend(page.rows);
            match page.next_offset {
                // A full page leaves the exact next offset, so the walk
                // never lands on an empty trailing page.
                Some(next) => {
                    assert_eq!(next, offset + 3);
                    offset = next;
                }
                None => break,
            }
        }
        assert_eq!(walked.len(), 8);
        let titles: Vec<String> = walked
            .iter()
            .map(|row| row.title.as_str().to_lowercase())
            .collect();
        let mut expected = titles.clone();
        expected.sort();
        assert_eq!(titles, expected);
    }

    #[tokio::test]
    async fn worker_filters_query_pages_by_title() {
        let store = MemoryStore::new();
        oeuvre_app::seed::seed_demo_library(&store, 5)
            .await
            .unwrap();
        let state = AppState::builder(store).surface(Surface::Test).build();
        let worker = DbWorker::spawn(state);

        // A mixed-case substring must match inside the title, not just its prefix.
        let page = worker
            .list_query_page(
                oeuvre_core::ResourceQuery {
                    sort: None,
                    filter: oeuvre_core::ResourceFilter {
                        search: None,
                        title: Some(oeuvre_core::NonEmptyStr::try_new("nEtWoR").unwrap()),
                        creator: None,
                        published: None,
                        kind: None,
                        favorite: None,
                        tag: None,
                        collection: None,
                    },
                },
                0,
                10,
            )
            .await
            .unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].title.as_str(), "Citation Networks 000");
        assert_eq!(page.next_offset, None);
    }

    #[tokio::test]
    async fn worker_filters_query_pages_by_kind() {
        let store = MemoryStore::new();
        oeuvre_app::seed::seed_demo_library(&store, 5)
            .await
            .unwrap();
        let state = AppState::builder(store).surface(Surface::Test).build();
        let worker = DbWorker::spawn(state);

        // The demo library holds exactly one journal among its articles
        // and books, so a kind-scoped retrieval — the parent picker's
        // narrowing — returns only the parentable kind.
        let page = worker
            .list_query_page(
                oeuvre_core::ResourceQuery {
                    sort: None,
                    filter: oeuvre_core::ResourceFilter::by_kind(
                        oeuvre_core::ResourceKind::Journal,
                    ),
                },
                0,
                10,
            )
            .await
            .unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].title.as_str(), "Journal of Citation Studies");
        assert_eq!(page.rows[0].kind, oeuvre_core::ResourceKind::Journal);
        assert_eq!(page.next_offset, None);
    }

    #[tokio::test]
    async fn worker_filters_query_pages_by_creator() {
        let store = MemoryStore::new();
        oeuvre_app::seed::seed_demo_library(&store, 5)
            .await
            .unwrap();
        let state = AppState::builder(store).surface(Surface::Test).build();
        let worker = DbWorker::spawn(state);

        let people = worker.list_persons().await.unwrap();
        let ada = people
            .iter()
            .find(|person| person.name.display() == "Ada Lovelace")
            .expect("demo people include Ada Lovelace");

        // In the five-row demo cycle Ada authors exactly the n = 0 article
        // (plus its credited alias) and the companion book; containers
        // have no creators. The two rows arrive in seed order, so the
        // assertion reads as a set.
        let page = worker
            .list_query_page(
                oeuvre_core::ResourceQuery {
                    sort: None,
                    filter: oeuvre_core::ResourceFilter {
                        search: None,
                        title: None,
                        creator: Some(ada.id),
                        published: None,
                        kind: None,
                        favorite: None,
                        tag: None,
                        collection: None,
                    },
                },
                0,
                10,
            )
            .await
            .unwrap();
        assert_eq!(page.rows.len(), 2);
        let mut got: Vec<String> = page
            .rows
            .iter()
            .map(|row| row.title.as_str().to_owned())
            .collect();
        got.sort();
        assert_eq!(got, ["Citation Networks 000", "The Reference Companion"]);
        assert_eq!(page.next_offset, None);
    }

    #[tokio::test]
    async fn worker_filters_query_pages_by_published_range() {
        let store = MemoryStore::new();
        oeuvre_app::seed::seed_demo_library(&store, 5)
            .await
            .unwrap();
        let state = AppState::builder(store).surface(Surface::Test).build();
        let worker = DbWorker::spawn(state);

        // The demo cycle stamps years 1962 + n, so a 1962..=1963 window
        // keeps the first two rows; the 1984 companion book stays out.
        let page = worker
            .list_query_page(
                oeuvre_core::ResourceQuery {
                    sort: None,
                    filter: oeuvre_core::ResourceFilter {
                        search: None,
                        title: None,
                        creator: None,
                        published: Some(oeuvre_core::PublishedRange {
                            from: oeuvre_core::UtcMillis::jan_1_utc(1962),
                            to: oeuvre_core::UtcMillis::jan_1_utc(1964),
                        }),
                        kind: None,
                        favorite: None,
                        tag: None,
                        collection: None,
                    },
                },
                0,
                10,
            )
            .await
            .unwrap();
        assert_eq!(page.rows.len(), 2);
        let mut got: Vec<String> = page
            .rows
            .iter()
            .map(|row| row.title.as_str().to_owned())
            .collect();
        got.sort();
        assert_eq!(got, ["Citation Networks 000", "Provenance Studies 001"]);
        assert_eq!(page.next_offset, None);
    }

    #[tokio::test]
    async fn worker_ands_all_three_filters() {
        let store = MemoryStore::new();
        oeuvre_app::seed::seed_demo_library(&store, 5)
            .await
            .unwrap();
        let state = AppState::builder(store).surface(Surface::Test).build();
        let worker = DbWorker::spawn(state);

        let people = worker.list_persons().await.unwrap();
        let ada = people
            .iter()
            .find(|person| person.name.display() == "Ada Lovelace")
            .expect("demo people include Ada Lovelace");

        // Every predicate alone matches several demo rows: "0" is in
        // every demo title, Ada authored two works, and 1962..=1966 spans
        // the whole cycle. Only their intersection names one.
        let page = worker
            .list_query_page(
                oeuvre_core::ResourceQuery {
                    sort: None,
                    filter: oeuvre_core::ResourceFilter {
                        search: None,
                        title: Some(oeuvre_core::NonEmptyStr::try_new("0").unwrap()),
                        creator: Some(ada.id),
                        published: Some(oeuvre_core::PublishedRange {
                            from: oeuvre_core::UtcMillis::jan_1_utc(1962),
                            to: oeuvre_core::UtcMillis::jan_1_utc(1967),
                        }),
                        kind: None,
                        favorite: None,
                        tag: None,
                        collection: None,
                    },
                },
                0,
                10,
            )
            .await
            .unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].title.as_str(), "Citation Networks 000");
        assert_eq!(page.next_offset, None);
    }

    #[tokio::test]
    async fn worker_updates_resource_through_command() {
        let store = MemoryStore::new();
        oeuvre_app::seed::seed_demo_library(&store, 5)
            .await
            .unwrap();
        let state = AppState::builder(store).surface(Surface::Test).build();
        let worker = DbWorker::spawn(state);

        let page = worker.list_page(None, 3).await.unwrap();
        let id = page.rows[0].id;

        // The edit flow's two legs: fetch the raw record, merge a new
        // title into its write-side twin, save, read back the change.
        let record = worker.resource(id).await.unwrap();
        let mut edit = record.to_new();
        edit.title =
            oeuvre_core::NonEmptyStr::try_new("Retitled by worker").expect("title non-empty");
        let updated = worker.update_resource(id, edit).await.unwrap();
        assert_eq!(updated.title.as_str(), "Retitled by worker");

        let again = worker.resource(id).await.unwrap();
        assert_eq!(again.title.as_str(), "Retitled by worker");

        // The listing row the grid shows carries the new title too.
        let fresh = worker.list_page(None, 3).await.unwrap();
        let row = fresh
            .rows
            .iter()
            .find(|row| row.id == id)
            .expect("row kept");
        assert_eq!(row.title.as_str(), "Retitled by worker");

        let err = worker
            .update_resource(ResourceId::generate(), again.to_new())
            .await
            .unwrap_err();
        assert!(err.contains("not found"), "message says not found: {err}");
    }

    #[tokio::test]
    async fn worker_creates_resource_through_command() {
        let store = MemoryStore::new();
        oeuvre_app::seed::seed_demo_library(&store, 2)
            .await
            .unwrap();
        let state = AppState::builder(store).surface(Surface::Test).build();
        let worker = DbWorker::spawn(state);

        let created = worker
            .create_resource(oeuvre_core::NewResource {
                kind: oeuvre_core::ResourceKind::Book,
                title: oeuvre_core::NonEmptyStr::try_new("Inline draft book")
                    .expect("title non-empty"),
                creators: vec![],
                publisher: None,
                published: None,
                accessed: None,
                parent_id: None,
                series_id: None,
                series_number: None,
                tag_ids: vec![],
                collection_ids: vec![],
                metadata: Default::default(),
                subtitle: None,
                title_addon: None,
                short_title: None,
                place: None,
                volume: None,
                issue: None,
                pages: None,
                total_pages: None,
                article_number: None,
                edition: None,
                doi: None,
                url: None,
                eprint: None,
                isbns: vec![],
                issns: vec![],
                isrns: vec![],
                original_date: None,
                event_date: None,
                language: None,
                document_type: None,
                publication_method: None,
                institution: None,
                organization: None,
                school: None,
                abstract_text: None,
                keywords: vec![],
            })
            .await
            .unwrap();
        assert_eq!(created.title.as_str(), "Inline draft book");
        assert!(created.creators.is_empty());

        let again = worker.resource(created.id).await.unwrap();
        assert_eq!(again.title.as_str(), "Inline draft book");
    }
}
