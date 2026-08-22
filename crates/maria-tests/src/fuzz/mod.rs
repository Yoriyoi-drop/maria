//! Metode test fuzzing terarah (guided structure-aware fuzzing).
//!
//! Berbeda dari fuzzing buta (byte acak): generator menghasilkan SV
//! *well-formed* (grammar-aware), coverage guide mengarahkan mutasi ke fitur
//! bahasa yang belum tereksekusi, dan oracle memverifikasi via *differential
//! testing* terhadap model emas `Expr::eval`. Tujuannya menemukan panic,
//! non-determinism, atau ketidakcocokan semantik di pipeline Maria.

mod expr;
mod gen;
mod guide;
mod oracle;

use gen::GenInput;
use guide::CoverageGuide;
use oracle::{check, Verdict};

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
