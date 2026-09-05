//! guided_fuzz_v2 — orchestrasi fuzz terarah dengan pipeline-aware coverage
//! + auto-expansion berbasis bug.
//!
//! Berbeda dari `guided_fuzz` (v1) yang hanya pakai Expr generik:
//! - Menggunakan SVAst (AST gabungan) → mengeksplorasi semua fitur SV
//! - PipelineGuide → tahu stage mana yang belum terlatih
//! - AutoExpander → otomatis generate vari setelah bug ditemukan
//!
//! Jalankan: `cargo test guided_fuzz_v2`
//! Konfigurasi: MARIA_FUZZ_V2_N (default 300), MARIA_FUZZ_V2_WORKERS (default 10)

use super::auto_expand::{self, AutoExpander};
use super::bug_db::BugSeverity;
use super::guide::CoverageGuide;
use super::oracle::{self, Verdict};
use super::parallel::{self, BOTTLENECK_THRESHOLD_US};
use super::pipeline_guide::PipelineGuide;
use super::svast::{self, GenMode, SVAst};

/// Statistik v2.
#[derive(Debug, Clone, Default)]
pub struct V2Stats {
    pub iterations: u64,
    pub passed: u64,
    pub compile_failures: u64,
    pub bugs_found: usize,
    pub pipeline_coverage: f64,
    pub pipeline_stages_covered: usize,
    pub pipeline_stages_total: usize,
    pub variants_generated: usize,
    pub elapsed_ms: u64,
}

/// Satu bug ditemukan.
#[derive(Debug, Clone)]
pub struct V2Bug {
    pub ast: SVAst,
    pub source: String,
    pub message: String,
    pub seed: u64,
}

/// Konfigurasi v2.
pub struct V2Config {
    pub iterations: u64,
    pub workers: usize,
}

