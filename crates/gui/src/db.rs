//! Store access for the UI: one OS thread owns a tokio runtime and serves
//! page requests over channels, so gpui's executor never runs Surreal I/O.

use oeuvre_app::command::{
    GetResourceDetail, ListPersons, ListResourcesPage, ListResourcesQueryPage,
};
use oeuvre_app::{AppState, Command, Outcome};
use oeuvre_core::{
    Person, ResourceDetail, ResourceId, ResourcePage, ResourceQuery, SortedResourcePage,
};
use tokio::sync::oneshot;

pub enum DbRequest {
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
    GetDetail {
        id: ResourceId,
        reply: oneshot::Sender<Result<ResourceDetail, String>>,
    },
    ListPersons {
        reply: oneshot::Sender<Result<Vec<Person>, String>>,
    },
}

#[derive(Clone)]
pub struct DbWorker {
    tx: std::sync::mpsc::Sender<DbRequest>,
}

impl DbWorker {
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
                            DbRequest::ListPage {
                                before,
                                limit,
                                reply,
                            } => {
                                let result = state
                                    .execute(Command::ListResourcesPage(ListResourcesPage {
                                        before,
                                        limit,
                                    }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::ListResourcesPage(page) => Ok(page),
                                        _ => Err(
                                            "unexpected outcome from list_resources_page".into(),
                                        ),
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
                                        _ => Err(
                                            "unexpected outcome from list_resources_query_page"
                                                .into(),
                                        ),
                                    });
                                let _ = reply.send(result);
                            }
                            DbRequest::GetDetail { id, reply } => {
                                let result = state
                                    .execute(Command::GetResourceDetail(GetResourceDetail {
                                        id,
                                    }))
                                    .await
                                    .map_err(|e| e.to_string())
                                    .and_then(|outcome| match outcome {
                                        Outcome::GetResourceDetail(detail) => Ok(detail),
                                        _ => Err(
                                            "unexpected outcome from get_resource_detail".into(),
                                        ),
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
                        }
                    }
                });
            })
            .expect("spawn db worker thread");
        Self { tx }
    }

    /// Request a page and wait for the reply off the UI thread.
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

    /// Fetch every person for the creator filter's dropdown.
    pub async fn list_persons(&self) -> Result<Vec<Person>, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::ListPersons { reply: tx })
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

    /// Same request for the window bootstrap, where no async context exists
    /// on the main thread yet.
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

#[cfg(test)]
mod tests {
    use super::*;
    use oeuvre_app::Surface;
    use oeuvre_app::testing::MemoryStore;
    use oeuvre_core::published_label_from;

    #[tokio::test]
    async fn worker_walks_pages() {
        let store = MemoryStore::new();
        oeuvre_app::seed::seed_demo_library(&store, 5).await.unwrap();
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
    async fn worker_fetches_resource_detail() {
        let store = MemoryStore::new();
        oeuvre_app::seed::seed_demo_library(&store, 5).await.unwrap();
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
        oeuvre_app::seed::seed_demo_library(&store, 5).await.unwrap();
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
        oeuvre_app::seed::seed_demo_library(&store, 5).await.unwrap();
        let state = AppState::builder(store).surface(Surface::Test).build();
        let worker = DbWorker::spawn(state);

        // Exactly one seeded row carries "Networks" in its title.
        let page = worker
            .list_query_page(
                oeuvre_core::ResourceQuery {
                    sort: None,
                    filter: oeuvre_core::ResourceFilter {
                        title: Some(oeuvre_core::NonEmptyStr::try_new("Networks").unwrap()),
                        creator: None,
                        published: None,
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
    async fn worker_filters_query_pages_by_creator() {
        let store = MemoryStore::new();
        oeuvre_app::seed::seed_demo_library(&store, 5).await.unwrap();
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
                        title: None,
                        creator: Some(ada.id),
                        published: None,
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
        oeuvre_app::seed::seed_demo_library(&store, 5).await.unwrap();
        let state = AppState::builder(store).surface(Surface::Test).build();
        let worker = DbWorker::spawn(state);

        // The demo cycle stamps years 1962 + n, so a 1962..=1963 window
        // keeps the first two rows; the 1984 companion book stays out.
        let page = worker
            .list_query_page(
                oeuvre_core::ResourceQuery {
                    sort: None,
                    filter: oeuvre_core::ResourceFilter {
                        title: None,
                        creator: None,
                        published: Some(oeuvre_core::PublishedRange {
                            from: oeuvre_core::UtcMillis::jan_1_utc(1962),
                            to: oeuvre_core::UtcMillis::jan_1_utc(1964),
                        }),
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

}
