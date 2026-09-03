//! Feedback/coverage guide — inti "tidak buta" pada fuzzer.
//!
//! Fuzzer buta = mutasi acak tanpa umpan balik. Di sini kita pelihara
//! *feature map* (fitur bahasa yang sudah tereksekusi: op, lebar, outcome)
//! dan *corpus* (seed yang menemukan fitur baru). Iterasi berikutnya
//! memprioritaskan mutasi dari corpus → arahkan eksplorasi ke jalur yang
//! belum tersentuh. Ini coverage-guided structure-aware fuzzing.
//!
//! **Energy-based seed scheduling** (AFLFast/FairFuzz inspired):
//! - Path frequency tracking: seed yang exercise path jarang mendapat
//!   energy lebih tinggi → dipilih lebih sering untuk mutasi.
//! - Exponential power schedule: energy ∝ 1/freq(path)^α
//! - Bug-guided energy boost: seed yang dekat bug database mendapat
//!   energy bonus (ChipFuzzer concept).

use std::collections::HashSet;

use super::bug_db::BugDatabase;
use super::gen::GenInput;

#[derive(Debug, Clone)]
struct Corpuseed {
    input: GenInput,
    ok: bool,
    /// Feature tags that were newly discovered by this seed.
    new_features: Vec<String>,
    /// Energy score (higher = more likely to be selected for mutation).
    energy: f64,
    /// Path frequency: how many seeds exercise the same path.
    path_freq: u64,
}

/// Pengendali umpan balik: lacak coverage fitur + corpus + energy scheduling.
///
/// Energy scheduling follows AFLFast's exponential power schedule:
/// ```text
/// energy(seed) = 1 / (path_frequency(seed))^α
/// ```
/// where α controls how aggressively we favor rare paths (default α=1.0).
pub struct CoverageGuide {
    features: HashSet<String>,
    /// Feature → count of seeds that exercised this feature.
    feature_frequency: HashMap<String, u64>,
    corpus: Vec<Corpuseed>,
    pub total: u64,
    pub discovered: u64,
    pub new_features: u64,
    /// Path frequency map: feature → how many seeds hit it.
    /// Used to compute energy for AFLFast-style scheduling.
    path_frequencies: HashMap<String, u64>,
    /// α parameter for exponential power schedule (higher = favor rare paths more).
    alpha: f64,
    /// Reference to bug database for bug-guided energy boost.
    bug_db: BugDatabase,
}

use std::collections::HashMap;

impl CoverageGuide {
    pub fn new() -> Self {
        CoverageGuide {
            features: HashSet::new(),
            feature_frequency: HashMap::new(),
            corpus: Vec::new(),
            total: 0,
            discovered: 0,
            new_features: 0,
            path_frequencies: HashMap::new(),
            alpha: 1.0,
            bug_db: BugDatabase::new(),
        }
    }

    /// Hitung fitur dari satu input (struktur ekspresi + lebar + outcome).
    fn feature_tags(input: &GenInput, ok: bool) -> Vec<String> {
        let mut tags = Vec::new();
        input.expr.features(&mut tags);
        tags.push(format!("W:{}", input.w));
        tags.push(if ok {
            "out:ok".to_string()
        } else {
            "out:fail".to_string()
        });
        tags
    }

    /// Compute energy for a seed based on path frequency (AFLFast concept).
    /// Lower frequency = higher energy → more mutations from this seed.
    fn compute_energy(&self, features: &[String]) -> f64 {
        if features.is_empty() {
            return 1.0;
        }
        // Average path frequency across all features
        let avg_freq: f64 = features
            .iter()
            .map(|f| {
                self.path_frequencies
                    .get(f)
                    .copied()
                    .unwrap_or(1) as f64
            })
            .sum::<f64>()
            / features.len() as f64;

        // Exponential power schedule: energy ∝ 1/freq^α
        // Add 1.0 to avoid division by zero and ensure minimum energy
        let base_energy = 1.0 / (avg_freq.powf(self.alpha) + 1.0);

        // Bug-guided boost: if any feature is hot in bug DB, boost energy
        let bug_boost = self.bug_db.bug_priority(features) * 2.0;

        (base_energy + bug_boost).max(0.01)
    }

