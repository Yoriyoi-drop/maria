//! Simulation context — kernel, event queue, waveform, dpi, coverage.
//!
//! Diagram (doc/env.md): Simulator → Kernel → Event Queue → Time Wheel →
//! Waveform → Coverage → DPI.

mod coverage;
mod dpi;
mod event_queue;
mod kernel;
mod simulation;
mod waveform;

pub use coverage::{attach_coverage_db, CoverageStats};
pub use dpi::DpiInfo;
pub use event_queue::EventQueueStats;
pub use kernel::{KernelOptions, SimulationKernel};
pub use simulation::SimulationContext;
pub use waveform::{WaveformOptions, attach_waveforms};
