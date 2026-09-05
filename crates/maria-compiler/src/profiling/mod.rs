//! Profiling — built-in profiler, performance counters.
//!
//! Phase 5 implementation. Thread-safe, lock-free counters.

pub mod counters;
pub mod dashboard;
pub mod flamegraph;
pub mod profiler;
pub mod trace;

pub use counters::{AtomicCounters, CounterType};
pub use dashboard::{
    CompileMetrics, CoverageMetrics, MetricsReport, PerfDashboard, SimPerfCounters,
};
pub use flamegraph::FlamegraphProfiler;
pub use profiler::{Counter, Phase, PhaseTimer, ProfileReport, Profiler};
pub use trace::{TraceEvent, Tracer};
