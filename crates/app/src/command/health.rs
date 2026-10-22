use std::time::Instant;

use crate::event::{self, Event};
use crate::{AppState, CommandError, Handler, Outcome};

/// No inputs. Proves AppState → dispatch → store → wide event → sink.
#[derive(Debug, Clone, Default)]
pub struct Health;

#[derive(Debug, Clone, serde::Serialize)]
pub struct HealthReport {
    pub version: &'static str,
    pub uptime_ms: u64,
    pub store_backend: &'static str,
    pub store_ok: bool,
}

impl From<HealthReport> for Outcome {
    fn from(r: HealthReport) -> Self {
        Outcome::Health(r)
    }
}

#[async_trait::async_trait]
impl Handler for Health {
    const NAME: &'static str = "health";
    type Output = HealthReport;

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<HealthReport, CommandError> {
        let t = Instant::now();
        let probe = state.store().ping().await;
        ev.set(event::STORE_LATENCY_US, t.elapsed().as_micros() as u64);
        tracing::trace!(?probe, "store ping returned");
        let h = probe?;
        ev.set(event::STORE_BACKEND, h.backend);
        ev.set(event::STORE_OK, h.ok);
        ev.set(event::APP_UPTIME_MS, state.uptime().as_millis() as u64);
        Ok(HealthReport {
            version: state.version(),
            uptime_ms: state.uptime().as_millis() as u64,
            store_backend: h.backend,
            store_ok: h.ok,
        })
    }
}
