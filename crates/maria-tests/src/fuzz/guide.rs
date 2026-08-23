//! Feedback/coverage guide — inti "tidak buta" pada fuzzer.
//!
//! Fuzzer buta = mutasi acak tanpa umpan balik. Di sini kita pelihara
//! *feature map* (fitur bahasa yang sudah tereksekusi: op, lebar, outcome)
//! dan *corpus* (seed yang menemukan fitur baru). Iterasi berikutnya
//! memprioritaskan mutasi dari corpus → arahkan eksplorasi ke jalur yang
//! belum tersentuh. Ini coverage-guided structure-aware fuzzing.

use std::collections::HashSet;

use super::gen::GenInput;

#[derive(Debug, Clone)]
struct Corpuseed {
    input: GenInput,
    ok: bool,
}

/// Pengendali umpan balik: lacak coverage fitur + corpus.
pub struct CoverageGuide {
    features: HashSet<String>,
    corpus: Vec<Corpuseed>,
    pub total: u64,
    pub discovered: u64,
    pub new_features: u64,
}

impl CoverageGuide {
    pub fn new() -> Self {
        CoverageGuide {
            features: HashSet::new(),
            corpus: Vec::new(),
            total: 0,
            discovered: 0,
            new_features: 0,
        }
    }

    /// Hitung fitur dari satu input (struktur ekspresi + lebar + outcome).
    fn feature_tags(input: &GenInput, ok: bool) -> Vec<String> {
        let mut tags = Vec::new();
        input.expr.features(&mut tags);
        tags.push(format!("W:{}", input.w));
        tags.push(if ok { "out:ok".to_string() } else { "out:fail".to_string() });
        tags
    }

    /// Observasi hasil satu iterasi. Kembalikan true bila menemukan fitur baru.
    pub fn observe(&mut self, input: &GenInput, ok: bool) -> bool {
        self.total += 1;
        let tags = Self::feature_tags(input, ok);
        let mut fresh = false;
        for t in &tags {
            if self.features.insert(t.clone()) {
                fresh = true;
                self.new_features += 1;
            }
        }
        if fresh {
            self.discovered += 1;
            self.corpus.push(Corpuseed {
                input: input.clone(),
                ok,
            });
        }
        fresh
    }

    pub fn coverage_len(&self) -> usize {
        self.features.len()
    }

    pub fn corpus_len(&self) -> usize {
        self.corpus.len()
    }

    pub fn corpus_get(&self, idx: usize) -> Option<GenInput> {
        self.corpus.get(idx).map(|c| c.input.clone())
    }

    /// Pilih input berikutnya: dari corpus (mutasi) bila ada & beruntung,
    /// else generate segar. Seed diturunkan dari counter agar deterministik.
    pub fn next(&self, counter: u64) -> GenInput {
        if !self.corpus.is_empty() && (counter % 2 == 0) {
            // Pilih record corpus acak, lalu mutasi structure-aware.
            let idx = (counter as usize) % self.corpus.len();
            let base = &self.corpus[idx].input;
            super::gen::mutate_from(base, counter.wrapping_mul(2654435761))
        } else {
            super::gen::generate(counter.wrapping_mul(40503).wrapping_add(1))
        }
    }
}

impl Default for CoverageGuide {
    fn default() -> Self {
        Self::new()
    }
}
