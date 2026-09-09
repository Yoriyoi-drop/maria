//! Corpus seed SV nyata + minimizer.
//!
//! Paper #12 (Holler et al., "Fuzzing with Code Fragments"): mutasi efektif
//! datang dari serpihan kode *nyata*, bukan literal acak. Corpus diambil
//! dari direktori proyek (test/, opentitan/, cva6/, …) lewat `--corpus-dir`,
//! atau dari FILELIST (`.f`/`.maria` — mendukung proyek nyata penuh).
//! Default: auto-detect SV files dari project root jika tidak ada corpus-dir.
//! `minimize` = reduksi input bug ke bentuk minimal (OSS-Fuzz-style).
//!
//! DEEP CHANGE (GAP-11): file >64KB sebelumnya di-buang (fuzzer hanya
//! melihat snippet permukaan). File besar membawa KEDALAMAN struktural nyata
//! (generate, interface, package, hierarchy 3+ level) — biang bug deep.
//! Limit naik ke `MAX_SEED_BYTES`; korpus real diambil dari filelist
//! (`from_filelist`) sehingga file yang sama di-compile penuh menjadi seed.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rand::rngs::StdRng;
use serde::{Deserialize, Serialize};

use rand::seq::SliceRandom;
use rand::Rng;

/// Limit ukuran file seed (AUDIT GAP-11: naik dari 64KB → 1MB agar file RTL
/// nyata dengan modul dalam — generate/interface/param — masuk corpus.
/// File >1MB sangat jarang dan terlalu mahal utk mutasi; dilewati).
pub const MAX_SEED_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Corpus {
    pub seeds: Vec<String>,
}

impl Corpus {
    pub fn empty() -> Self {
        Corpus::default()
    }

