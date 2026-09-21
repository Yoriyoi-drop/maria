//! mivon-simulator — simulation engine, waveform, scheduler (simulation cluster),
//! debugger, dan VPI.
//!
//! Migrasi monorepo (crate 7): seluruh src/{simulator,waveform,scheduler,debugger,vpi}
//! pindah ke sini. Scheduler PENUH (termasuk sim_dag/clock_domain/cdc) ikut pindah
//! karena sim_dag bergantung pada simulator::parallel (cycle jika terpisah).
//! Cluster kompilasi (dag/incremental/work_stealing/priority) berada di
//! mivon-compiler::scheduler; scheduler/mod.rs di sini re-export dari
//! mivon_compiler untuk menjaga path lama (`crate::scheduler::Task` dkk) valid.

// Allow large Result Err variant for SimError (intentional — Diagnostic contains spans/files)
#![allow(clippy::result_large_err, clippy::type_complexity)]

pub mod debugger;
pub mod foreign;
pub mod pli;
pub mod scheduler;
pub mod simulator;
pub mod vhpi;
pub mod vpi;
pub mod waveform;

#[cfg(test)]
pub mod test_util;
