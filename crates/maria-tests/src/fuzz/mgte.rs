//! Maria Guided Test Engine (MGTE) — orkestrator utama testing terarah.

use super::gen::GenInput;
use super::guide::CoverageGuide;
use super::oracle::{check, Verdict};
use super::semantic_mutator::SemanticMutator;
use super::semantic_mutator::ModuleContext;
use super::hierarchy_mutator::{HierarchyMutator, HierarchyNode};
use super::minimizer::TestcaseMinimizer;
use super::differential::{DifferentialExecutor, Simulator, DifferentialResult};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MGTEMode {
    Parser,
    Semantic,
    Type,
    Elaboration,
    Hierarchy,
    Generate,
    Simulator,
    Differential,
    Resource,
}

#[derive(Debug, Clone)]
pub struct MGTEConfig {
    pub modes: Vec<MGTEMode>,
    pub iterations: u64,
    pub seed: u64,
    pub enable_differential: bool,
    pub enable_minimizer: bool,
    pub differential_sims: Vec<Simulator>,
    pub timeout_secs: u64,
}

impl Default for MGTEConfig {
    fn default() -> Self {
        MGTEConfig {
            modes: vec![
                MGTEMode::Semantic,
                MGTEMode::Elaboration,
                MGTEMode::Hierarchy,
                MGTEMode::Simulator,
                MGTEMode::Differential,
            ],
            iterations: 300,
            seed: 0,
            enable_differential: true,
            enable_minimizer: true,
            differential_sims: vec![Simulator::Verilator, Simulator::Icarus],
            timeout_secs: 60,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MGTEStats {
    pub total_iterations: u64,
    pub passed: u64,
    pub compile_failures: u64,
    pub bugs_found: u64,
    pub differential_mismatches: u64,
    pub minimized_cases: u64,
    pub coverage_features: usize,
    pub corpus_size: usize,
    pub elapsed_ms: u64,
}

pub struct MGTE {
    config: MGTEConfig,
    guide: CoverageGuide,
    semantic_mutator: SemanticMutator,
    hierarchy_mutator: HierarchyMutator,
    minimizer: TestcaseMinimizer,
    differential: Option<DifferentialExecutor>,
    stats: MGTEStats,
    bug_corpus: Vec<(GenInput, String)>,
}

impl MGTE {
    pub fn new(config: MGTEConfig) -> Self {
        let differential = if config.enable_differential {
            Some(DifferentialExecutor::new().with_simulators(config.differential_sims.clone()))
        } else {
            None
        };

        MGTE {
            guide: CoverageGuide::new(),
            semantic_mutator: SemanticMutator::new(config.seed),
            hierarchy_mutator: HierarchyMutator::new(config.seed),
            minimizer: TestcaseMinimizer::new(),
            differential,
            config,
            stats: MGTEStats {
                total_iterations: 0,
                passed: 0,
                compile_failures: 0,
                bugs_found: 0,
                differential_mismatches: 0,
                minimized_cases: 0,
                coverage_features: 0,
                corpus_size: 0,
                elapsed_ms: 0,
            },
            bug_corpus: Vec::new(),
        }
    }

    pub fn run(&mut self) -> MGTEStats {
        let start = Instant::now();
        let n = self.config.iterations;

        for i in 0..n {
            let input = self.select_next_input(i);

            let result = check(&input);
            let compiled = result.compiled;

            match &result.verdict {
                Verdict::Bug(msg) => {
                    self.stats.bugs_found += 1;
                    self.bug_corpus.push((input.clone(), msg.clone()));
                    eprintln!("[MGTE] BUG #{}: {}\n=== SOURCE ===\n{}\n=== END ===", self.stats.bugs_found, msg, input.to_source());

                    if self.config.enable_differential {
                        if let Some(diff) = &self.differential {
                            let diff_result = diff.run(&input.to_source());
                            if diff_result.has_bug() {
                                self.stats.differential_mismatches += 1;
                                eprintln!("[MGTE] DIFFERENTIAL MISMATCH: {}", diff_result.summary());

                                if self.config.enable_minimizer {
                                    self.minimize_bug(&input, &diff_result);
                                }
                            }
                        }
                    }
                }
                Verdict::Pass => {
                    self.stats.passed += 1;
                }
                Verdict::CompileFail => {
                    self.stats.compile_failures += 1;
                }
            }

            self.guide.observe(&input, compiled);

            if i % 50 == 0 && i > 0 {
                eprintln!(
                    "[MGTE] iter={}/{} pass={} fail={} bugs={} diff={} cov={} corpus={}",
                    i, n,
                    self.stats.passed,
                    self.stats.compile_failures,
                    self.stats.bugs_found,
                    self.stats.differential_mismatches,
                    self.guide.coverage_len(),
                    self.guide.corpus_len()
                );
            }
        }

        self.stats.total_iterations = n;
        self.stats.coverage_features = self.guide.coverage_len();
        self.stats.corpus_size = self.guide.corpus_len();
        self.stats.elapsed_ms = start.elapsed().as_millis() as u64;

        self.stats.clone()
    }

    fn select_next_input(&mut self, counter: u64) -> GenInput {
        if self.config.modes.contains(&MGTEMode::Hierarchy) && self.hierarchy_mutator.has_hierarchy() {
            if let Some(target) = self.select_hierarchy_target() {
                if let Some(input) = self.hierarchy_mutator.generate_targeted_testcase(&target) {
                    return input;
                }
            }
        }

        if self.config.modes.contains(&MGTEMode::Semantic) && self.guide.corpus_len() > 0 && counter % 3 == 0 {
            let idx = (counter as usize) % self.guide.corpus_len();
            if let Some(base) = self.guide.corpus_get(idx) {
                return self.semantic_mutator.mutate_input(&base);
            }
        }

        if self.config.modes.contains(&MGTEMode::Generate) && self.guide.corpus_len() > 0 && counter % 5 == 0 {
            let idx = (counter as usize) % self.guide.corpus_len();
            if let Some(base) = self.guide.corpus_get(idx) {
                return self.mutate_generate_bounds(&base);
            }
        }

        self.guide.next(counter)
    }

    fn select_hierarchy_target(&self) -> Option<String> {
        if let Some(paths) = self.hierarchy_mutator.root_all_paths() {
            if !paths.is_empty() {
                let mut rng = fastrand::Rng::with_seed(self.config.seed.wrapping_add(12345));
                Some(paths[rng.usize(0..paths.len())].clone())
            } else {
                None
            }
        } else {
            None
        }
    }

    fn mutate_generate_bounds(&self, input: &GenInput) -> GenInput {
        let boundaries = [1u32, 2, 4, 8, 16, 31, 32, 33, 64, 128, 255, 256, 512, 1024];
        let mut rng = fastrand::Rng::with_seed(input.seed.wrapping_mul(98765));
        let boundary = boundaries[rng.usize(0..boundaries.len())];

        let mut new_input = input.clone();
        new_input.w = boundary;
        new_input.seed = rng.u64(..);
        let mask = if boundary >= 64 { u64::MAX } else { (1u64 << boundary) - 1 };
        new_input.a = rng.u64(..) & mask;
        new_input.b = rng.u64(..) & mask;
        new_input.expr = super::expr::gen_node(boundary, &mut rng, 0);
        new_input
    }

    fn minimize_bug(&mut self, input: &GenInput, diff_result: &DifferentialResult) {
        let check_fn = |candidate: &GenInput| -> bool {
            let result = check(candidate);
            matches!(result.verdict, Verdict::Bug(_))
        };

        let minimize_result = self.minimizer.minimize(input, check_fn);

        if minimize_result.still_fails {
            self.stats.minimized_cases += 1;
            eprintln!("[MGTE] MINIMIZED: {} -> {} ({} steps)",
                self.source_size(&minimize_result.original),
                self.source_size(&minimize_result.minimized),
                minimize_result.steps.len()
            );
        }
    }

    fn source_size(&self, input: &GenInput) -> usize {
        input.to_source().len()
    }

    pub fn set_hierarchy(&mut self, hierarchy: HierarchyNode) {
        self.hierarchy_mutator = self.hierarchy_mutator.clone().with_hierarchy(hierarchy);
    }

    pub fn set_context(&mut self, ctx: ModuleContext) {
        self.semantic_mutator = self.semantic_mutator.clone().with_context(ctx);
    }

    pub fn get_bugs(&self) -> &[(GenInput, String)] {
        &self.bug_corpus
    }

    pub fn get_stats(&self) -> &MGTEStats {
        &self.stats
    }

    pub fn export_report(&self) -> String {
        format!(
            r#"# MGTE Test Report

## Configuration
- Modes: {:?}
- Iterations: {}
- Differential: {}
- Minimizer: {}

## Statistics
- Total Iterations: {}
- Passed: {}
- Compile Failures: {}
- Bugs Found: {}
- Differential Mismatches: {}
- Minimized Cases: {}
- Coverage Features: {}
- Corpus Size: {}
- Elapsed Time: {} ms

## Bugs Found
{}
"#,
            self.config.modes,
            self.config.iterations,
            self.config.enable_differential,
            self.config.enable_minimizer,
            self.stats.total_iterations,
            self.stats.passed,
            self.stats.compile_failures,
            self.stats.bugs_found,
            self.stats.differential_mismatches,
            self.stats.minimized_cases,
            self.stats.coverage_features,
            self.stats.corpus_size,
            self.stats.elapsed_ms,
            self.bug_corpus.iter().enumerate().map(|(i, (input, msg))| {
                format!(
                    "{}. seed={} w={} expr=`{}`\n   ```\n{}\n   ```\n   **Bug:** {}\n",
                    i + 1,
                    input.seed,
                    input.w,
                    input.expr.to_sv(input.w),
                    input.to_source(),
                    msg
                )
            }).collect::<String>()
        )
    }
}

impl MGTEConfig {
    pub fn for_elaboration() -> Self {
        MGTEConfig {
            modes: vec![MGTEMode::Elaboration, MGTEMode::Hierarchy, MGTEMode::Generate],
            iterations: 500,
            ..Default::default()
        }
    }

    pub fn for_simulator() -> Self {
        MGTEConfig {
            modes: vec![MGTEMode::Simulator, MGTEMode::Semantic, MGTEMode::Type],
            iterations: 300,
            enable_differential: true,
            ..Default::default()
        }
    }

    pub fn for_differential() -> Self {
        MGTEConfig {
            modes: vec![MGTEMode::Differential, MGTEMode::Elaboration, MGTEMode::Simulator],
            iterations: 200,
            enable_differential: true,
            differential_sims: vec![Simulator::Verilator, Simulator::Icarus],
            ..Default::default()
        }
    }

    pub fn for_full() -> Self {
        MGTEConfig {
            modes: vec![
                MGTEMode::Parser,
                MGTEMode::Semantic,
                MGTEMode::Type,
                MGTEMode::Elaboration,
                MGTEMode::Hierarchy,
                MGTEMode::Generate,
                MGTEMode::Simulator,
                MGTEMode::Differential,
                MGTEMode::Resource,
            ],
            iterations: 1000,
            enable_differential: true,
            enable_minimizer: true,
            ..Default::default()
        }
    }
}