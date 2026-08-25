//! manifest — metadata cache kategori (Kritik 3 db.md: versioned schema).
//!
//! `manifest.mdb` di root tiap kategori cache menyimpan versi skema, versi
//! compiler, hash konfigurasi (defines/incdirs), waktu, dan ringkasan isi.
//! Skema berubah → kategori dibangun ulang dari kosong (store lama tidak
//! kompatibel).

use serde::{Deserialize, Serialize};

use super::super::verify::now_ns;
use super::super::pipeline_revision;

/// Versi skema lapisan `cache/`. Naikkan bila struktur persistensi kategori
/// berubah (field payload, layout index). Store dengan versi berbeda dianggap
/// tidak kompatibel → di-rebuild (Kritik 3 db.md).
pub const CACHE_SCHEMA_VERSION: u64 = 1;

/// Metadata satu kategori cache, disimpan di `manifest.mdb`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CacheManifest {
    pub schema_version: u64,
    pub compiler_version: String,
    /// Hash konfigurasi build (defines + incdirs) — berubah → hasil berbeda.
    pub config_hash: u64,
    pub created_ns: u64,
    pub updated_ns: u64,
    pub entry_count: u64,
    /// Total bytes payload.
    pub bytes: u64,
}

impl CacheManifest {
    pub fn fresh(config_hash: u64) -> Self {
        CacheManifest {
            schema_version: CACHE_SCHEMA_VERSION,
            compiler_version: pipeline_revision(),
            config_hash,
            created_ns: now_ns(),
            updated_ns: now_ns(),
            entry_count: 0,
            bytes: 0,
        }
    }

    /// Apakah skema kompatibel dengan versi terkini. compiler_version juga
    /// dibandingkan: revisi pipeline efektif berubah (bump manual ATAU
    /// fingerprint binary baru — rebuild cargo apa pun) → seluruh kategori
    /// cache dibangun ulang, mencegah restore hasil lama dari binary baru.
    pub fn valid(&self) -> bool {
        self.schema_version == CACHE_SCHEMA_VERSION && self.compiler_version == pipeline_revision()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fresh_is_valid() {
        let m = CacheManifest::fresh(42);
        assert!(m.valid());
        assert_eq!(m.schema_version, CACHE_SCHEMA_VERSION);
        assert_eq!(m.config_hash, 42);
    }

    #[test]
    fn test_old_schema_invalid() {
        let m = CacheManifest {
            schema_version: CACHE_SCHEMA_VERSION - 1,
            ..CacheManifest::fresh(0)
        };
        assert!(!m.valid());
    }

    #[test]
    fn test_serialize_roundtrip() {
        let m = CacheManifest::fresh(7);
        let bytes = bincode::serialize(&m).unwrap();
        let m2: CacheManifest = bincode::deserialize(&bytes).unwrap();
        assert_eq!(m, m2);
    }
}
