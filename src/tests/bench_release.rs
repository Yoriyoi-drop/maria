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

#[test]
#[ignore]
fn bench_release_opentitan_compile() {
    // Compile all OpenTitan RTL files listed in opentitan_rtl.f
    let file_list_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("opentitan_rtl.f");
    let content = std::fs::read_to_string(&file_list_path)
        .expect("opentitan_rtl.f not found");

    let manifest = env!("CARGO_MANIFEST_DIR");
    let sources: Vec<std::path::PathBuf> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let p = std::path::Path::new(manifest).join(l);
            assert!(p.exists(), "file not found: {:?}", p);
            p
        })
        .collect();

    eprintln!("OpenTitan RTL files: {}", sources.len());

    // Cold compile via compile_files (tolerates partial failures)
    use crate::compile_files;
    let string_sources: Vec<String> = sources
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    let start = Instant::now();
    match compile_files(&string_sources) {
        Ok(design) => {
            let elapsed = start.elapsed();
            eprintln!(
                "OpenTitan cold compile: {:?} ({} modules, {} classes, top={})",
                elapsed,
                design.modules.len(),
                design.classes.len(),
                design.top.name
            );
        }
        Err(e) => {
            let elapsed = start.elapsed();
            eprintln!("OpenTitan compile partially failed after {:?}: {:?}", elapsed, e);
            eprintln!("Note: OpenTitan uses advanced SV features (reggen output, interfaces, etc.)");
            eprintln!("that Maria's parser doesn't fully support yet.");
            // Don't panic — this is a benchmark, not a correctness test
        }
    }
}

#[test]
#[ignore]
fn bench_release_opentitan_warm_compile() {
    // Measure warm (cached) compile after a cold compile
    let file_list_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("opentitan_rtl.f");
    let content = std::fs::read_to_string(&file_list_path)
        .expect("opentitan_rtl.f not found");

    let manifest = env!("CARGO_MANIFEST_DIR");
    let sources: Vec<std::path::PathBuf> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| std::path::Path::new(manifest).join(l))
        .collect();

    // First compile to warm cache
    {
        let config = SessionConfig {
            sources: sources.clone(),
            ..Default::default()
        };
        let mut session = CompileSession::new(config);
        let _ = session.compile().expect("warm-up compile failed");
    }

    // Second compile — should hit cache
    {
        let config = SessionConfig {
            sources: sources.clone(),
            ..Default::default()
        };
        let mut session = CompileSession::new(config);
        let start = Instant::now();
        match session.compile() {
            Ok((design, _idx)) => {
                let elapsed = start.elapsed();
                session.print_timing();
                eprintln!(
                    "OpenTitan warm (cached) compile: {:?} ({} modules)",
                    elapsed,
                    design.modules.len()
                );
            }
            Err(e) => {
                eprintln!("OpenTitan warm compile failed: {:?}", e);
            }
        }
    }
}
