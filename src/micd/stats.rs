//! stats.mdb — statistics database (Kritik 14 db.md).
//!
//! Mencatat profil tiap build: timing per fase (lexer/parser/elab/optimize/
//! verify/save), memory puncak, cache hit/miss, dirty node, dsb. Dari sini
//! Maria bisa mengenali bottleneck sendiri dan memberi rekomendasi. Dibuat
//! append-only dengan batas profil maksimum (GC sederhana: buang tertua).

use serde::{Deserialize, Serialize};

use super::verify::now_ns;

/// Profil satu build.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BuildProfile {
    /// Nomor build (incrementing).
    pub build_id: u64,
    /// Total waktu compile (ms).
    pub total_ms: u64,
    pub preprocess_ms: u64,
    pub lex_ms: u64,
    pub parse_ms: u64,
    pub elaborate_ms: u64,
    pub optimize_ms: u64,
    pub verify_ms: u64,
    pub save_ms: u64,
    /// Jumlah file terdaftar.
    pub files: usize,
    /// File berubah pada build ini.
    pub changed_files: usize,
    /// Node kotor (dirty) — file yang perlu dibangun ulang via dep graph.
    pub dirty_nodes: usize,
    /// AST yang di-restore (parse di-skip).
    pub restored_designs: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    /// Peak RSS (VmAWM / KiB).
    pub peak_mem_kb: u64,
    /// ID snapshot yang dibuat build ini (0 = tidak ada).
    pub snapshot_id: u64,
    /// Waktu pencatatan (unix ns).
    pub verified_at_ns: u64,
}

/// Statistics database — kumpulan profil build berurutan.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatsDb {
    pub profiles: Vec<BuildProfile>,
    /// Maksimal profil yang disimpan (GC: buang paling tua).
    pub max_profiles: usize,
}

impl StatsDb {
    pub fn new() -> Self {
        StatsDb {
            profiles: Vec::new(),
            max_profiles: 256,
        }
    }

    /// Catat profil build. Bila melebihi batas, profil tertua dibuang.
    pub fn record(&mut self, p: BuildProfile) {
        self.profiles.push(p);
        while self.profiles.len() > self.max_profiles {
            self.profiles.remove(0);
        }
    }

    /// Banyak build tercatat.
    pub fn total_builds(&self) -> usize {
        self.profiles.len()
    }

    /// Profil build terakhir.
    pub fn last(&self) -> Option<&BuildProfile> {
        self.profiles.last()
    }

    /// Rata-rata total waktu compile dari seluruh profil tersimpan.
    pub fn avg_total_ms(&self) -> u64 {
        if self.profiles.is_empty() {
            return 0;
        }
        let sum: u64 = self.profiles.iter().map(|p| p.total_ms).sum();
        sum / self.profiles.len() as u64
    }

    /// Cache hit rate keseluruhan (0..100).
    pub fn hit_rate_pct(&self) -> u8 {
        let hits: usize = self.profiles.iter().map(|p| p.cache_hits).sum();
        let misses: usize = self.profiles.iter().map(|p| p.cache_misses).sum();
        let total = hits + misses;
        if total == 0 {
            return 0;
        }
        ((hits as f64 / total as f64) * 100.0) as u8
    }

    /// Buat profil kosong dengan build_id lanjutan.
    pub fn next_profile(&self) -> BuildProfile {
        let mut p = BuildProfile::default();
        p.build_id = self.profiles.last().map(|l| l.build_id + 1).unwrap_or(1);
        p.verified_at_ns = now_ns();
        p
    }
}

/// Peak RSS (VmHWM) dalam KiB dari `/proc/self/status`. Non-Linux → 0.
pub fn peak_rss_kb() -> u64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    status
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BuildProfile {
        let mut p = BuildProfile::default();
        p.build_id = 1;
        p.total_ms = 100;
        p.lex_ms = 10;
        p.files = 5;
        p.cache_hits = 4;
        p.cache_misses = 1;
        p
    }

    #[test]
    fn test_record_and_bounds() {
        let mut db = StatsDb::new();
        db.max_profiles = 3;
        for i in 0..5u64 {
            let mut p = sample();
            p.build_id = i + 1;
            db.record(p);
        }
        assert_eq!(db.total_builds(), 3);
        assert_eq!(db.last().unwrap().build_id, 5);
    }

    #[test]
    fn test_avg_and_hit_rate() {
        let mut db = StatsDb::new();
        let mut p1 = sample();
        p1.total_ms = 50;
        p1.cache_hits = 1;
        p1.cache_misses = 1;
        db.record(p1);
        let mut p2 = sample();
        p2.total_ms = 150;
        p2.cache_hits = 3;
        p2.cache_misses = 1;
        db.record(p2);
        assert_eq!(db.avg_total_ms(), 100);
        assert_eq!(db.hit_rate_pct(), 66);
    }

    #[test]
    fn test_next_profile_increments() {
        let mut db = StatsDb::new();
        assert_eq!(db.next_profile().build_id, 1);
        db.record(sample());
        assert_eq!(db.next_profile().build_id, 2);
    }

    #[test]
    fn test_serialize_roundtrip() {
        let mut db = StatsDb::new();
        db.record(sample());
        let bytes = bincode::serialize(&db).unwrap();
        let db2: StatsDb = bincode::deserialize(&bytes).unwrap();
        assert_eq!(db2.total_builds(), 1);
        assert_eq!(db2.last().unwrap().total_ms, 100);
    }
}
