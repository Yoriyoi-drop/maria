//! Corpus seed SV nyata + minimizer.
//!
//! Paper #12 (Holler et al., "Fuzzing with Code Fragments"): mutasi efektif
//! datang dari serpihan kode *nyata*, bukan literal acak. Corpus diambil
//! dari direktori proyek (test/, opentitan/, cva6/, …) lewat `--corpus-dir`.
//! `minimize` = reduksi input bug ke bentuk minimal (OSS-Fuzz-style).

use std::path::{Path, PathBuf};

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::Rng;

#[derive(Debug, Default)]
pub struct Corpus {
    pub seeds: Vec<String>,
}

impl Corpus {
    pub fn empty() -> Self {
        Corpus::default()
    }

    /// Muat semua file `.sv` dari direktori (rekursif). File besar (>64 KB)
    /// dilewati — fuzzer menyukai seed kecil (Paper #12: unit mutasi).
    pub fn from_dirs(dirs: &[PathBuf]) -> Self {
        let mut seeds: Vec<String> = Vec::new();
        for dir in dirs {
            collect_sv(dir, &mut seeds);
        }
        Corpus { seeds }
    }

    pub fn len(&self) -> usize {
        self.seeds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seeds.is_empty()
    }

    /// Serpihan acak (fragment) dari corpus — bahan splice mutasi (#12).
    pub fn random_fragment(&self, rng: &mut StdRng) -> Option<String> {
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
        if meta.len() > 64 * 1024 {
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