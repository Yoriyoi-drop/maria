//! Bug database persisten lintas-kampanye.
//!
//! Paper #18 (Trippel et al., "Fuzzing Hardware Like Software"): seed dari
//! bug yang ditemukan di kampanye sebelumnya dire-seed ulang ke kampanye baru
//! supaya regressi terus diexercise (bug database = prioritas coverage).
//!
//! Implementasi: JSON sederhana (serde_json) di `BugDb::save`/`load`. Dipanggil
//! `run_fuzz` sekali: load bug lama → re-seed seed awal (prevent regressi hilang).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::BugKind;
use crate::BugRecord;

/// Lock global akses file bug DB — worker paralel (main.rs --workers)
/// membaca & menulis path yang sama; tanpa lock, load/save bisa ter-interleave
/// dan save terakhir menimpa entri worker lain (last-writer-wins).
static BUGDB_LOCK: Mutex<()> = Mutex::new(());

/// Satu entri bug database — identik dengan `BugRecord` tapi ada field metadata
/// kampanye (seed, iterasi, waktu) untuk rujukan regressi.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BugDbEntry {
    pub kind: BugKind,
    pub source: String,
    pub detail: String,
    /// Seed RNG kampanye yang menemukan bug ini.
    pub seed: u64,
    /// Iterasi milestone (iterasi pertama kali menemukan bug). 0 = unknown.
    pub iter: u64,
    /// Timestamp Unix saat ditemukan (detik).
    pub t: u64,
}

/// Database bug yang persisten di disk (JSON, satu file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BugDb {
    /// Semua bug yang ditemukan (berisi entri + duplicate raw bug yang
    /// sebelumnya tidak ada di DB → di-append).
    pub entries: Vec<BugDbEntry>,
}

impl Default for BugDb {
    fn default() -> Self {
        BugDb { entries: Vec::new() }
    }
}

impl BugDb {
    /// Kosong — tanpa file DB (fresh campaign).
    pub fn empty() -> Self {
        BugDb::default()
    }

    /// Muat DB dari path. None bila file tidak ada (kampanye pertama).
    pub fn load(path: &Path) -> Option<Self> {
        let _g = BUGDB_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        Self::load_locked(path)
    }

    /// Muat tanpa lock (internal — pemanggil wajib memegang BUGDB_LOCK).
    fn load_locked(path: &Path) -> Option<Self> {
        let raw = std::fs::read(path).ok()?;
        let db: BugDb = serde_json::from_slice(&raw).ok()?;
        Some(db)
    }

    /// Simpan DB ke path — MERGE-SAFE lintas worker paralel: entri yang sudah
    /// ada di disk digabung (dedup by kind+source+detail). Tanpa merge, tiap
    /// worker menimpa file dengan bug miliknya sendiri (last-writer-wins).
    /// Tulis atomik (temp + rename) di bawah lock global.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let _g = BUGDB_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut merged = self.clone();
        if let Some(existing) = Self::load_locked(path) {
            for e in existing.entries {
                let dup = merged.entries.iter().any(|m| {
                    m.kind == e.kind && m.source == e.source && m.detail == e.detail
                });
                if !dup {
                    merged.entries.push(e);
                }
            }
        }
        let tmp = path.with_extension("json.tmp");
        let serialized = serde_json::to_vec(&merged)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(&tmp, &serialized)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    /// Append satu bug ke DB (clone isi). Backend MvMediated: simpan canonical
    /// MV sebagai source re-seed (bukan HDL minimized) — bug db = corpus MV
    /// lintas kampanye (Paper #18; task §8/§9 reproducible).
    pub fn push(&mut self, bug: &BugRecord, seed: u64, iter: u64) {
        let src = bug.mv.clone().unwrap_or_else(|| bug.source.clone());
        self.entries.push(BugDbEntry {
            kind: bug.kind.clone(),
            source: src,
            detail: bug.detail.clone(),
            seed,
            iter,
            t: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        });
    }

    /// Re-seed: ekstrak hanya `source` dari entri (bug-minimal) untuk
    /// dimasukkan ke seed corpus kampanye baru. Paper #18: bug sebelumnya
    /// dire-seed → regressi terus diexercise.
    pub fn reseed_sources(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.source.clone()).collect()
    }

    /// Jumlah entri.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Helper: path DB dari direktori emit-bugs atau default.
/// Seharusnya `bugdb.json` di direktori campaign yang sama dengan emit bugs.
pub fn db_path(emit_dir: &Path) -> PathBuf {
    emit_dir.join("bugdb.json")
}
