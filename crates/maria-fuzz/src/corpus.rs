//! Seed corpus: loader direktori + fallback builtin minimal (hanya guard).
//!
//! Seed diambil dari project RTL NYATA: cva6, openc910, opentitan.
//! Format didukung: .sv, .vh, .svh, .v (target berdasarkan ekstensi).

use std::path::{Path, PathBuf};

/// Seed corpus. Load sekali per kampanye.
pub struct Corpus {
    seeds: Vec<PathBuf>,
}

impl Corpus {
    /// Load semua seed (bukan rekursif) dari dir. Urut stable (sorted).
    ///
    /// Default: `crates/maria-fuzz/fuzz/corpus/seeds`. Ambil juga seed dari
    /// project nyata langsung jika dir kosong/missing (opentitan_rtl.f).
    pub fn load(dir: Option<&Path>) -> Self {
        let mut seeds = Vec::new();

        let dir = dir.map(|d| d.to_path_buf()).unwrap_or_else(|| {
            PathBuf::from("crates/maria-fuzz/fuzz/corpus/seeds")
        });

        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() && Self::supported_ext(&p) {
                    seeds.push(p);
                }
            }
        }

        seeds.sort();
        // Guard: reject file terlalu besar (corpus hanya untuk mutation, bukan sim)
        seeds.retain(|p| {
            p.metadata().map(|m| m.len() <= 4_000_000).unwrap_or(false)
        });

        Self { seeds }
    }

    /// Apakah ekstensi file didukung sebagai seed.
    fn supported_ext(p: &Path) -> bool {
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e, "sv" | "svh" | "v" | "vh"))
            .unwrap_or(false)
    }

    pub fn is_empty(&self) -> bool {
        self.seeds.is_empty()
    }

    pub fn len(&self) -> usize {
        self.seeds.len()
    }

    /// Ambil seed random (isi file).
    pub fn random_seed(&self, rng: &mut crate::Rng) -> Option<String> {
        if self.seeds.is_empty() {
            return None;
        }
        let p = &self.seeds[rng.below(self.seeds.len())];
        std::fs::read_to_string(p).ok()
    }

    /// Ambil seed by index.
    pub fn seed_at(&self, idx: usize) -> Option<&Path> {
        self.seeds.get(idx).map(|v| &**v)
    }
}