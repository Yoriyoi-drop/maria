//! Metode test fuzzing terarah (guided structure-aware fuzzing).
//!
//! Berbeda dari fuzzing buta (byte acak): generator menghasilkan SV
//! *well-formed* (grammar-aware), coverage guide mengarahkan mutasi ke fitur
//! bahasa yang belum tereksekusi, dan oracle memverifikasi via *differential
//! testing* terhadap model emas `Expr::eval`. Tujuannya menemukan panic,
//! non-determinism, atau ketidakcocokan semantik di pipeline Maria.
//!
//! MGTE (Maria Guided Test Engine) mengintegrasikan:
//! - Semantic Mutator (mutasi berdasarkan tipe & konteks)
//! - Hierarchy Mutator (mutasi subtree modul)
//! - Testcase Minimizer (mengecilkan reproducer)
//! - Differential Executor (bandingkan dengan Verilator/Icarus)

#[cfg(test)]
mod concurrency_diff;
#[allow(dead_code)]
mod differential;
#[cfg(test)]
mod eof_truncation;
#[allow(dead_code)]
mod expr;
#[allow(dead_code)]
mod gen;
#[allow(dead_code)]
mod guide;
#[allow(dead_code)]
mod hierarchy_mutator;
#[cfg(test)]
mod metamorphic;
#[allow(dead_code)]
mod mgte;
#[allow(dead_code)]
mod minimizer;
#[allow(dead_code)]
mod oracle;
#[allow(dead_code)]
mod parallel;
#[cfg(test)]
mod preproc;
#[cfg(test)]
mod resources;
#[cfg(test)]
mod scheduler;
#[allow(dead_code)]
mod semantic_mutator;
#[cfg(test)]
mod waveform;

use gen::GenInput;
use guide::CoverageGuide;
use mgte::{MGTEConfig, MGTEMode, MGTE};
use oracle::{check, Verdict};
use parallel::{ParallelConfig, DEFAULT_WORKERS};

#[test]
fn guided_fuzz() {
    let n: u64 = std::env::var("MARIA_FUZZ_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let workers: usize = std::env::var("MARIA_FUZZ_WORKERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_WORKERS);

    let (stats, bugs) = parallel::run_parallel(ParallelConfig {
        workers,
        iterations: n,
    });

    eprintln!(
        "[guided_fuzz] workers={} iter={} pass={} compile_fail={} bugs={} coverage={} corpus={} time={}ms",
        workers,
        stats.iterations,
        stats.passed,
        stats.compile_failures,
        stats.bugs_found,
        stats.coverage_features,
        stats.corpus_size,
        stats.elapsed_ms
    );

    // Temuan bottleneck (bukan kegagalan keras — info performa pipeline).
    if !stats.bottlenecks.is_empty() {
        eprintln!(
            "[guided_fuzz] {} bottleneck (>{}us per testcase):{}",
            stats.bottlenecks.len(),
            parallel::BOTTLENECK_THRESHOLD_US,
            stats
                .bottlenecks
                .iter()
                .take(5)
                .map(|s| format!("\n  seed={} {:.3}s", s.seed, s.micros as f64 / 1e6))
                .collect::<String>()
        );
    }

    // Persist reproducer bug ke disk agar bisa dianalisis ulang tanpa
    // mengulang run (dulu hanya stderr — hilang begitu terminal ditutup).
    if !bugs.is_empty() {
        let dir = std::env::temp_dir().join(format!("maria-fuzz-crashes-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        for (i, b) in bugs.iter().enumerate() {
            let path = dir.join(format!("bug-{:03}-seed-{}.sv", i, b.input.seed));
            let _ = std::fs::write(
                &path,
                format!(
                    "// bug: {}\n// seed={} w={}\n{}",
                    b.message,
                    b.input.seed,
                    b.input.w,
                    b.input.to_source()
                ),
            );
            eprintln!("[guided_fuzz] crash disimpan: {}", path.display());
        }
    }

    assert!(
        bugs.is_empty(),
        "ditemukan {} anomali:\n{}",
        bugs.len(),
        bugs.iter()
            .map(|b| format!(
                "  seed={} w={} expr=`{}`\n    source:\n{}\n    bug: {}\n",
                b.input.seed,
                b.input.w,
                b.input.expr.to_sv(b.input.w),
                indent(&b.input.to_source()),
                b.message
            ))
            .collect::<String>()
    );
}

#[test]
fn guided_fuzz_deterministic_regression() {
    // Seed tetap → hasil stabil antar run (regression guard untuk pipeline).
    let mut guide = CoverageGuide::new();
    let mut bugs = 0u64;
    for i in 0..50 {
        let input = guide.next(i);
        if matches!(check(&input).verdict, Verdict::Bug(_)) {
            bugs += 1;
        }
        guide.observe(&input, true);
    }
    assert_eq!(bugs, 0, "regression: ada bug pada seed deterministik");
}

fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("    {}", l))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn mgte_semantic_mode() {
    let config = MGTEConfig {
        modes: vec![MGTEMode::Semantic],
        iterations: 50,
        enable_differential: false,
        enable_minimizer: false,
        ..Default::default()
    };
    let mut mgte = MGTE::new(config);
    let stats = mgte.run();

    eprintln!("[MGTE Semantic] {}", stats_summary(&stats));
    assert!(stats.total_iterations > 0);
}

