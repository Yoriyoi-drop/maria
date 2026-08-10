//! Statistik warning — penghitung warning yang dikumpulkan pipeline.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct WarningStats {
    pub total: AtomicU64,
    pub lint: AtomicU64,
    pub width: AtomicU64,
    pub unused: AtomicU64,
}

impl WarningStats {
    pub fn record(&self, kind: &str) {
        self.total.fetch_add(1, Ordering::Relaxed);
        match kind {
            "lint" => {
                self.lint.fetch_add(1, Ordering::Relaxed);
            }
            "width" => {
                self.width.fetch_add(1, Ordering::Relaxed);
            }
            "unused" => {
                self.unused.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    pub fn snapshot(&self) -> (u64, u64, u64, u64) {
        (
            self.total.load(Ordering::Relaxed),
            self.lint.load(Ordering::Relaxed),
            self.width.load(Ordering::Relaxed),
            self.unused.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_warning_stats() {
        let s = WarningStats::default();
        s.record("lint");
        s.record("width");
        assert_eq!(s.total(), 2);
        assert_eq!(s.snapshot().1, 1);
    }
}