    /// Observasi hasil satu iterasi. Kembalikan true bila menemukan fitur baru.
    pub fn observe(&mut self, input: &GenInput, ok: bool) -> bool {
        self.total += 1;
        let tags = Self::feature_tags(input, ok);
        let mut fresh = false;
        let mut new_tags = Vec::new();

        // Update path frequencies for all features
        for t in &tags {
            *self.path_frequencies.entry(t.clone()).or_insert(0) += 1;
            *self.feature_frequency.entry(t.clone()).or_insert(0) += 1;
        }

        for t in &tags {
            if self.features.insert(t.clone()) {
                fresh = true;
                self.new_features += 1;
                new_tags.push(t.clone());
            }
        }

        if fresh {
            self.discovered += 1;
            let energy = self.compute_energy(&tags);
            self.corpus.push(Corpuseed {
                input: input.clone(),
                ok,
                new_features: new_tags,
                energy,
                path_freq: tags
                    .iter()
                    .map(|f| self.path_frequencies.get(f).copied().unwrap_or(1))
                    .max()
                    .unwrap_or(1),
            });
        }
        fresh
    }

    /// Record a bug in the internal bug database (for bug-guided energy boost).
    pub fn record_bug(
        &mut self,
        input: &GenInput,
        message: &str,
        severity: super::bug_db::BugSeverity,
    ) {
        self.bug_db.record_bug(input, message, severity);
    }

    /// Get reference to bug database.
    pub fn bug_database(&self) -> &BugDatabase {
        &self.bug_db
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

    pub fn corpus_seeds(&self) -> Vec<u64> {
        self.corpus.iter().map(|c| c.input.seed).collect()
    }

    /// Snapshot seluruh tag fitur — dipakai agregasi paralel untuk UNION
    /// antar worker (bukan max yang menyesatkan).
    pub fn features_snapshot(&self) -> HashSet<String> {
        self.features.clone()
    }

    /// Pilih input berikutnya berdasarkan energy (AFLFast/FairFuzz concept).
    ///
    /// Seeds with higher energy (rare paths, bug-proximate) are selected
    /// more frequently. This avoids wasting mutations on common paths.
    pub fn next(&self, counter: u64) -> GenInput {
        if self.corpus.is_empty() {
            return super::gen::generate(counter.wrapping_mul(40503).wrapping_add(1));
        }

        // Weighted selection based on energy (AFLFast exponential schedule)
        let total_energy: f64 = self.corpus.iter().map(|c| c.energy).sum();
        if total_energy <= 0.0 {
            // Fallback: uniform selection
            let idx = (counter as usize) % self.corpus.len();
            let base = &self.corpus[idx].input;
            return super::gen::mutate_from(base, counter.wrapping_mul(2654435761));
        }

        // Deterministic weighted selection using counter
        let target = (counter as f64 / (counter + 1000) as f64) * total_energy;
        let mut cumulative = 0.0;
        let mut selected_idx = 0;
        for (i, c) in self.corpus.iter().enumerate() {
            cumulative += c.energy;
            if cumulative >= target {
                selected_idx = i;
                break;
            }
        }

        let base = &self.corpus[selected_idx].input;
        super::gen::mutate_from(base, counter.wrapping_mul(2654435761))
    }

    /// Get path frequency distribution (for debugging/reporting).
    pub fn path_frequency_distribution(&self) -> Vec<(String, u64)> {
        let mut dist: Vec<_> = self
            .path_frequencies
            .iter()
            .map(|(k, &v)| (k.clone(), v))
            .collect();
        dist.sort_by(|a, b| b.1.cmp(&a.1));
        dist
    }
}

impl Default for CoverageGuide {
    fn default() -> Self {
        Self::new()
    }
}
