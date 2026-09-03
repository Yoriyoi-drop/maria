//! Metode test fuzzing terarah (guided structure-aware fuzzing).
//!
//! Struktur:
//! - `fuzz/{arithmetic,operator,control,...}/` — test modules per kategori
//! - `fuzz/` root — infra modules (expr, gen, guide, oracle, parallel, mgte, …)
//! - `guided_fuzz()` — orchestrated test utama

// ── Infrastructure (shared across all fuzz modules) ──────────────────────
#[cfg(test)]
pub(crate) mod expr;
#[cfg(test)]
pub(crate) mod gen;
#[cfg(test)]
pub(crate) mod guide;
#[cfg(test)]
pub(crate) mod oracle;
#[cfg(test)]
pub(crate) mod parallel;
#[cfg(test)]
pub(crate) mod mgte;
#[cfg(test)]
pub(crate) mod minimizer;
#[cfg(test)]
pub(crate) mod semantic_mutator;
#[cfg(test)]
pub(crate) mod hierarchy_mutator;
#[cfg(test)]
pub(crate) mod differential;
// ── Paper-inspired enhancements ──────────────────────────────────────────
// Paper #2 (ChipFuzzer): bug database + bug-guided prioritization
#[cfg(test)]
pub(crate) mod bug_db;
// Paper #5 (EMI): differential compiler testing via equivalence modulo inputs
#[cfg(test)]
pub(crate) mod emi;
// Paper #7 (MOpt): adaptive mutation scheduling
#[cfg(test)]
pub(crate) mod mutation_scheduler;

// ── Unified AST + Pipeline-Aware Coverage + Auto-Expansion ───────────────
// AST gabungan: satu representasi untuk semua fitur SV (mengganti 30+ mini-AST)
#[cfg(test)]
pub(crate) mod svast;
// Coverage pipeline-aware: tracking stage mana yang sudah terlatih
#[cfg(test)]
pub(crate) mod pipeline_guide;
// Bug-guided auto-expansion: otomatis generate vari setelah bug ditemukan
#[cfg(test)]
pub(crate) mod auto_expand;
// Orchestration v2: pipeline-aware + auto-expansion
#[cfg(test)]
pub(crate) mod guided_fuzz_v2;

// ── Test modules per kategori ────────────────────────────────────────────
mod arithmetic;
mod operator;
mod control;
mod shift;
mod assign;
mod robustness;
mod misc;
mod infra;

use gen::GenInput;
use guide::CoverageGuide;
use mgte::{MGTEConfig, MGTEMode, MGTE};
use oracle::{check, Verdict};
use parallel::{ParallelConfig, DEFAULT_WORKERS};

// ── Guided fuzz test (orchestrated) ──────────────────────────────────────

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
    let mut guide = CoverageGuide::new();
    let mut bugs = 0u64;
    for i in 0..50 {
        let input = guide.next(i);
        if let Verdict::Bug(ref msg) = check(&input).verdict {
            bugs += 1;
            eprintln!(
                "[regression-dbg] i={} seed={} w={} wb={} expr=`{}`\n{}\nmsg={}",
                i,
                input.seed,
                input.w,
                input.wb,
                input.expr.to_sv(input.w),
                input.to_source(),
                msg
            );
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
