//! Pipeline-Aware Coverage Guide — coverage di tingkat pipeline stage.
//!
//! Berbeda dari CoverageGuide di guide.rs yang hanya melacak fitur
//! expression-level, ini menambahkan tracking pipeline stage:
//! combinational, sequential, concurrent, class, generate, dll.
//!
//! Tujuan: setelah bug fixed, fuzzer tahu pipeline stage mana yang masih
//! kosong dan otomatis mengarah eksplorasi ke sana.

use std::collections::{HashMap, HashSet};

use super::svast::{self, SVAst, GenMode};

/// Pipeline stage yang di-track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineStage {
    Combinational,
    Sequential,
    Posedge,
    NonBlocking,
    IfInAlways,
    CaseInAlways,
    LoopInAlways,
    Concurrent,
    ForkJoin,
    ForkJoinAny,
    ForkJoinNone,
    Class,
    Constraint,
    GenerateFor,
    GenerateIf,
}

impl PipelineStage {
    pub fn all() -> &'static [PipelineStage] {
        &[
            Self::Combinational, Self::Sequential, Self::Posedge,
            Self::NonBlocking, Self::IfInAlways, Self::CaseInAlways,
            Self::LoopInAlways, Self::Concurrent, Self::ForkJoin,
            Self::ForkJoinAny, Self::ForkJoinNone, Self::Class,
            Self::Constraint, Self::GenerateFor, Self::GenerateIf,
        ]
    }

    pub fn tag(&self) -> &'static str {
        match self {
            Self::Combinational => "pipeline:combinational",
            Self::Sequential => "pipeline:sequential",
            Self::Posedge => "pipeline:posedge",
            Self::NonBlocking => "pipeline:nonblocking",
            Self::IfInAlways => "pipeline:if_in_always",
            Self::CaseInAlways => "pipeline:case_in_always",
            Self::LoopInAlways => "pipeline:loop_in_always",
            Self::Concurrent => "pipeline:concurrent",
            Self::ForkJoin => "pipeline:fork_join",
            Self::ForkJoinAny => "pipeline:fork_join_any",
            Self::ForkJoinNone => "pipeline:fork_join_none",
            Self::Class => "pipeline:class",
            Self::Constraint => "pipeline:constraint",
            Self::GenerateFor => "pipeline:generate_for",
            Self::GenerateIf => "pipeline:generate_if",
        }
    }
}

/// Pipeline-aware coverage guide — memperluas CoverageGuide dengan
/// tracking pipeline stage.
pub struct PipelineGuide {
    /// Stage yang sudah terlihat.
    covered_stages: HashSet<String>,
    /// Stage → jumlah kali dilatih.
    stage_frequency: HashMap<String, u64>,
    /// All features (expression + pipeline).
    all_features: HashSet<String>,
    /// Total iterasi.
    pub total: u64,
    /// Iterasi yang menemukan fitur baru.
    pub discovered: u64,
    /// Total fitur baru ditemukan.
    pub new_features: u64,
    /// Stage yang belum tercakup.
    uncovered_stages: Vec<PipelineStage>,
}

impl PipelineGuide {
    pub fn new() -> Self {
        PipelineGuide {
            covered_stages: HashSet::new(),
            stage_frequency: HashMap::new(),
            all_features: HashSet::new(),
            total: 0,
            discovered: 0,
            new_features: 0,
            uncovered_stages: PipelineStage::all().to_vec(),
        }
    }

    /// Observasi satu AST. Kembalikan true bila ada fitur baru.
    pub fn observe_ast(&mut self, ast: &SVAst) -> bool {
        self.total += 1;
        let features = svast::pipeline_features(ast);
        let mut fresh = false;

        for f in &features {
            *self.stage_frequency.entry(f.clone()).or_insert(0) += 1;
            if self.all_features.insert(f.clone()) {
                fresh = true;
                self.new_features += 1;
                if self.covered_stages.insert(f.clone()) {
                    // Update uncovered list
                    self.uncovered_stages.retain(|s| s.tag() != f.as_str());
                }
            }
        }

        if fresh { self.discovered += 1; }
        fresh
    }

