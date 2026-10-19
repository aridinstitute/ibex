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

#[tokio::test]
async fn health_emits_exactly_one_ok_event() {
    let sink = MemorySink::new();
    let state = AppState::builder(StubStore::healthy())
        .sink(sink.clone())
        .build();

    let Outcome::Health(report) = state.execute(Command::Health(Health)).await.unwrap();
    assert!(report.store_ok);
    assert_eq!(report.store_backend, "stub");

    let events = sink.take();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.command(), "health");
    assert_eq!(ev.status(), Status::Ok);
    assert!(ev.get("store.latency_us").is_some());
    assert_eq!(
        ev.get("store.backend"),
        Some(&FieldValue::from("stub"))
    );
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
}

struct NoopWaker;

impl Wake for NoopWaker {
    fn wake(self: Arc<Self>) {}
}

#[tokio::test]
async fn dropped_execute_emits_cancelled() {
    let sink = MemorySink::new();
    let state = AppState::builder(HangStore)
        .sink(sink.clone())
        .build();

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
