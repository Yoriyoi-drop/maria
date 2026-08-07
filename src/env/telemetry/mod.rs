//! Telemetry context — hanya MENGAMATI, tidak mengubah perilaku sistem.
//!
//! Profiler (per-fase), metrics (counter atomik), tracing (event log),
//! performance (timing per fase). Semua operasi best-effort.

mod metrics;
mod performance;
mod profiler;
mod telemetry;
mod tracing;

pub use metrics::Metrics;
pub use performance::{PhaseTiming, PhaseTimings};
pub use profiler::ProfilerHandle;
pub use telemetry::TelemetryContext;
pub use tracing::TraceHandle;
