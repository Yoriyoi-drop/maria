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

#[allow(dead_code)]
mod expr;
#[allow(dead_code)]
mod gen;
#[allow(dead_code)]
mod guide;
#[allow(dead_code)]
mod oracle;
#[allow(dead_code)]
mod semantic_mutator;
#[allow(dead_code)]
mod hierarchy_mutator;
#[allow(dead_code)]
mod minimizer;
#[allow(dead_code)]
mod differential;
#[allow(dead_code)]
mod mgte;

use gen::GenInput;
use guide::CoverageGuide;
use oracle::{check, Verdict};
use mgte::{MGTE, MGTEConfig, MGTEMode};

#[test]
fn guided_fuzz() {
    let n: u64 = std::env::var("MARIA_FUZZ_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);

    let mut guide = CoverageGuide::new();
    let mut bugs: Vec<(GenInput, String)> = Vec::new();
    let mut pass = 0u64;
    let mut compile_fail = 0u64;

    for i in 0..n {
        let input = guide.next(i);
        use std::io::Write;
        let _ = std::io::stderr().flush();
        eprintln!("[fuzz] iter {}/{} seed={}", i, n, input.seed);
        let _ = std::io::stderr().flush();
        let result = check(&input);
        let compiled = result.compiled;

        match &result.verdict {
            Verdict::Bug(_) => bugs.push((input.clone(), describe(&input, &result.verdict))),
            Verdict::Pass => pass += 1,
            Verdict::CompileFail => compile_fail += 1,
        }

        guide.observe(&input, compiled);
    }

    eprintln!(
        "[guided_fuzz] iter={} pass={} compile_fail={} coverage={} corpus={}",
        n,
        pass,
        compile_fail,
        guide.coverage_len(),
        guide.corpus_len()
    );

    assert!(
        bugs.is_empty(),
        "ditemukan {} anomali:\n{}",
        bugs.len(),
        bugs
            .iter()
            .map(|(i, m)| format!(
                "  seed={} w={} expr=`{}`\n    source:\n{}\n    bug: {}\n",
                i.seed,
                i.w,
                i.expr.to_sv(i.w),
                indent(&i.to_source()),
                m
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

fn describe(input: &GenInput, v: &Verdict) -> String {
    if let Verdict::Bug(m) = v {
        format!("{} (expected={:#x})", m, input.expr.eval(input.w, input.a, input.b))
    } else {
        String::new()
    }
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
    assert_eq!(stats.bugs_found, 0, "regression: MGTE found bugs on deterministic seed");
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
