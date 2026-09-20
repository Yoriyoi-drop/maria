//! Metrics — counter atomik ringan untuk observability (tanpa lock).
//!
//! Hanya mengamati; tidak memengaruhi perilaku sistem.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct Metrics {
    pub builds: AtomicU64,
    pub simulations: AtomicU64,
    pub files_parsed: AtomicU64,
    pub tokens_lexed: AtomicU64,
    pub total_elapsed_ns: AtomicU64,
}

impl Metrics {
    pub fn inc_build(&self) {
        self.builds.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_simulation(&self) {
        self.simulations.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_files(&self, n: u64) {
        self.files_parsed.fetch_add(n, Ordering::Relaxed);
    }

    pub fn add_tokens(&self, n: u64) {
        self.tokens_lexed.fetch_add(n, Ordering::Relaxed);
    }

    pub fn add_elapsed(&self, ns: u64) {
        self.total_elapsed_ns.fetch_add(ns, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> (u64, u64, u64, u64, u64) {
        (
            self.builds.load(Ordering::Relaxed),
            self.simulations.load(Ordering::Relaxed),
            self.files_parsed.load(Ordering::Relaxed),
            self.tokens_lexed.load(Ordering::Relaxed),
            self.total_elapsed_ns.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics() {
        let m = Metrics::default();
        m.inc_build();
        m.inc_simulation();
        m.add_files(10);
        m.add_tokens(100);
        assert_eq!(m.snapshot(), (1, 1, 10, 100, 0));
    }
}
