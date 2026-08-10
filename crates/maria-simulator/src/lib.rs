//! maria-simulator — simulation engine, waveform, scheduler (simulation cluster),
//! debugger, dan VPI.
//!
//! Migrasi monorepo (crate 7): seluruh src/{simulator,waveform,scheduler,debugger,vpi}
//! pindah ke sini. Scheduler PENUH (termasuk sim_dag/clock_domain/cdc) ikut pindah
//! karena sim_dag bergantung pada simulator::parallel (cycle jika terpisah).
//! Cluster kompilasi (dag/incremental/work_stealing/priority) berada di
//! maria-compiler::scheduler; scheduler/mod.rs di sini re-export dari
//! maria_compiler untuk menjaga path lama (`crate::scheduler::Task` dkk) valid.

pub mod simulator;
pub mod waveform;
pub mod scheduler;
pub mod debugger;
pub mod vpi;

#[cfg(test)]
pub mod test_util;
