//! Coverage guide — inti "tidak buta" fuzzer: pemilihan seed
//! berdasar energi, bukan acak buta.
//!
//! Paper #4 (AFLFast): power schedule `energy ∝ 1/freq(path)^α`.
//! Paper #5 (FairFuzz): fitur langka dapat energi bonus (rare-branch).
//! Paper #7 (VUzzer): bobot dari fitur dataflow (feature set tiap seed).
//! Paper #8 (EcoFuzz): energi adaptif — saat coverage mandek, α diturunkan
//!   agar eksplorasi melebar.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

use crate::feature::FeatureMap;

const MAX_SEEDS: usize = 2048;
const ALPHA_DECAY: f64 = 0.15;
const ALPHA_MIN: f64 = 0.3;
const ADAPT_WINDOW: u64 = 200;

#[derive(Debug, Clone)]
pub struct SeedEntry {
    pub source: String,
    pub feats: Vec<String>,
    pub visits: u64,
    pub is_bug: bool,
    pub energy: f64,
}

pub struct CoverageGuide {
    map: FeatureMap,
    seeds: Vec<SeedEntry>,
    rng: StdRng,
    alpha: f64,
    stall: u64,
}

impl CoverageGuide {
    pub fn new(seed: u64) -> Self {
        CoverageGuide {
            map: FeatureMap::new(),
            seeds: Vec::new(),
            rng: StdRng::seed_from_u64(seed),
            alpha: 1.0,
            stall: 0,
        }
    }

