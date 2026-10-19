use super::{EventSink, FinishedEvent};

/// Default sink: one JSON object per line. stderr by default so MCP stdio
/// keeps stdout for protocol frames.
pub struct JsonLinesSink {
    out: std::sync::Mutex<Box<dyn std::io::Write + Send>>,
}

impl JsonLinesSink {
    pub fn stderr() -> Self {
        Self::new(std::io::stderr())
    }

    pub fn new(w: impl std::io::Write + Send + 'static) -> Self {
        Self {
            out: std::sync::Mutex::new(Box::new(w)),
        }
    }
}

impl EventSink for JsonLinesSink {
    fn emit(&self, event: &FinishedEvent) {
        let Ok(line) = serde_json::to_string(event) else {
            return;
        };
        let mut guard = match self.out.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let _ = writeln!(guard, "{line}");
    }
}
