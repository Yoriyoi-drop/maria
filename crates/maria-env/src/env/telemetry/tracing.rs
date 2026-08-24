use maria_compiler::profiling::{TraceEvent, Tracer};

/// Handle ke event tracer — catat fase/kejadian untuk debugging.
#[derive(Default)]
pub struct TraceHandle {
    inner: Tracer,
}

impl std::fmt::Debug for TraceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceHandle")
            .field("events", &self.len())
            .finish()
    }
}

impl TraceHandle {
    pub fn new() -> Self {
        TraceHandle {
            inner: Tracer::new(),
        }
    }

    pub fn trace(&self, phase: &str, message: &str) {
        self.inner.trace(phase, message);
    }

    pub fn events(&self) -> Vec<TraceEvent> {
        self.inner.events()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }

    pub fn clear(&self) {
        self.inner.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_handle() {
        let t = TraceHandle::new();
        assert!(t.is_empty());
        t.trace("startup", "env dibangun");
        t.trace("compile", "parse 12 file");
        assert_eq!(t.len(), 2);
        assert_eq!(t.events()[0].phase, "startup");
    }
}
