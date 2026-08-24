use crate::env::telemetry::{Metrics, ProfilerHandle, TraceHandle};

/// TelemetryContext — hanya MENGAMATI, tidak mengubah perilaku sistem.
///
/// Compiler mencatat `telemetry.profiler.record_phase(...)` /
/// `telemetry.metrics.add_files(...)`; telemetry tidak pernah memanggil balik.
pub struct TelemetryContext {
    pub profiler: ProfilerHandle,
    pub tracing: TraceHandle,
    pub metrics: Metrics,
}

impl std::fmt::Debug for TelemetryContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelemetryContext")
            .field("profiler", &self.profiler)
            .field("tracing", &self.tracing)
            .finish()
    }
}

impl TelemetryContext {
    pub fn new() -> Self {
        TelemetryContext {
            profiler: ProfilerHandle::new(),
            tracing: TraceHandle::new(),
            metrics: Metrics::default(),
        }
    }

    /// Telemetry tanpa profiler (mode ringan).
    pub fn light() -> Self {
        TelemetryContext {
            profiler: ProfilerHandle::disabled(),
            tracing: TraceHandle::new(),
            metrics: Metrics::default(),
        }
    }

    pub fn trace(&self, phase: &str, message: &str) {
        self.tracing.trace(phase, message);
    }

    pub fn summary(&self) -> String {
        let m = self.metrics.snapshot();
        format!(
            "builds={} sims={} files={} tokens={} elapsed_ms={}",
            m.0,
            m.1,
            m.2,
            m.3,
            m.4 / 1_000_000,
        )
    }
}

impl Default for TelemetryContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telemetry_default() {
        let t = TelemetryContext::new();
        assert!(t.profiler.is_active());
        assert!(t.summary().contains("builds=0"));
    }
}
