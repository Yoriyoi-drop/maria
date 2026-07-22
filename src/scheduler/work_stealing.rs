//! Work-Stealing Task Scheduler.
//!
//! Menggunakan crossbeam-deque untuk work-stealing antar threads.
//! Setiap thread punya local queue, dan thread idle bisa steal dari thread lain.

use crossbeam::deque::Worker;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

// ─── Task ───

/// Task types untuk scheduler.
#[derive(Debug, Clone)]
pub enum Task {
    // File-level
    PreprocessFile(String),
    TokenizeFile(String),
    ParseFile(String),
    // Module-level
    TypeCheck(String),
    ElaborateModule(String),
    ResolvePackage(String),
    // Post-processing
    FlattenHierarchy,
    LowerToSimIr,
    // System
    DiagnosticFlush,
    CacheEviction,
    // Custom
    Custom(String),
}

// ─── Task ───

/// Work-stealing task scheduler.
///
/// # Examples
///
/// ```
/// use maria::scheduler::{Scheduler, Task};
///
/// let scheduler = Scheduler::new(2);
/// scheduler.submit(Task::ParseFile("test.sv".into()));
/// scheduler.wait_for_completion();
/// ```
pub struct Scheduler {
    /// Global work-stealing deque
    global_queue: crossbeam::queue::SegQueue<Task>,
    /// Number of worker threads
    num_threads: usize,
    /// Total tasks submitted
    pub total_submitted: AtomicUsize,
    /// Total tasks completed
    pub completed: AtomicUsize,
    /// Pending task count (submitted - completed)
    pending: AtomicUsize,
}

impl Scheduler {
    /// Create a new scheduler with specified number of worker threads.
    pub fn new(num_threads: usize) -> Self {
        Scheduler {
            global_queue: crossbeam::queue::SegQueue::new(),
            num_threads,
            total_submitted: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
            pending: AtomicUsize::new(0),
        }
    }

    /// Create scheduler with number of threads = num_cpus.
    pub fn new_default() -> Self {
        Self::new(num_cpus::get())
    }

    /// Submit a task to the scheduler.
    pub fn submit(&self, task: Task) {
        self.total_submitted.fetch_add(1, Ordering::Relaxed);
        self.pending.fetch_add(1, Ordering::Relaxed);
        self.global_queue.push(task);
    }

    /// Submit tasks in batch (better cache locality).
    pub fn submit_batch(&self, tasks: Vec<Task>) {
        let n = tasks.len();
        self.total_submitted.fetch_add(n, Ordering::Relaxed);
        self.pending.fetch_add(n, Ordering::Relaxed);
        for task in tasks {
            self.global_queue.push(task);
        }
    }

    /// Run the scheduler with worker threads.
    pub fn run(&self) {
        let global = &self.global_queue as *const crossbeam::queue::SegQueue<Task>;
        let completed = &self.completed as *const AtomicUsize;
        let pending = &self.pending as *const AtomicUsize;

        thread::scope(|s| {
            for id in 0..self.num_threads {
                let global = unsafe { &*global };
                let completed = unsafe { &*completed };
                let pending = unsafe { &*pending };
                s.spawn(move || {
                    let worker = Worker::new_fifo();
                    Self::worker_loop(id, &worker, global, completed, pending);
                });
            }
        });
    }

    /// Wait for all tasks to complete.
    pub fn wait_for_completion(&self) {
        self.run();
    }

    /// Get number of pending tasks.
    pub fn pending_count(&self) -> usize {
        self.global_queue.len()
    }

    /// Get number of tasks that have been submitted but not yet completed.
    pub fn pending_tasks(&self) -> usize {
        self.pending.load(Ordering::Relaxed)
    }

    fn worker_loop(
        _id: usize,
        worker: &Worker<Task>,
        global: &crossbeam::queue::SegQueue<Task>,
        completed: &AtomicUsize,
        pending: &AtomicUsize,
    ) {
        loop {
            // 1. Try local queue
            if let Some(task) = worker.pop() {
                Self::execute_task(task, completed, pending);
                continue;
            }

            // 2. Try global queue
            match global.pop() {
                Some(task) => {
                    Self::execute_task(task, completed, pending);
                    continue;
                }
                None => {}
            }

            // 3. No tasks — check if all pending tasks done
            let p = pending.load(Ordering::Acquire);
            if p == 0 {
                break;
            }

            // 4. Yield to avoid busy-wait
            thread::yield_now();
        }
    }

    fn execute_task(task: Task, completed: &AtomicUsize, pending: &AtomicUsize) {
        match task {
            Task::PreprocessFile(_path) => {}
            Task::TokenizeFile(_path) => {}
            Task::ParseFile(_path) => {}
            Task::TypeCheck(_module) => {}
            Task::ElaborateModule(_module) => {}
            Task::ResolvePackage(_pkg) => {}
            Task::FlattenHierarchy => {}
            Task::LowerToSimIr => {}
            Task::DiagnosticFlush => {}
            Task::CacheEviction => {}
            Task::Custom(_) => {}
        }
        completed.fetch_add(1, Ordering::Relaxed);
        pending.fetch_sub(1, Ordering::Release);
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scheduler_new() {
        let scheduler = Scheduler::new(4);
        assert_eq!(scheduler.num_threads, 4);
    }

    #[test]
    fn test_scheduler_submit() {
        let scheduler = Scheduler::new(2);
        scheduler.submit(Task::ParseFile("test.sv".into()));
        assert_eq!(scheduler.pending_count(), 1);
    }

    #[test]
    fn test_scheduler_batch() {
        let scheduler = Scheduler::new(2);
        let tasks: Vec<Task> = (0..10)
            .map(|i| Task::ParseFile(format!("file_{}.sv", i)))
            .collect();
        scheduler.submit_batch(tasks);
        assert_eq!(scheduler.pending_count(), 10);
    }

    #[test]
    fn test_scheduler_run_completes() {
        let scheduler = Scheduler::new(2);
        scheduler.submit(Task::ParseFile("a.sv".into()));
        scheduler.submit(Task::ParseFile("b.sv".into()));
        scheduler.wait_for_completion();
        assert_eq!(scheduler.completed.load(Ordering::Relaxed), 2);
    }
}