    /// Apakah stage tertentu sudah tercakup?
    pub fn is_stage_covered(&self, stage: PipelineStage) -> bool {
        self.covered_stages.contains(stage.tag())
    }

    /// Berapa banyak stage yang belum tercakup?
    pub fn uncovered_count(&self) -> usize {
        self.uncovered_stages.len()
    }

    /// Daftar stage yang belum tercakup.
    pub fn uncovered_stages(&self) -> &[PipelineStage] {
        &self.uncovered_stages
    }

    /// Rekomendasi mode berikutnya berdasarkan coverage gap.
    /// Mengembalikan mode yang paling banyak belum tercakup.
    pub fn recommend_mode(&self, rng: &mut fastrand::Rng) -> GenMode {
        if self.uncovered_stages.is_empty() {
            // Semua stage tercakup — pilih acak
            return GenMode::all()[rng.usize(0..GenMode::all().len())];
        }

        // Pilih stage terdekat dengan uncovered, map ke GenMode
        let stage = self.uncovered_stages[rng.usize(0..self.uncovered_stages.len())];
        match stage {
            PipelineStage::Combinational => GenMode::Combinational,
            PipelineStage::Sequential | PipelineStage::Posedge
            | PipelineStage::NonBlocking | PipelineStage::IfInAlways
            | PipelineStage::CaseInAlways | PipelineStage::LoopInAlways =>
                GenMode::Sequential,
            PipelineStage::Concurrent | PipelineStage::ForkJoin
            | PipelineStage::ForkJoinAny | PipelineStage::ForkJoinNone =>
                GenMode::ForkJoin,
            PipelineStage::Class | PipelineStage::Constraint =>
                GenMode::Class,
            PipelineStage::GenerateFor | PipelineStage::GenerateIf =>
                GenMode::Generate,
        }
    }

    /// Coverage score: rasio stage tercakup / total stage.
    pub fn coverage_score(&self) -> f64 {
        let total = PipelineStage::all().len() as f64;
        if total == 0.0 { return 1.0; }
        // Hanya hitung stage yang valid (tag == PipelineStage::tag), bukan
        // semua fitur expression (bin:*, un:*, W:*).
        let stage_tags: std::collections::HashSet<&'static str> =
            PipelineStage::all().iter().map(|s| s.tag()).collect();
        let covered_stages = self.covered_stages
            .iter()
            .filter(|f| stage_tags.contains(f.as_str()))
            .count();
        covered_stages as f64 / total
    }

    /// Snapshot semua features.
    pub fn features_snapshot(&self) -> HashSet<String> {
        self.all_features.clone()
    }

    /// Stage frequency distribution (untuk debugging).
    pub fn stage_frequency_distribution(&self) -> Vec<(String, u64)> {
        let mut dist: Vec<_> = self.stage_frequency.iter()
            .map(|(k, &v)| (k.clone(), v))
            .collect();
        dist.sort_by(|a, b| b.1.cmp(&a.1));
        dist
    }
}

impl Default for PipelineGuide {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_guide_covers_stages() {
        let mut guide = PipelineGuide::new();
        let seed = 42u64;

        // Generate one of each mode
        for &mode in GenMode::all() {
            let ast = svast::generate_svast_mode(seed, mode);
            guide.observe_ast(&ast);
        }

        // At least combinational should be covered
        assert!(guide.is_stage_covered(PipelineStage::Combinational));
        assert!(guide.coverage_score() > 0.0);
    }

    #[test]
    fn pipeline_guide_recommends_uncovered() {
        let mut guide = PipelineGuide::new();
        let mut rng = fastrand::Rng::with_seed(42);

        // Don't cover anything
        let mode = guide.recommend_mode(&mut rng);
        // Should recommend something (any mode is fine when nothing covered)
        assert!(GenMode::all().contains(&mode));
    }
}
