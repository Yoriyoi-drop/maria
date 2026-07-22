//! Benchmark suite — ukur performa di release mode.
//!
//! Run: cargo test --release -- --ignored bench_release::
//!       (requires benchmark feature to be enabled)

use crate::compile_str;
use crate::frontend::compile_session::{CompileSession, SessionConfig};
use crate::intern::Symbol;
use std::time::Instant;

#[test]
#[ignore]
fn bench_release_compile_counter() {
    let src = include_str!("../../test/counter.sv");
    let start = Instant::now();
    for _ in 0..100 {
        let _ = compile_str(src).unwrap();
    }
    let avg = start.elapsed() / 100;
    eprintln!("counter.sv: {:.1} µs avg (100x)", avg.as_nanos() as f64 / 1000.0);
}

#[test]
#[ignore]
fn bench_release_parse_large() {
    let mut src = String::new();
    for i in 0..1000 {
        src.push_str(&format!(
            "module m_{}(input clk, output reg [7:0] q);
             always_ff @(posedge clk) q <= q + 8'h1;
             endmodule\n", i));
    }
    let start = Instant::now();
    let design = compile_str(&src).unwrap();
    let elapsed = start.elapsed();
    eprintln!("1000 modules: {:?} ({} modules)", elapsed, design.modules.len());
}

#[test]
#[ignore]
fn bench_release_string_intern() {
    let start = Instant::now();
    for i in 0..100000 {
        let s = format!("bench_var_{}", i);
        let _sym = Symbol::intern(&s);
    }
    let elapsed = start.elapsed();
    eprintln!("100K symbols: {:?} ({:.0} sym/sec)",
        elapsed, 100000.0 / elapsed.as_secs_f64());
}

#[test]
#[ignore]
fn bench_release_session_100_files() {
    let dir = std::env::temp_dir().join("maria_release_bench");
    let _ = std::fs::create_dir_all(&dir);
    let mut sources = Vec::new();
    for i in 0..100 {
        let p = dir.join(format!("m_{}.sv", i));
        let c = format!(
            "module m_{}(input clk, output reg [7:0] q);
             always_ff @(posedge clk) q <= q + 8'h1;
             endmodule\n", i);
        std::fs::write(&p, &c).unwrap();
        sources.push(p);
    }
    let config = SessionConfig { sources, ..Default::default() };
    let mut session = CompileSession::new(config);
    let start = Instant::now();
    let (_design, _idx) = session.compile().unwrap();
    let elapsed = start.elapsed();
    session.print_timing();
    eprintln!("100 files: {:?}", elapsed);
    let _ = std::fs::remove_dir_all(&dir);
}
