use maria_compiler::scheduler::Task;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Handle ke work-stealing scheduler (dependency-aware task scheduling).
/// Compiler mengirim task tanpa tahu detail antrian/stealing.
#[derive(Clone)]
pub struct SchedulerHandle {
    inner: Arc<maria_compiler::scheduler::Scheduler>,
}

impl fmt::Debug for SchedulerHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SchedulerHandle")
            .field("pending", &self.pending())
            .field("completed", &self.completed())
            .finish()
    }
}

impl SchedulerHandle {
    pub fn new(threads: usize) -> Self {
        SchedulerHandle {
            inner: Arc::new(maria_compiler::scheduler::Scheduler::new(threads.max(1))),
        }
    }

    pub fn submit(&self, task: Task) {
        self.inner.submit(task);
    }

    pub fn submit_batch(&self, tasks: Vec<Task>) {
        self.inner.submit_batch(tasks);
    }

    pub fn pending(&self) -> usize {
        self.inner.pending_tasks()
    }

    pub fn completed(&self) -> usize {
        self.inner.completed.load(Ordering::Relaxed)
    }

    /// Jalankan scheduler sampai semua task selesai.
    pub fn wait(&self) {
        self.inner.wait_for_completion();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scheduler_submit_and_wait() {
        let sched = SchedulerHandle::new(2);
        sched.submit(Task::ParseFile("a.sv".into()));
        sched.submit_batch(vec![
            Task::ParseFile("b.sv".into()),
            Task::ElaborateModule("top".into()),
        ]);
        assert_eq!(sched.pending(), 3);
        sched.wait();
        assert_eq!(sched.completed(), 3);
    }
}
