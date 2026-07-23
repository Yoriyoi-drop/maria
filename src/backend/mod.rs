//! Backend — simulator, waveform, coverage, debugger.
//!
//! Sesuai DESIGN.md: backend layer berisi:
//! - simulator/engine.rs — event-driven simulator
//! - simulator/state.rs — signal storage (SoA)
//! - simulator/value.rs — LogicVec evaluation
//! - simulator/scheduler.rs — delta cycle scheduler
//! - simulator/fork_join.rs — fork/join support
//! - simulator/parallel.rs — parallel evaluation
//! - simulator/jit.rs — JIT compilation
//! - waveform/vcd.rs — VCD writer
//! - waveform/fst.rs — FST writer
//! - coverage.rs — coverage engine
//! - debugger.rs — debugger API
//!
//! Migrasi dari src/simulator/ dan src/waveform/ ke struktur baru.
//! Saat ini mere-export module yang sudah ada.

pub mod simulator {
    //! Re-export existing simulator engine and types.
    pub use crate::simulator::SimulationEngine;
    pub use crate::simulator::SimulationState;
    pub use crate::simulator::value::{eval_binary, eval_unary};
    pub use crate::simulator::jit::{JITCompiler, CompiledExpr, JITCache};
    pub use crate::simulator::types::*;
    pub use crate::simulator::parallel::ParallelConfig;
}

pub mod waveform {
    //! Re-export existing waveform writers.
    pub use crate::waveform::VcdWriter;
    pub use crate::waveform::FstWaveWriter;
}

pub use crate::simulator::SimulationEngine;
pub use crate::debugger::Debugger;
