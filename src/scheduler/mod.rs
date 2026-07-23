//! Scheduler — work-stealing task pool, dependency-aware scheduling.
//!
//! Phase 1 implementation. Menggunakan crossbeam-deque untuk work-stealing.

pub mod dag;
pub mod incremental;
pub mod priority;
pub mod clock_domain;
pub mod sim_dag;
pub mod work_stealing;

pub use clock_domain::{ClockDomainAnalysis, ClockDomain, ClockEdgeType};
pub use dag::{DependencyGraph, NodeId};
pub use incremental::IncrementalTracker;
pub use priority::{task_priority, Priority, PriorityQueue};
pub use sim_dag::{evaluate_layer_parallel, is_process_parallelizable, layer_to_string, SimulationDag};
pub use work_stealing::{Scheduler, Task};
