//! Scheduler — work-stealing task pool, dependency-aware scheduling.
//!
//! Phase 1 implementation. Menggunakan crossbeam-deque untuk work-stealing.

pub mod dag;
pub mod incremental;
pub mod priority;
pub mod work_stealing;

pub use dag::{DependencyGraph, NodeId};
pub use incremental::IncrementalTracker;
pub use priority::{task_priority, Priority, PriorityQueue};
pub use work_stealing::{Scheduler, Task};
