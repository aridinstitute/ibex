#![cfg(feature = "testing")]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use oeuvre_app::command::Health;
use oeuvre_app::testing::{MemorySink, StubStore};
use oeuvre_app::{
    AppState, Command, CommandError, FieldValue, Outcome, Status, Store, StoreError, StoreHealth,
};
use oeuvre_core::{
    Collection, CollectionId, CollectionTreeNode, Name, NewResource, NewSeries, NonEmptyStr,
    Person, PersonId, Resource, ResourceId, Series, SeriesId, Tag, TagId,
};

#[tokio::test]
async fn health_emits_exactly_one_ok_event() {
    let sink = MemorySink::new();
    let state = AppState::builder(StubStore::healthy())
        .sink(sink.clone())
        .build();

    let Outcome::Health(report) = state.execute(Command::Health(Health)).await.unwrap() else {
        panic!("expected Health");
    };
    assert!(report.store_ok);
    assert_eq!(report.store_backend, "stub");

    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.command(), "health");
    assert_eq!(ev.status(), Status::Ok);
    assert!(ev.get("store.latency_us").is_some());
    assert_eq!(ev.get("store.backend"), Some(&FieldValue::from("stub")));
    assert_eq!(ev.get("store.ok"), Some(&FieldValue::from(true)));
}

#[tokio::test]
async fn health_store_error_emits_exactly_one_error_event() {
    let sink = MemorySink::new();
    let state = AppState::builder(StubStore::failing("boom"))
        .sink(sink.clone())
        .build();

    let err = state.execute(Command::Health(Health)).await.unwrap_err();
    assert!(matches!(err, CommandError::Store(_)));

    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.status(), Status::Error);
    assert_eq!(ev.get("error.kind"), Some(&FieldValue::from("store")));
    assert!(ev.get("store.latency_us").is_some());
}

struct HangStore;

#[async_trait::async_trait]
impl Store for HangStore {
    async fn ping(&self) -> Result<StoreHealth, StoreError> {
        std::future::pending().await
    }

    async fn create_person(&self, _name: Name, _aliases: Vec<Name>) -> Result<Person, StoreError> {
        std::future::pending().await
    }

    async fn get_person(&self, _id: PersonId) -> Result<Person, StoreError> {
        std::future::pending().await
    }

    async fn update_person(
        &self,
        _id: PersonId,
        _name: Name,
        _aliases: Vec<Name>,
    ) -> Result<Person, StoreError> {
        std::future::pending().await
    }

    async fn delete_person(&self, _id: PersonId) -> Result<(), StoreError> {
        std::future::pending().await
    }

    async fn find_persons_by_name(&self, _name: &Name) -> Result<Vec<Person>, StoreError> {
        std::future::pending().await
    }

    async fn list_persons(&self) -> Result<Vec<Person>, StoreError> {
        std::future::pending().await
    }

    async fn create_series(&self, _new: NewSeries) -> Result<Series, StoreError> {
        std::future::pending().await
    }

    async fn get_series(&self, _id: SeriesId) -> Result<Series, StoreError> {
        std::future::pending().await
    }

    async fn create_tag(&self, _text: NonEmptyStr) -> Result<Tag, StoreError> {
        std::future::pending().await
    }

    async fn get_tag(&self, _id: TagId) -> Result<Tag, StoreError> {
        std::future::pending().await
    }

    async fn find_tag_by_text(&self, _text: &NonEmptyStr) -> Result<Option<Tag>, StoreError> {
        std::future::pending().await
    }

    async fn list_tags(&self) -> Result<Vec<Tag>, StoreError> {
        std::future::pending().await
    }

    async fn create_collection(
        &self,
        _name: NonEmptyStr,
        _parent: Option<CollectionId>,
    ) -> Result<Collection, StoreError> {
        std::future::pending().await
    }

    async fn get_collection(&self, _id: CollectionId) -> Result<Collection, StoreError> {
        std::future::pending().await
    }

    async fn list_collection_forest(&self) -> Result<Vec<CollectionTreeNode>, StoreError> {
        std::future::pending().await
    }

    async fn add_collection_parent(
        &self,
        _child: CollectionId,
        _parent: CollectionId,
    ) -> Result<(), StoreError> {
        std::future::pending().await
    }

    async fn delete_collection(&self, _id: CollectionId) -> Result<(), StoreError> {
        std::future::pending().await
    }

    async fn collection_descendant_ids(
        &self,
        _id: CollectionId,
    ) -> Result<Vec<CollectionId>, StoreError> {
        std::future::pending().await
    }

    async fn create_resource(&self, _new: NewResource) -> Result<Resource, StoreError> {
        std::future::pending().await
    }

    async fn update_resource(
        &self,
        _id: ResourceId,
        _new: NewResource,
    ) -> Result<Resource, StoreError> {
        std::future::pending().await
    }

    async fn set_resource_favorite(
        &self,
        _id: ResourceId,
        _favorite: bool,
    ) -> Result<Resource, StoreError> {
        std::future::pending().await
    }

    async fn get_resource(&self, _id: ResourceId) -> Result<Resource, StoreError> {
        std::future::pending().await
    }

    async fn list_resources(&self) -> Result<Vec<oeuvre_core::ResourceSummary>, StoreError> {
        std::future::pending().await
    }

    async fn list_resources_page(
        &self,
        _before: Option<ResourceId>,
        _limit: u32,
    ) -> Result<oeuvre_core::ResourcePage, StoreError> {
        std::future::pending().await
    }
}

struct NoopWaker;

impl Wake for NoopWaker {
    fn wake(self: Arc<Self>) {}
}

#[tokio::test]
async fn dropped_execute_emits_cancelled() {
    let sink = MemorySink::new();
    let state = AppState::builder(HangStore).sink(sink.clone()).build();

    let mut fut: Pin<Box<dyn Future<Output = Result<Outcome, CommandError>> + Send>> =
        Box::pin(state.execute(Command::Health(Health)));

    let waker = Waker::from(Arc::new(NoopWaker));
    let mut cx = Context::from_waker(&waker);
    assert!(matches!(fut.as_mut().poll(&mut cx), Poll::Pending));

    drop(fut);

    let events = sink.take();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status(), Status::Cancelled);
}
