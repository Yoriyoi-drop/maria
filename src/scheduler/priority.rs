//! Priority Queue — task priority scheduling.
//!
//! Tasks diurutkan berdasarkan priority: filesystem > preprocessing > lexing
//! > parsing > semantic > elaboration.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Mutex;

use super::work_stealing::Task;

// ─── Priority ───

/// Priority levels untuk task scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Low priority — cache eviction, diagnostics flush
    Low = 0,
    /// Normal priority — type checking, elaboration
    Normal = 1,
    /// High priority — parsing, lexing
    High = 2,
    /// Critical priority — filesystem operations
    Critical = 3,
}

impl Default for Priority {
    fn default() -> Self {
        Priority::Normal
    }
}

// ─── Prioritized Task ───

/// Task dengan priority untuk priority queue.
#[derive(Debug, Clone)]
pub struct PrioritizedTask {
    pub task: Task,
    pub priority: Priority,
}

impl PartialEq for PrioritizedTask {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl Eq for PrioritizedTask {}

impl PartialOrd for PrioritizedTask {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PrioritizedTask {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
    }
}

// ─── Priority Queue ───

/// Thread-safe priority queue.
pub struct PriorityQueue {
    heap: Mutex<BinaryHeap<PrioritizedTask>>,
}

impl PriorityQueue {
    pub fn new() -> Self {
        PriorityQueue {
            heap: Mutex::new(BinaryHeap::new()),
        }
    }

    /// Push a task with priority.
    pub fn push(&self, task: Task, priority: Priority) {
        self.heap
            .lock()
            .unwrap()
            .push(PrioritizedTask { task, priority });
    }

    /// Pop the highest-priority task.
    pub fn pop(&self) -> Option<PrioritizedTask> {
        self.heap.lock().unwrap().pop()
    }

    /// Peek at the highest-priority task.
    pub fn peek(&self) -> Option<PrioritizedTask> {
        self.heap.lock().unwrap().peek().cloned()
    }

    /// Number of pending tasks.
    pub fn len(&self) -> usize {
        self.heap.lock().unwrap().len()
    }

    /// Is empty.
    pub fn is_empty(&self) -> bool {
        self.heap.lock().unwrap().is_empty()
    }

    /// Clear all tasks.
    pub fn clear(&self) {
        self.heap.lock().unwrap().clear();
    }
}

impl Default for PriorityQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Map task type to default priority.
pub fn task_priority(task: &Task) -> Priority {
    match task {
        Task::PreprocessFile(_) => Priority::Critical,
        Task::TokenizeFile(_) => Priority::High,
        Task::ParseFile(_) => Priority::High,
        Task::TypeCheck(_) => Priority::Normal,
        Task::ElaborateModule(_) => Priority::Normal,
        Task::ResolvePackage(_) => Priority::Normal,
        Task::FlattenHierarchy => Priority::Low,
        Task::LowerToSimIr => Priority::Low,
        Task::DiagnosticFlush => Priority::Low,
        Task::CacheEviction => Priority::Low,
        Task::Custom(_) => Priority::Normal,
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_priority_ordering() {
        assert!(Priority::Critical > Priority::High);
        assert!(Priority::High > Priority::Normal);
        assert!(Priority::Normal > Priority::Low);
    }

    #[test]
    fn test_priority_queue_push_pop() {
        let pq = PriorityQueue::new();
        pq.push(Task::CacheEviction, Priority::Low);
        pq.push(Task::ParseFile("a.sv".into()), Priority::High);
        pq.push(Task::DiagnosticFlush, Priority::Low);

        let first = pq.pop().unwrap();
        assert_eq!(first.priority, Priority::High);
    }

    #[test]
    fn test_task_priority() {
        assert_eq!(
            task_priority(&Task::PreprocessFile("x.sv".into())),
            Priority::Critical
        );
        assert_eq!(
            task_priority(&Task::ParseFile("x.sv".into())),
            Priority::High
        );
        assert_eq!(task_priority(&Task::CacheEviction), Priority::Low);
    }
}
