use std::sync::{Arc, Mutex};

use oeuvre_core::{Store, StoreError, StoreHealth};

use crate::event::{EventSink, FinishedEvent};

/// Captures finished events in memory. Clone shares the buffer.
#[derive(Clone, Default)]
pub struct MemorySink {
    events: Arc<Mutex<Vec<FinishedEvent>>>,
}

impl MemorySink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn take(&self) -> Vec<FinishedEvent> {
        std::mem::take(&mut *self.events.lock().expect("MemorySink lock"))
    }
}

impl EventSink for MemorySink {
    fn emit(&self, event: &FinishedEvent) {
        self.events
            .lock()
            .expect("MemorySink lock")
            .push(event.clone());
    }
}

pub struct StubStore {
    health: Result<StoreHealth, String>,
}

impl StubStore {
    pub fn healthy() -> Self {
        Self {
            health: Ok(StoreHealth {
                backend: "stub",
                ok: true,
            }),
        }
    }

    pub fn failing(msg: &str) -> Self {
        Self {
            health: Err(msg.to_string()),
        }
    }
}

#[async_trait::async_trait]
impl Store for StubStore {
    async fn ping(&self) -> Result<StoreHealth, StoreError> {
        match &self.health {
            Ok(h) => Ok(h.clone()),
            Err(msg) => Err(StoreError::new(msg.clone())),
        }
    }
}