    /// Tambah seed ke corpus fuzzer (feature sudah diekstrak pemanggil).
    pub fn add(&mut self, source: String, feats: Vec<String>, is_bug: bool) {
        self.map.record(&feats);
        let energy = self.compute_energy(&feats, 0, is_bug);
        // Dedup by source (hash = source string).
        if let Some(e) = self.seeds.iter_mut().find(|e| e.source == source) {
            e.feats = feats;
            e.is_bug |= is_bug;
            e.energy = energy;
            return;
        }
        self.seeds.push(SeedEntry {
            source,
            feats,
            visits: 0,
            is_bug,
            energy,
        });
        // Jaga memori: buang seed berenergi rendah (bukan bug) bila penuh.
        if self.seeds.len() > MAX_SEEDS {
            if let Some(pos) = self
                .seeds
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.is_bug)
                .min_by(|a, b| a.1.energy.partial_cmp(&b.1.energy).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i)
            {
                self.seeds.swap_remove(pos);
            }
        }
    }

    /// Catat hasil iterasi: fitur baru? → adaptasi energi (Paper #8).
    pub fn record(&mut self, source: &str, feats: &[String]) -> bool {
        let is_new = self.map.has_new(feats);
        if is_new {
            self.map.record(feats);
            // Seed parent yang memicu fitur baru dapat dorongan energi.
            if let Some(e) = self.seeds.iter_mut().find(|e| e.source == source) {
                e.energy *= 2.0;
            }
        }
        is_new
    }

    /// Adaptasi alpha (EcoFuzz #8): mandek → eksplorasi lebih luas.
    pub fn record_adapt(&mut self, new_found: bool) {
        if new_found {
            self.stall = 0;
            self.alpha = 1.0;
        } else {
            self.stall += 1;
            if self.stall % ADAPT_WINDOW == 0 {
                self.alpha = (self.alpha - ALPHA_DECAY).max(ALPHA_MIN);
            }
        }
    }

    /// Increment kunjungan parent yang baru saja dieksekusi (path frequency).
    pub fn note_visit(&mut self, source: &str) {
        let pos = self.seeds.iter().position(|e| e.source == source);
        if let Some(pos) = pos {
            self.seeds[pos].visits += 1;
            let (feats, visits, is_bug) = {
                let e = &self.seeds[pos];
                (e.feats.clone(), e.visits, e.is_bug)
            };
            let energy = self.compute_energy(&feats, visits, is_bug);
            self.seeds[pos].energy = energy;
        }
    }

    /// Pilih seed parent — weighted random oleh energi (AFLFast power schedule).
    pub fn select(&mut self) -> Option<String> {
        if self.seeds.is_empty() {
            return None;
        }
        // Energi bisa nol bila semua seed sudah sangat sering dikunjungi —
        // fallback ke pilihan acak seragam.
        let total: f64 = self.seeds.iter().map(|e| e.energy).sum();
        if total <= f64::EPSILON {
            return self.seeds.choose(&mut self.rng).map(|e| e.source.clone());
        }
        let mut pick = self.rng.gen_range(0.0..total);
        for e in &self.seeds {
            if pick < e.energy {
                return Some(e.source.clone());
            }
            pick -= e.energy;
        }
        self.seeds.last().map(|e| e.source.clone())
    }

    pub fn len(&self) -> usize {
        self.seeds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seeds.is_empty()
    }

    /// Ringkasan coverage: fitur tertutup dari fitur yang pernah dilihat.
    pub fn coverage(&self) -> FeatureMap {
        self.map.clone()
    }

    /// Power schedule (Paper #4): 1/freq^α + bonus fitur langka (#5)
    /// + bug boost (#20: bug-guided utility).
    fn compute_energy(&self, feats: &[String], visits: u64, is_bug: bool) -> f64 {
        let base = 1.0 / (1.0 + visits as f64).powf(self.alpha);
        // Fitur langka (jarang dieksekusi seluruh corpus) → energi tinggi.
        let rare: u64 = feats
            .iter()
            .filter(|f| self.map.counts.get(*f).copied().unwrap_or(0) == 1)
            .count() as u64;
        let mut e = base * (1.0 + 0.25 * rare as f64);
        if is_bug {
            e *= 4.0;
        }
        e.max(0.001)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_returns_existing_seed() {
        let mut g = CoverageGuide::new(1);
        assert!(g.select().is_none());
        g.add("a".to_string(), vec!["+".to_string()], false);
        g.add("b".to_string(), vec!["*".to_string()], false);
        let s = g.select().unwrap();
        assert!(s == "a" || s == "b");
        assert_eq!(g.len(), 2);
    }

    #[test]
    fn rare_feature_gets_more_energy() {
        let mut g = CoverageGuide::new(2);
        // Seed umum: fitur "+" sudah sering (dihitung 2x).
        g.add("common_a".to_string(), vec!["+".to_string()], false);
        g.add("common_b".to_string(), vec!["+".to_string()], false);
        // Seed langka: fitur unik yang baru diperkenalkan seed ini.
        g.add("rare".to_string(), vec!["width:999".to_string()], false);
        let rare_e = g.seeds.iter().find(|e| e.source == "rare").unwrap().energy;
        let common_e = g.seeds.iter().find(|e| e.source == "common_b").unwrap().energy;
        assert!(
            rare_e > common_e,
            "seed fitur langka harus energi lebih tinggi ({} > {})",
            rare_e,
            common_e
        );
    }

    #[test]
    fn bug_seed_boosted() {
        let mut g = CoverageGuide::new(3);
        g.add("x".to_string(), vec!["+".to_string()], false);
        g.add("y".to_string(), vec!["+".to_string()], true);
        let bx = g.seeds.iter().find(|e| e.source == "y").unwrap().energy;
        let nx = g.seeds.iter().find(|e| e.source == "x").unwrap().energy;
        assert!(bx > nx, "bug seed wajib energi lebih tinggi");
    }

    #[test]
    fn note_visit_lowers_energy() {
        let mut g = CoverageGuide::new(4);
        g.add("s".to_string(), vec!["+".to_string()], false);
        let e0 = g.seeds[0].energy;
        g.note_visit("s");
        g.note_visit("s");
        let e2 = g.seeds[0].energy;
        assert!(e2 < e0, "kunjungan menurunkan energi (power schedule)");
    }

    #[test]
    fn record_new_feature_returns_true_once() {
        let mut g = CoverageGuide::new(5);
        g.add("s".to_string(), vec!["+".to_string()], false);
        assert!(g.record("s", &["width:77".to_string()])); // fitur baru
        assert!(!g.record("s", &["width:77".to_string()])); // sudah ada
    }
}