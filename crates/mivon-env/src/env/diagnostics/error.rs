//! Statistik error — penghitung error yang dikumpulkan pipeline.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct ErrorStats {
    pub total: AtomicU64,
    pub parse: AtomicU64,
    pub semantic: AtomicU64,
    pub elaboration: AtomicU64,
    pub runtime: AtomicU64,
}

impl ErrorStats {
    pub fn record(&self, kind: &str) {
        self.total.fetch_add(1, Ordering::Relaxed);
        match kind {
            "parse" => {
                self.parse.fetch_add(1, Ordering::Relaxed);
            }
            "semantic" => {
                self.semantic.fetch_add(1, Ordering::Relaxed);
            }
            "elaboration" => {
                self.elaboration.fetch_add(1, Ordering::Relaxed);
            }
            "runtime" => {
                self.runtime.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    pub fn any(&self) -> bool {
        self.total() > 0
    }

    pub fn snapshot(&self) -> (u64, u64, u64, u64, u64) {
        (
            self.total.load(Ordering::Relaxed),
            self.parse.load(Ordering::Relaxed),
            self.semantic.load(Ordering::Relaxed),
            self.elaboration.load(Ordering::Relaxed),
            self.runtime.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_stats() {
        let s = ErrorStats::default();
        s.record("parse");
        s.record("parse");
        s.record("elaboration");
        assert_eq!(s.total(), 3);
        assert!(s.any());
        assert_eq!(s.parse.load(Ordering::Relaxed), 2);
        assert_eq!(s.snapshot().0, 3);
    }
}
