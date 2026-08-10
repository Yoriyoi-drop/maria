//! Statistik diagnostik agregat (error + warning + note) untuk laporan akhir.

use crate::env::diagnostics::{ErrorStats, WarningStats};

/// Ringkasan diagnostik satu run — untuk summary CLI/GUI/JSON.
#[derive(Debug, Default)]
pub struct DiagStatistics {
    pub errors: ErrorStats,
    pub warnings: WarningStats,
    pub notes: std::sync::atomic::AtomicU64,
}

impl DiagStatistics {
    pub fn record_error(&self, kind: &str) {
        self.errors.record(kind);
    }

    pub fn record_warning(&self, kind: &str) {
        self.warnings.record(kind);
    }

    pub fn record_note(&self) {
        self.notes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn total(&self) -> u64 {
        self.errors.total() + self.warnings.total() + self.note_count()
    }

    pub fn note_count(&self) -> u64 {
        self.notes.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn is_clean(&self) -> bool {
        self.errors.total() == 0
    }

    /// Ringkasan satu baris, mis. "3 error, 2 warning, 1 note".
    pub fn summary_line(&self) -> String {
        let mut parts = Vec::new();
        if self.errors.total() > 0 {
            parts.push(format!("{} error", self.errors.total()));
        }
        if self.warnings.total() > 0 {
            parts.push(format!("{} warning", self.warnings.total()));
        }
        if self.note_count() > 0 {
            parts.push(format!("{} note", self.note_count()));
        }
        if parts.is_empty() {
            "bersih".to_string()
        } else {
            parts.join(", ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_statistics() {
        let s = DiagStatistics::default();
        s.record_error("parse");
        s.record_warning("width");
        s.record_note();
        assert_eq!(s.total(), 3);
        assert!(!s.is_clean());
        assert!(s.summary_line().contains("1 error"));
        assert!(s.summary_line().contains("2 warning") || s.summary_line().contains("1 warning"));
    }

    #[test]
    fn test_statistics_clean() {
        let s = DiagStatistics::default();
        assert!(s.is_clean());
        assert_eq!(s.summary_line(), "bersih");
    }
}
