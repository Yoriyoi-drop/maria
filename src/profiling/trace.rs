//! Tracing events — lightweight event logging untuk debugging profiling.

use std::sync::Mutex;
use std::time::Instant;

/// A single trace event.
#[derive(Debug, Clone)]
pub struct TraceEvent {
    pub timestamp_ns: u64,
    pub phase: String,
    pub message: String,
    pub thread_id: usize,
}

/// Simple trace collector.
pub struct Tracer {
    events: Mutex<Vec<TraceEvent>>,
    start: Instant,
}

impl Tracer {
    pub fn new() -> Self {
        Tracer {
            events: Mutex::new(Vec::new()),
            start: Instant::now(),
        }
    }

    /// Record a trace event.
    pub fn trace(&self, phase: &str, message: &str) {
        let event = TraceEvent {
            timestamp_ns: self.start.elapsed().as_nanos() as u64,
            phase: phase.to_string(),
            message: message.to_string(),
            thread_id: {
                // Use a simple hash of thread id as usize
                let id = std::thread::current().id();
                format!("{:?}", id).bytes().fold(0usize, |acc, b| {
                    acc.wrapping_mul(31).wrapping_add(b as usize)
                })
            },
        };
        self.events.lock().unwrap().push(event);
    }

    /// Get all collected events.
    pub fn events(&self) -> Vec<TraceEvent> {
        self.events.lock().unwrap().clone()
    }

    /// Number of events.
    pub fn len(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    /// Clear events.
    pub fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
}

impl Default for Tracer {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracer() {
        let tracer = Tracer::new();
        tracer.trace("lex", "tokenizing file.sv");
        tracer.trace("parse", "parsing file.sv");

        let events = tracer.events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].phase, "lex");
        assert_eq!(events[1].phase, "parse");
    }
}
