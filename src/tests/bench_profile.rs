//! Benchmark + profiling — ukur performa compile di berbagai fase.
//!
//! Run: cargo test -- --ignored bench_profile::

use crate::compile_str;
use crate::frontend::compile_session::{CompileSession, SessionConfig};
use crate::profiling::{Profiler, Phase, PhaseTimer, Counter};
use std::time::Instant;

#[test]
#[ignore]
fn bench_compile_counter_sv() {
    let source = include_str!("../../test/counter.sv");
    let start = Instant::now();
    for _ in 0..10 {
        let _ = compile_str(source).unwrap();
    }
    let elapsed = start.elapsed() / 10;
    eprintln!("counter.sv: avg {:?} per compile", elapsed);
    assert!(elapsed.as_millis() < 5000, "too slow: {:?}", elapsed);
}

#[test]
#[ignore]
fn bench_compile_100_modules() {
    let mut source = String::new();
    for i in 0..100 {
        source.push_str(&format!(r#"module m_{}(input clk, output reg [3:0] q);
            always_ff @(posedge clk) q <= q + 4'h1;
        endmodule
"#, i));
    }
    let start = Instant::now();
    let _ = compile_str(&source).unwrap();
    let elapsed = start.elapsed();
    eprintln!("100 modules: {:?}", elapsed);
    assert!(elapsed.as_secs() < 10, "too slow: {:?}", elapsed);
}

#[test]
#[ignore]
fn bench_profiler_overhead() {
    let profiler = Profiler::new();
    let count = 1_000_000;
    let start = Instant::now();
    for _ in 0..count {
        profiler.count(Counter::TokensLexed, 1);
    }
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / count as f64;
    eprintln!("Profiler count overhead: {:.1} ns/op", ns_per_op);
    assert!(ns_per_op < 200.0, "too slow: {:.1} ns/op", ns_per_op);
}

#[test]
#[ignore]
fn bench_session_50_files() {
    let dir = std::env::temp_dir().join("maria_bench_50");
    let _ = std::fs::create_dir_all(&dir);
    let mut sources = Vec::new();
    for i in 0..50 {
        let path = dir.join(format!("mod_{}.sv", i));
        let content = format!(
            "module mod_{}(input clk, output reg [7:0] q);\n\
             always_ff @(posedge clk) q <= q + 8'h1;\n\
             endmodule\n", i);
        std::fs::write(&path, &content).unwrap();
        sources.push(path);
    }
    let config = SessionConfig {
        sources,
        ..Default::default()
    };
    let mut session = CompileSession::new(config);
    let start = Instant::now();
    let (_design, _index) = session.compile().unwrap();
    let elapsed = start.elapsed();
    session.print_timing();
    eprintln!("Session 50 files: {:?}", elapsed);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn bench_compares_legacy_vs_simd_lexer() {
    use crate::frontend::lexer::lexer::{tokenize_legacy, tokenize_simd};
    let source = include_str!("../../test/counter.sv");

    let start = Instant::now();
    for _ in 0..100 {
        let _ = tokenize_legacy(source);
    }
    let legacy_time = start.elapsed();

    let start = Instant::now();
    for _ in 0..100 {
        let _ = tokenize_simd(source);
    }
    let simd_time = start.elapsed();

    eprintln!("Legacy lexer: {:?} for 100 iterations", legacy_time);
    eprintln!("SIMD lexer:  {:?} for 100 iterations", simd_time);
    eprintln!("Speedup: {:.1}x", legacy_time.as_nanos() as f64 / simd_time.as_nanos() as f64);
}

#[test]
#[ignore]
fn bench_string_intern_speed() {
    use crate::intern::Symbol;
    let start = Instant::now();
    for i in 0..20000 {
        let s = format!("very_long_variable_name_{}", i);
        let _sym = Symbol::intern(&s);
    }
    let elapsed = start.elapsed();
    eprintln!("20K unique symbols: {:?} ({:.0} symbols/sec)",
        elapsed, 20000.0 / elapsed.as_secs_f64());
}