    /// Muat SV files dari direktori project otomatis jika tidak ada corpus-dir.
    /// Cari di lokasi umum: test/, opentitan/, crates/*/.
    fn auto_corpus_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        // Project root-based paths
        let candidates = [
            Path::new("test"),
            Path::new("examples"),
            Path::new("fuzz"),
            Path::new("opentitan"),
            Path::new("cva6"),
            Path::new("crates"),
        ];
        for c in &candidates {
            if c.exists() {
                dirs.push(c.to_path_buf());
            }
        }
        dirs
    }

    /// Muat semua file `.sv` dari direktori (rekursif). File besar (>64 KB)
    /// dilewati — fuzzer menyukai seed kecil (Paper #12: unit mutasi).
    /// Jika dirs kosong, auto-detect dari project root.
    pub fn from_dirs(dirs: &[PathBuf]) -> Self {
        let mut seeds: Vec<String> = Vec::new();
        let effective_dirs: Vec<PathBuf> = if dirs.is_empty() {
            Self::auto_corpus_dirs()
        } else {
            dirs.iter().map(|d| d.to_path_buf()).collect()
        };
        for dir in &effective_dirs {
            collect_sv(dir, &mut seeds);
        }
        // Sample awal: kumpulkan semua seed tapi batasi memori.
        // Jika terlalu banyak, ambil sampel acak (fix seed untuk deterministik).
        if seeds.len() > 500 {
            let mut sampled = Vec::with_capacity(500);
            let mut idx = 0usize;
            let step = seeds.len() / 500;
            for _ in 0..500 {
                if idx < seeds.len() {
                    sampled.push(seeds[idx].clone());
                }
                idx += step;
            }
            seeds = sampled;
        }
        Corpus { seeds }
    }

    pub fn len(&self) -> usize {
        self.seeds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seeds.is_empty()
    }

    /// Muat SV files dari FILELIST (`.f` / `.maria`): satu path per baris,
    /// `#` komentar. File TIDAK di-skip berdasarkan ukuran (DEEP GAP-11) —
    /// file besar membawa kedalaman struktural nyata. Batasi total seed agar
    /// memori wajar (sampel deterministik stride jika > `cap`).
    pub fn from_filelist(path: &Path, cap: usize) -> Self {
        let mut files: Vec<String> = Vec::new();
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                let l = line.trim();
                if l.is_empty() || l.starts_with('#') {
                    continue;
                }
                // Path relatif terhadap direktori filelist.
                let full = if Path::new(l).is_absolute() {
                    PathBuf::from(l)
                } else {
                    path.parent().unwrap_or(Path::new(".")).join(l)
                };
                files.push(full.to_string_lossy().to_string());
            }
        }
        let mut seeds: Vec<String> = Vec::new();
        for f in &files {
            if seeds.len() >= cap {
                break;
            }
            if let Ok(content) = std::fs::read_to_string(f) {
                if content.contains("module ") || content.contains("interface ") {
                    seeds.push(content);
                }
            }
        }
        Corpus { seeds }
    }

    /// Serpihan acak (fragment) dari corpus — bahan splice mutasi (#12).
    ///
    /// DEEP CHANGE (GAP-11): sebelumnya 1-5 baris (permukaan). Kini dengan
    /// probabilitas 50% mengambil blok STRUKTURAL UTUH (`module ... endmodule`,
    /// `interface ... endinterface`, `package ... endpackage`, `always_* begin
    /// ... end` bersarang) sehingga splice membawa kedalaman generate/interface/
    /// hierarchy — bukan baris lepas.
    pub fn random_fragment(&self, rng: &mut StdRng) -> Option<String> {
        if self.seeds.is_empty() {
            return None;
        }
        // 50%: blok struktural utuh.
        if rng.gen_bool(0.5) {
            if let Some(block) = self.random_block(rng) {
                return Some(block);
            }
        }
        let seed = self.seeds.choose(rng)?;
        let lines: Vec<&str> = seed.lines().collect();
        if lines.is_empty() {
            return None;
        }
        let start = rng.gen_range(0..lines.len());
        let n = rng.gen_range(1..=5.min(lines.len().max(1)));
        let end = (start + n).min(lines.len());
        Some(lines[start..end].join("\n"))
    }

    /// Ambil blok struktural utuh acak: module/interface/package/procedural
    /// block paling luar yang bisa ditutup dengan benar (balance begin/end).
    fn random_block(&self, rng: &mut StdRng) -> Option<String> {
        let seed = self.seeds.choose(rng)?;
        let lines: Vec<&str> = seed.lines().collect();
        if lines.is_empty() {
            return None;
        }
        // Cari semua posisi awal blok top-level.
        let openers: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                let t = l.trim_start();
                t.starts_with("module ")
                    || t.starts_with("interface ")
                    || t.starts_with("package ")
                    || t.starts_with("function ")
                    || t.starts_with("task ")
                    || t.starts_with("always ")
                    || t.starts_with("always_comb")
                    || t.starts_with("always_ff")
                    || t.starts_with("always_latch")
                    || t.starts_with("initial ")
                    || t.starts_with("final ")
                    || t.starts_with("fork")
            })
            .map(|(i, _)| i)
            .collect();
        if openers.is_empty() {
            return None;
        }
        let start = *openers.choose(rng)?;
        // Balance begin/end untuk menemukan penutup (dengan kedalaman).
        let mut depth = 0i32;
        for (i, l) in lines.iter().enumerate().skip(start) {
            let t = l.trim();
            if t.starts_with("//") {
                continue;
            }
            let begins = t.matches("begin").count() as i32;
            let ends = t.matches("end").count() as i32;
            depth += begins - ends;
            if depth <= 0 {
                return Some(lines[start..=i].join("\n"));
            }
        }
        Some(lines[start..].join("\n"))
    }

    /// Sampel beberapa seed utuh (untuk seed awal corpus).
    pub fn sample_batch(&self, rng: &mut StdRng, n: usize) -> Vec<String> {
        if self.seeds.is_empty() {
            return Vec::new();
        }
        let mut v: Vec<&String> = self.seeds.iter().collect();
        v.shuffle(rng);
        v.truncate(n);
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Minimizer berbasis baris: hapus baris selama predikat tetap benar.
    /// Predikat = masih bug (panic / mismatch) — dicek pemanggil.
    /// Paper #12/#14: input minimal = bug report yang bisa ditelusuri.
    pub fn minimize(source: &str, keep: &mut impl FnMut(&str) -> bool) -> String {
        let mut lines: Vec<&str> = source.lines().collect();
        if lines.is_empty() {
            return source.to_string();
        }
        let mut i = 0usize;
        let mut tries = 0usize;
        const MAX_TRIES: usize = 400;
        while i < lines.len() && tries < MAX_TRIES {
            tries += 1;
            let mut cand = lines.clone();
            cand.remove(i);
            let joined = cand.join("\n");
            if keep(&joined) {
                lines = cand;
            } else {
                i += 1;
            }
        }
        lines.join("\n")
    }
}

fn collect_sv(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sv(&path, out);
            continue;
        }
        let is_sv = path
            .extension()
            .is_some_and(|e| e == "sv" || e == "v" || e == "svh");
        if !is_sv {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        // DEEP GAP-11: batas naik dari 64KB → MAX_SEED_BYTES (1MB). File
        // RTL nyata (generate/interface/param dalam) sering >64KB; membuang
        // mereka = fuzzer buta terhadap kedalaman struktural.
        if meta.len() > MAX_SEED_BYTES {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if content.contains("module ") || content.contains("interface ") {
                out.push(content);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn empty_corpus_safe() {
        let c = Corpus::empty();
        let mut rng = StdRng::seed_from_u64(1);
        assert!(c.random_fragment(&mut rng).is_none());
        assert!(c.sample_batch(&mut rng, 5).is_empty());
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn minimize_removes_lines_keeping_predicate() {
        let src = "module top;\n  assign a = b;\n  assign c = d;\nendmodule\n";
        // Predikat: selama masih mengandung "assign" (bug-ish guard),
        // minimizer harus mengurangi baris sebanyak mungkin.
        let mut keep = |s: &str| s.contains("assign");
        let min = Corpus::minimize(src, &mut keep);
        assert!(min.contains("assign"));
        assert!(min.len() <= src.len());
        assert!(min.lines().count() <= src.lines().count());
    }

    #[test]
    fn minimize_empty_keeps_empty() {
        let mut keep = |s: &str| s.is_empty() || s.contains('x');
        let min = Corpus::minimize("", &mut keep);
        assert!(min.is_empty());
    }
}