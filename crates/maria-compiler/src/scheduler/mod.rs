//! Scheduler (cluster compiler) — work-stealing task pool, dependency-aware
//! scheduling, incremental tracking, priority queue.
//!
//! Bagian scheduler yang dipakai pipeline kompilasi (frontend/env). Cluster
//! simulasi (sim_dag/clock_domain/cdc) tetap di src/scheduler (crate
//! maria-simulator) karena bergantung pada simulator::parallel.

pub mod dag;
pub mod incremental;
pub mod priority;
pub mod work_stealing;

pub use dag::{DependencyGraph, NodeId};
pub use incremental::IncrementalTracker;
pub use priority::{task_priority, Priority, PriorityQueue};
pub use work_stealing::{Scheduler, Task};