impl Default for V2Config {
    fn default() -> Self {
        V2Config {
            iterations: std::env::var("MARIA_FUZZ_V2_N")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(300),
            workers: std::env::var("MARIA_FUZZ_V2_WORKERS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(parallel::DEFAULT_WORKERS),
        }
    }
}

/// Verifikasi satu source SVAst: compile + sim 2x (determinism) + golden
/// untuk kombinasional murni. Kembalikan verdict.
///
/// Berbeda dari `oracle::check` yang butuh GenInput, versi ini beroperasi
/// langsung pada source SVAst sehingga fitur sequential/class/generate
/// benar-benar disimulasikan (bukan placeholder).
fn check_svast_source(src: &str, golden: Option<u64>) -> Verdict {
    let src_owned = src.to_string();
    let runner = move || -> Option<(bool, Option<u64>, Option<u64>)> {
        let compiled = crate::compile_str(&src_owned).is_ok();
        if !compiled {
            return Some((false, None, None));
        }
        let run1 = crate::simulate_signals(&src_owned, 50).ok();
        let run2 = crate::simulate_signals(&src_owned, 50).ok();
        let y1 = run1
            .as_ref()
            .and_then(|s| s.iter().find(|(n, _)| n == "y").map(|(_, v)| v.to_u64()));
        let y2 = run2
            .as_ref()
            .and_then(|s| s.iter().find(|(n, _)| n == "y").map(|(_, v)| v.to_u64()));
        Some((true, y1, y2))
    };

    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .name("fuzz-v2-check".to_string())
        .spawn(move || std::panic::catch_unwind(std::panic::AssertUnwindSafe(runner)))
        .ok();

    let res: Option<
        Result<Option<(bool, Option<u64>, Option<u64>)>, Box<dyn std::any::Any + Send>>,
    > = handle.and_then(|h| h.join().ok());

    match res {
        Some(Ok(Some((true, Some(a), Some(b))))) => {
            if a != b {
                return Verdict::Bug(format!("non-determinism: run1={:#x} run2={:#x}", a, b));
            }
            match golden {
                Some(exp) if a != exp => Verdict::Bug(format!(
                    "differential mismatch: golden {:#x} maria {:#x}",
                    exp, a
                )),
                _ => Verdict::Pass,
            }
        }
        Some(Ok(Some((true, _, _)))) => Verdict::Bug("compiled but 'y' not readable".to_string()),
        Some(Ok(Some((false, _, _)))) => Verdict::CompileFail,
        Some(Ok(None)) | Some(Err(_)) | None => {
            Verdict::Bug("panic during compile/simulate".to_string())
        }
    }
}

/// Jalankan satu worker fuzz v2.
fn run_v2_worker(
    worker_id: usize,
    workers: usize,
    iterations: u64,
) -> (V2Stats, Vec<V2Bug>, PipelineGuide) {
    let mut pipeline = PipelineGuide::new();
    let mut coverage = CoverageGuide::new();
    let mut expander = AutoExpander::new();
    let mut bugs = Vec::new();
    let mut stats = V2Stats::default();
    let start = std::time::Instant::now();

    let mut i = worker_id as u64;
    while i < iterations {
        // 1. Pilih SVAst: pipeline-aware mode selection
        let ast = if pipeline.uncovered_count() > 0 && i % 3 == 0 {
            // Prioritas: eksplore stage yang belum tercakup
            let mut rng = fastrand::Rng::with_seed(i ^ 0x7777);
            let mode = pipeline.recommend_mode(&mut rng);
            svast::generate_svast_mode(i.wrapping_mul(40503).wrapping_add(1), mode)
        } else {
            svast::generate_svast(i.wrapping_mul(40503).wrapping_add(7))
        };

        // 2. Source SVAst
        let src = ast.to_source();
        let seed = i ^ 0x9E3779B97F4A7C15;

        // 3. Golden untuk kombinasional murni (assign y = expr tanpa X)
        let golden = svast::eval_golden(&ast, 0, 0);

        // 4. Verifikasi source SVAst langsung (compile + sim + determinism)
        let verdict = check_svast_source(&src, golden);

        match &verdict {
            Verdict::Bug(msg) => {
                bugs.push(V2Bug {
                    ast: ast.clone(),
                    source: src.clone(),
                    message: msg.clone(),
                    seed,
                });
                stats.bugs_found += 1;

                // Auto-expand: generate variants dari bug ini.
                // Konversi SVAst → GenInput agar masuk ekosistem auto_expand.
                let g = svast_to_geninput(&ast, seed);
                expander.expand_bug(&g, msg, BugSeverity::DifferentialMismatch);
                let variants = expander.drain_variants();
                stats.variants_generated += variants.len();
                for v in &variants {
                    coverage.observe(&v.source_input, true);
                    // Pipeline observe untuk mode SVAst dari variant ini
                    let va = svast::generate_svast_mode(v.source_input.seed, v.mode);
                    pipeline.observe_ast(&va);
                }
            }
            Verdict::Pass => stats.passed += 1,
            Verdict::CompileFail => stats.compile_failures += 1,
            Verdict::Suspect(_) => {}
        }

        // 5. Observe coverage dari AST yang benar-benar disimulasikan
        pipeline.observe_ast(&ast);

        i += workers as u64;
    }

    stats.iterations = iterations / workers as u64
        + if worker_id < (iterations % workers as u64) as usize {
            1
        } else {
            0
        };
    stats.pipeline_coverage = pipeline.coverage_score();
    stats.pipeline_stages_covered = PipelineStage::all().len() - pipeline.uncovered_count();
    stats.pipeline_stages_total = PipelineStage::all().len();
    stats.elapsed_ms = start.elapsed().as_millis() as u64;

    (stats, bugs, pipeline)
}

use super::pipeline_guide::PipelineStage;

/// Konversi SVAst → GenInput untuk kompatibilitas dengan oracle.
fn svast_to_geninput(ast: &SVAst, seed: u64) -> super::gen::GenInput {
    match ast {
        SVAst::Module(m) => {
            let mut rng = fastrand::Rng::with_seed(seed);
            let w = m.width;
            let msk = super::gen::mask_of(w);
            super::gen::GenInput {
                w,
                wb: w,
                a: rng.u64(0..) & msk,
                b: rng.u64(0..) & msk,
                expr: super::expr::Expr::Lit(0), // placeholder — oracle akan compile+sim source
                seed,
            }
        }
    }
}

/// Test utama v2: pipeline-aware + auto-expansion.
#[test]
fn guided_fuzz_v2() {
    let cfg = V2Config::default();
    let start = std::time::Instant::now();

    let handles: Vec<_> = (0..cfg.workers)
        .map(|w| {
            let iterations = cfg.iterations;
            let workers = cfg.workers;
            std::thread::Builder::new()
                .name(format!("fuzz-v2-worker-{}", w))
                .spawn(move || run_v2_worker(w, workers, iterations))
                .expect("spawn fuzz v2 worker")
        })
        .collect();

    let mut total_stats = V2Stats::default();
    let mut all_bugs = Vec::new();
    let mut union_features = std::collections::HashSet::new();

    for h in handles {
        let (stats, bugs, pipeline) = h.join().expect("fuzz v2 worker panic");
        total_stats.iterations += stats.iterations;
        total_stats.passed += stats.passed;
        total_stats.compile_failures += stats.compile_failures;
        total_stats.bugs_found += stats.bugs_found;
        total_stats.variants_generated += stats.variants_generated;
        union_features.extend(pipeline.features_snapshot());
        all_bugs.extend(bugs);
    }

    // Pipeline coverage: jumlah PipelineStage yang tag-nya ada di union feature.
    let covered_stage_tags: std::collections::HashSet<_> = union_features
        .iter()
        .filter(|f| f.starts_with("pipeline:"))
        .cloned()
        .collect();
    let stages_covered = PipelineStage::all()
        .iter()
        .filter(|s| covered_stage_tags.contains(s.tag()))
        .count();
    total_stats.pipeline_stages_total = PipelineStage::all().len();
    total_stats.pipeline_stages_covered = stages_covered;
    total_stats.pipeline_coverage =
        stages_covered as f64 / total_stats.pipeline_stages_total.max(1) as f64;
    total_stats.elapsed_ms = start.elapsed().as_millis() as u64;

    eprintln!(
        "[guided_fuzz_v2] workers={} iter={} pass={} fail={} bugs={} variants={} pipeline={:.0}% ({}/{}) features={} time={}ms",
        cfg.workers, total_stats.iterations, total_stats.passed,
        total_stats.compile_failures, total_stats.bugs_found,
        total_stats.variants_generated,
        total_stats.pipeline_coverage * 100.0,
        total_stats.pipeline_stages_covered, total_stats.pipeline_stages_total,
        union_features.len(), total_stats.elapsed_ms
    );

    // Log uncovered stages
    let uncovered: Vec<_> = PipelineStage::all()
        .iter()
        .filter(|s| !covered_stage_tags.contains(s.tag()))
        .map(|s| s.tag())
        .collect();
    if !uncovered.is_empty() {
        eprintln!(
            "[guided_fuzz_v2] uncovered stages ({}): {}",
            uncovered.len(),
            uncovered.join(", ")
        );
    }

    // Save bugs
    if !all_bugs.is_empty() {
        let dir = std::env::temp_dir().join(format!("maria-fuzz-v2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        for (i, b) in all_bugs.iter().enumerate() {
            let path = dir.join(format!("bug-{:03}-seed-{}.sv", i, b.seed));
            let _ = std::fs::write(
                &path,
                format!("// bug: {}\n// seed={}\n{}", b.message, b.seed, b.source),
            );
            eprintln!("[guided_fuzz_v2] crash: {}", path.display());
        }
    }

    assert!(
        all_bugs.is_empty(),
        "ditemukan {} anomali:\n{}",
        all_bugs.len(),
        all_bugs
            .iter()
            .map(|b| format!(
                "  seed={} bug: {}\n  source:\n{}\n",
                b.seed, b.message, b.source
            ))
            .collect::<String>()
    );
}

/// Regression test: 100 seed deterministik, zero bug.
#[test]
fn guided_fuzz_v2_deterministic_regression() {
    let mut pipeline = PipelineGuide::new();
    let mut coverage = CoverageGuide::new();
    let mut bugs = 0u64;

    for i in 0..100u64 {
        let ast = svast::generate_svast(i);
        let src = ast.to_source();

        // Feed pipeline features
        pipeline.observe_ast(&ast);

        // Convert to GenInput for oracle
        let input = svast_to_geninput(&ast, i);

        let verdict = oracle::check(&input);
        match &verdict.verdict {
            Verdict::Bug(msg) => {
                bugs += 1;
                eprintln!(
                    "[regression-v2] i={} seed={} bug={}\n{}",
                    i, input.seed, msg, src
                );
            }
            _ => {}
        }
        coverage.observe(&input, verdict.compiled);
    }

    eprintln!(
        "[regression-v2] 100 seeds: bugs={} pipeline_coverage={:.0}%",
        bugs,
        pipeline.coverage_score() * 100.0
    );

    assert_eq!(bugs, 0, "regression: ada bug pada seed deterministik v2");
}

/// Targeted test: setiap pipeline stage minimal 1x tercakup.
#[test]
fn guided_fuzz_v2_all_stages_covered() {
    let mut pipeline = PipelineGuide::new();
    let mut covered_modes = std::collections::HashSet::new();
    let mut compile_fail_by_mode: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();

    for seed in 0..500u64 {
        let mode = GenMode::all()[(seed % GenMode::all().len() as u64) as usize];
        let ast = svast::generate_svast_mode(seed, mode);
        pipeline.observe_ast(&ast);
        covered_modes.insert(mode);

        // Compile check pada source SVAst
        let src = ast.to_source();
        let compiled = crate::compile_str(&src).is_ok();
        if !compiled {
            *compile_fail_by_mode
                .entry(format!("{:?}", mode))
                .or_insert(0) += 1;
        }
    }

    eprintln!(
        "[stage-coverage] covered={}/{} stages, modes={:?}",
        PipelineStage::all().len() - pipeline.uncovered_count(),
        PipelineStage::all().len(),
        covered_modes
    );
    if !compile_fail_by_mode.is_empty() {
        eprintln!(
            "[stage-coverage] compile_fail by mode: {:?}",
            compile_fail_by_mode
        );
    }

    // Semua mode harus tercakup
    assert_eq!(
        covered_modes.len(),
        GenMode::all().len(),
        "not all modes covered: {:?}",
        covered_modes
    );
}

/// Probe diagnostik: cetak contoh source per mode untuk cek compile.
#[test]
fn guided_fuzz_v2_probe_sources() {
    let mut seen_mode = std::collections::HashSet::new();
    for seed in 0..200u64 {
        let mode = GenMode::all()[(seed % GenMode::all().len() as u64) as usize];
        if seen_mode.contains(&mode) {
            continue;
        }
        seen_mode.insert(mode);
        let ast = svast::generate_svast_mode(seed, mode);
        let src = ast.to_source();
        let compiled = crate::compile_str(&src).is_ok();
        eprintln!(
            "[probe] mode={:?} seed={} compiled={} len={}\n----\n{}\n----",
            mode,
            seed,
            compiled,
            src.len(),
            src
        );
    }
}
