//! stats — statistik cache per kategori (db.md "stats/: statistik hit/miss,
//! ukuran, dan umur cache").
//!
//! Setiap kategori melacak hit/miss runtime, jumlah entry, bytes, dan umur
//! entry tertua/terbaru (untuk TTL/GC). Aggregasi lintas kategori untuk
//! laporan (mprof/mbench).

use crate::micd::cache::CacheCategory;

/// Statistik satu kategori cache.
#[derive(Debug, Clone, Copy)]
pub struct CategoryStats {
    pub category: CacheCategory,
    pub entries: usize,
    /// Total bytes payload.
    pub bytes: u64,
    pub hits: u64,
    pub misses: u64,
    /// Waktu akses entry tertua (ns).
    pub oldest_ns: Option<u64>,
    /// Waktu akses entry terbaru (ns).
    pub newest_ns: Option<u64>,
    /// Store di-rebuild (schema mismatch) pada sesi ini.
    pub rebuilt: bool,
}

impl Default for CategoryStats {
    fn default() -> Self {
        CategoryStats {
            category: CacheCategory::Preprocess,
            entries: 0,
            bytes: 0,
            hits: 0,
            misses: 0,
            oldest_ns: None,
            newest_ns: None,
            rebuilt: false,
        }
    }
}

impl CategoryStats {
    /// Hit rate 0..100 (0 bila belum ada akses).
    pub fn hit_rate_pct(&self) -> u8 {
        let total = self.hits + self.misses;
        if total == 0 {
            return 0;
        }
        ((self.hits as f64 / total as f64) * 100.0) as u8
    }
}

/// Statistik keseluruhan lapisan `cache/`.
#[derive(Debug, Clone, Default)]
pub struct CacheLayerStats {
    pub per_category: Vec<CategoryStats>,
    pub total_entries: usize,
    pub total_bytes: u64,
    pub total_hits: u64,
    pub total_misses: u64,
    pub stores: usize,
    /// Store yang di-rebuild karena schema mismatch (Kritik 3 db.md).
    pub rebuilt: usize,
}

impl CacheLayerStats {
    pub fn hit_rate_pct(&self) -> u8 {
        let total = self.total_hits + self.total_misses;
        if total == 0 {
            return 0;
        }
        ((self.total_hits as f64 / total as f64) * 100.0) as u8
    }

    /// Ringkasan satu baris untuk tool/CLI.
    pub fn summary(&self) -> String {
        format!(
            "categories={} entries={} bytes={} hit={}% rebuilt={}",
            self.stores,
            self.total_entries,
            self.total_bytes,
            self.hit_rate_pct(),
            self.rebuilt,
        )
    }

    /// Statistik kategori tertentu.
    pub fn category(&self, cat: CacheCategory) -> Option<&CategoryStats> {
        self.per_category.iter().find(|s| s.category == cat)
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hit_rate() {
        let mut s = CategoryStats::default();
        assert_eq!(s.hit_rate_pct(), 0);
        s.hits = 3;
        s.misses = 2;
        assert_eq!(s.hit_rate_pct(), 60);
    }

    #[test]
    fn test_layer_summary_and_lookup() {
        let mut l = CacheLayerStats::default();
        l.stores = 2;
        l.total_entries = 10;
        l.total_bytes = 500;
        l.total_hits = 4;
        l.total_misses = 1;
        l.per_category.push(CategoryStats {
            category: CacheCategory::Parser,
            entries: 7,
            ..Default::default()
        });
        assert_eq!(l.hit_rate_pct(), 80);
        assert!(l.summary().contains("entries=10"));
        assert!(l.category(CacheCategory::Parser).is_some());
        assert!(l.category(CacheCategory::Lexer).is_none());
    }
}