#[test]
fn mgte_elaboration_mode() {
    let mut config = MGTEConfig::for_elaboration();
    config.iterations = 50;
    config.enable_differential = false;
    config.enable_minimizer = false;
    let mut mgte = MGTE::new(config);
    let stats = mgte.run();

    eprintln!("[MGTE Elaboration] {}", stats_summary(&stats));
    assert!(stats.total_iterations > 0);
}

#[test]
fn mgte_simulator_mode() {
    let mut config = MGTEConfig::for_simulator();
    config.iterations = 30;
    config.enable_differential = false;
    config.enable_minimizer = false;
    let mut mgte = MGTE::new(config);
    let stats = mgte.run();

    eprintln!("[MGTE Simulator] {}", stats_summary(&stats));
    assert!(stats.total_iterations > 0);
}

#[test]
fn mgte_deterministic_regression() {
    let config = MGTEConfig {
        modes: vec![MGTEMode::Semantic, MGTEMode::Elaboration],
        iterations: 100,
        seed: 0xDEADBEEF,
        enable_differential: false,
        enable_minimizer: false,
        ..Default::default()
    };
    let mut mgte = MGTE::new(config);
    let stats = mgte.run();

    eprintln!("[MGTE Deterministic] {}", stats_summary(&stats));
    assert_eq!(
        stats.bugs_found, 0,
        "regression: MGTE found bugs on deterministic seed"
    );
}

#[test]
fn mgte_parallel_10_workers() {
    // Multi jalur MGTE: 10 worker paralel, iterasi dibagi rata.
    let config = MGTEConfig {
        modes: vec![MGTEMode::Semantic, MGTEMode::Elaboration],
        iterations: 100,
        seed: 0xC0FFEE,
        enable_differential: false,
        enable_minimizer: false,
        ..Default::default()
    };
    let stats = MGTE::run_parallel(config, DEFAULT_WORKERS);

    eprintln!("[MGTE Parallel] {}", stats_summary(&stats));
    assert_eq!(stats.total_iterations, 100);
    // Semua iterasi harus tercatat dengan outcome salah satu kategori.
    assert!(stats.passed + stats.compile_failures >= 90);
    // Dulu hanya cek total — bugs_found > 0 lolos tanpa gagal. Sekarang
    // bug apapun = kegagalan keras, konsisten dengan mode deterministik.
    assert_eq!(
        stats.bugs_found, 0,
        "regression: MGTE parallel menemukan {} bug",
        stats.bugs_found
    );
}

fn stats_summary(stats: &mgte::MGTEStats) -> String {
    format!(
        "iter={} pass={} fail={} bugs={} diff={} min={} cov={} corpus={} time={}ms",
        stats.total_iterations,
        stats.passed,
        stats.compile_failures,
        stats.bugs_found,
        stats.differential_mismatches,
        stats.minimized_cases,
        stats.coverage_features,
        stats.corpus_size,
        stats.elapsed_ms
    )
}
