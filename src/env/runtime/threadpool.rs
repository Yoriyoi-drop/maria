/// Handle thread pool (rayon global). Compiler memanggil `spawn(...)` tanpa
/// tahu detail implementasi thread — cukup tanya `runtime.thread_pool.spawn(f)`.
#[derive(Debug, Clone)]
pub struct ThreadPoolHandle {
    threads: usize,
}

impl ThreadPoolHandle {
    pub fn new(threads: usize) -> Self {
        ThreadPoolHandle { threads: threads.max(1) }
    }

    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Spawn task fire-and-forget ke rayon global pool.
    pub fn spawn<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        rayon::spawn(f);
    }

    /// Spawn FIFO (menjaga urutan antar spawn pada thread sama).
    pub fn spawn_fifo<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        rayon::spawn_fifo(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn test_spawn_runs_task() {
        let pool = ThreadPoolHandle::new(4);
        assert_eq!(pool.threads(), 4);
        let (tx, rx) = mpsc::channel();
        pool.spawn(move || {
            let _ = tx.send(42);
        });
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)), Ok(42));
    }

    #[test]
    fn test_threads_min_one() {
        let pool = ThreadPoolHandle::new(0);
        assert_eq!(pool.threads(), 1);
    }
}
