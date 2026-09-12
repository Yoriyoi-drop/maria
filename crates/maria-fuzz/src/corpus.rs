//! Seed corpus: loader direktori + fallback builtin minimal (hanya guard).
//!
//! Dua sumber seed:
//! - `fuzz/corpus/seeds/` — SystemVerilog RTL mandiri (`.sv/.svh/.v/.vh`)
//! - `fuzz/corpus/mv/` — Maria HDL DSL `.mv` (di-transpile on-the-fly ke SV)
//!
//! Seed `.mv` SDK milik maria (MARIA-HDL.md) — di-transpile saat diambil,
//! menjadi source SV yang siap mutasi/evaluasi (sama seperti `maria x.mv`).

use std::path::{Path, PathBuf};

/// Seed corpus. Load sekali per kampanye.
pub struct Corpus {
    seeds: Vec<PathBuf>,
}

/// Source siap-evaluasi: sudah di-transpile bila `.mv`.
pub struct SeedSource {
    /// Source SV siap mutasi/eval (MV sudah di-transpile).
    pub text: String,
    /// Path asal.
    pub path: PathBuf,
    /// Apakah seed asli `.mv` (Maria HDL).
    pub is_mv: bool,
}

impl Corpus {
    /// Load semua seed dari dir. Urut stable (sorted).
    ///
    /// Tanpa `dir` (None): default = `fuzz/corpus/seeds` (SV) + `fuzz/corpus/mv`
    /// (Maria HDL). Dengan `dir` eksplisit: hanya dir itu (cocok utk campain
    /// terarah --corpus).
    pub fn load(dir: Option<&Path>) -> Self {
        let mut seeds = Vec::new();

        match dir {
            // Eksplisit: hanya dir itu.
            Some(d) => {
                Self::push_dir(&mut seeds, d);
            }
            // Default: seeds/ + mv/.
            None => {
                let base = PathBuf::from("crates/maria-fuzz/fuzz/corpus");
                Self::push_dir(&mut seeds, &base.join("seeds"));
                Self::push_dir(&mut seeds, &base.join("mv"));
            }
        }

        seeds.sort();
        seeds.dedup();
        // Guard: reject file terlalu besar.
        seeds.retain(|p| {
            p.metadata().map(|m| m.len() <= 4_000_000).unwrap_or(false)
        });

        Self { seeds }
    }

    fn push_dir(seeds: &mut Vec<PathBuf>, dir: &Path) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() && Self::supported_ext(&p) {
                    seeds.push(p);
                }
            }
        }
    }

    /// Apakah ekstensi file didukung sebagai seed.
    fn supported_ext(p: &Path) -> bool {
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e, "sv" | "svh" | "v" | "vh" | "mv"))
            .unwrap_or(false)
    }

    pub fn is_empty(&self) -> bool {
        self.seeds.is_empty()
    }

    pub fn len(&self) -> usize {
        self.seeds.len()
    }

    /// Ambil seed random, sudah siap-evaluasi (MV di-transpile ke SV).
    pub fn random_seed(&self, rng: &mut crate::Rng) -> Option<SeedSource> {
        if self.seeds.is_empty() {
            return None;
        }
        let p = &self.seeds[rng.below(self.seeds.len())];
        let is_mv = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "mv")
            .unwrap_or(false);
        let raw = std::fs::read_to_string(p).ok()?;
        // Return raw source — untuk `.mv` mutasi harus menyentuh DSL aslinya,
        // transpile dilakukan SETELAH mutasi (di run_single). Transpile di
        // sini membuat mutasi terjadi pada .sv hasil — bukan yang diminta.
        Some(SeedSource {
            text: raw,
            path: p.clone(),
            is_mv,
        })
    }

    /// Ambil seed by index.
    pub fn seed_at(&self, idx: usize) -> Option<&Path> {
        self.seeds.get(idx).map(|v| &**v)
    }
}

/// Transpile `.mv` → SV (svh + sv digabung, baris `` `include `` di-strip).
/// Mirip jalur `maria x.mv` (F9 di main.rs). Pub agar dipakai combine_tb.
pub fn transpile_seed(p: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string(p)?;
    let base = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("design")
        .to_string();
    transpile_mv_string(&raw, &base)
}

/// Transpile `.mv` string → SV (path bebas, dipakai setelah mutasi di MV).
pub fn transpile_mv_string(mv_src: &str, base_name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let tr = maria_api::mv::transpile(mv_src, base_name)?;
    let mut buf = tr.svh.clone();
    buf.push('\n');
    for line in tr.sv.lines() {
        if line.trim_start().starts_with("`include") {
            continue;
        }
        buf.push_str(line);
        buf.push('\n');
    }
    Ok(buf)
}