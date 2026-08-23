//! Benchmark suite — ukur performa di release mode.
//!
//! Run: cargo test --release -- --ignored bench_release::
//!
//! Includes simulation throughput (cycles/sec), elaboration time,
//! and memory usage tracking for regression detection.

use crate::compile_str;
use maria_compiler::frontend::compile_session::{CompileSession, SessionConfig};
use maria_core::intern::Symbol;
use std::time::Instant;

// ─── Memory tracking (Linux /proc/self/status) ───

/// Read peak virtual memory (VmPeak) from /proc/self/status in MB.
/// Returns 0.0 on non-Linux or if file is unreadable.
fn peak_vmem_mb() -> f64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else { return 0.0; };
    for line in status.lines() {
        if let Some(val) = line.strip_prefix("VmPeak:") {
            let val = val.trim();
            if let Some(kb_str) = val.strip_suffix(" kB") {
                if let Ok(kb) = kb_str.trim().parse::<f64>() {
                    return kb / 1024.0;
                }
            }
        }
    }
    0.0
}

/// Read current virtual memory (VmRSS) from /proc/self/status in MB.
fn current_rss_mb() -> f64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else { return 0.0; };
    for line in status.lines() {
        if let Some(val) = line.strip_prefix("VmRSS:") {
            let val = val.trim();
            if let Some(kb_str) = val.strip_suffix(" kB") {
                if let Ok(kb) = kb_str.trim().parse::<f64>() {
                    return kb / 1024.0;
                }
            }
        }
    }
    0.0
}

/// Generate a simulation design with N-cycle counter and clock.
fn gen_sim_design(cycles: u64) -> String {
    format!(r#"
module bench_sim (
    input clk,
    input rst_n
);
    reg [31:0] count;
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n)
            count <= 32'h0;
        else
            count <= count + 32'h1;
    end
endmodule

module tb;
    reg clk = 0;
    reg rst_n = 0;
    bench_sim uut (.clk(clk), .rst_n(rst_n));
    initial begin
        #1 rst_n = 1;
        repeat ({}) @(posedge clk);
        $finish;
    end
    always #1 clk = ~clk;
endmodule
"#, cycles)
}

/// Generate N parallel always_comb blocks for throughput scaling test.
fn gen_parallel_design(blocks: usize) -> String {
    let mut src = String::from(
        "module bench_parallel;\n"
    );
    // Declare signals
    src.push_str("    reg [7:0] a, b;\n");
    for i in 0..blocks {
        src.push_str(&format!("    reg [7:0] out_{};\n", i));
    }
    // Always_comb blocks
    for i in 0..blocks {
        src.push_str(&format!(
            "    always_comb begin out_{} = a + b; end\n", i
        ));
    }
    // Stimulus
    src.push_str(
        "    initial begin\n"
    );
    src.push_str("        a = 10; b = 20;\n");
    src.push_str("        #1;\n");
    if blocks > 0 {
        // Trigger sensitivity by changing a
        src.push_str("        a = 30;\n");
        src.push_str("        #1;\n");
    }
    src.push_str("        $finish;\n");
    src.push_str("    end\n");
    src.push_str("endmodule\n");
    src
}

/// Generate N modules with inter-dependencies for elaboration benchmark.
fn gen_elab_design(count: usize) -> String {
    let mut src = String::with_capacity(count * 200);
    for i in 0..count {
        src.push_str(&format!(
            "module elab_{}(input clk, output reg [7:0] q);
             always_ff @(posedge clk) q <= q + 8'h1;
             endmodule\n", i));
    }
    // Top tunggal yang meng-instansiasi semua submodule → tepat satu top
    // candidate (StrictSimulation menolak design dengan banyak candidate tops).
    src.push_str("module top;\n    wire clk;\n");
    for i in 0..count {
        src.push_str(&format!("    wire [7:0] q_{};\n", i));
    }
    for i in 0..count {
        src.push_str(&format!(
            "    elab_{} u{}(.clk(clk), .q(q_{}));\n",
            i, i, i
        ));
    }
    src.push_str("endmodule\n");
    src
}

#[test]
#[ignore] // benchmark: 100K cycles sim di debug > 3 menit — jalankan manual via --ignored
fn bench_release_sim_throughput_counter() {
    // Simulation throughput: counter design, 100K cycles
    let cycles = 100_000u64;
    let source = gen_sim_design(cycles);

    let design = compile_str(&source).unwrap();
    let mut engine = crate::simulator::SimulationEngine::new(design, cycles * 2 + 10);

    let mem_before = peak_vmem_mb();
    let start = Instant::now();
    engine.run().unwrap();
    let elapsed = start.elapsed();
    let mem_after = peak_vmem_mb();

    let cycles_per_sec = cycles as f64 / elapsed.as_secs_f64();
    let ns_per_cycle = elapsed.as_nanos() as f64 / cycles as f64;
    let mem_delta = mem_after - mem_before;

    eprintln!("═══ Simulation Throughput (Counter) ═══");
    eprintln!("  Cycles:          {}", cycles);
    eprintln!("  Elapsed:         {:?}", elapsed);
    eprintln!("  Throughput:      {:.0} cycles/sec", cycles_per_sec);
    eprintln!("  Latency:         {:.1} ns/cycle", ns_per_cycle);
    eprintln!("  Sim time:        {}", engine.state.time);
    eprintln!("  Peak VMem:       {:.1} MB", mem_before);
    eprintln!("  Peak VMem after: {:.1} MB", mem_after);
    eprintln!("  Mem delta:       {:.1} MB", mem_delta);

    // Assert minimum performance (at least 10K cycles/sec in debug mode)
    assert!(cycles_per_sec > 10_000.0,
        "Simulation too slow: {:.0} cycles/sec (expected >10K)", cycles_per_sec);
}

#[test]
#[ignore]
fn bench_release_sim_throughput_counter_1m() {
    // Simulation throughput: 1M cycles counter (for longer benchmark)
    let cycles = 1_000_000u64;
    let source = gen_sim_design(cycles);

    let design = compile_str(&source).unwrap();
    let mut engine = crate::simulator::SimulationEngine::new(design, cycles * 2 + 10);

    let mem_before = peak_vmem_mb();
    let start = Instant::now();
    engine.run().unwrap();
    let elapsed = start.elapsed();
    let mem_after = peak_vmem_mb();

    let cycles_per_sec = cycles as f64 / elapsed.as_secs_f64();
    let mem_delta = mem_after - mem_before;

    eprintln!("═══ Simulation Throughput (1M Cycles) ═══");
    eprintln!("  Cycles:          {}", cycles);
    eprintln!("  Elapsed:         {:?}", elapsed);
    eprintln!("  Throughput:      {:.0} cycles/sec", cycles_per_sec);
    eprintln!("  Sim time:        {}", engine.state.time);
    eprintln!("  Peak VMem after: {:.1} MB", mem_after);
    eprintln!("  Mem delta:       {:.1} MB", mem_delta);

    assert!(cycles_per_sec > 10_000.0,
        "Simulation too slow: {:.0} cycles/sec (expected >10K)", cycles_per_sec);
}

#[test]
#[ignore]
fn bench_release_sim_throughput_parallel() {
    // Simulation throughput with parallel always_comb blocks
    let blocks = 100;
    let source = gen_parallel_design(blocks);

    let design = compile_str(&source).unwrap();
    let mut engine = crate::simulator::SimulationEngine::new(design, 20);

    let mem_before = peak_vmem_mb();
    let start = Instant::now();
    engine.run().unwrap();
    let elapsed = start.elapsed();
    let mem_after = peak_vmem_mb();

    eprintln!("═══ Simulation Throughput ({} Parallel Blocks) ═══", blocks);
    eprintln!("  Elapsed:         {:?}", elapsed);
    eprintln!("  Peak VMem:       {:.1} MB", mem_before);
    eprintln!("  Peak VMem after: {:.1} MB", mem_after);
    eprintln!("  Mem delta:       {:.1} MB", mem_after - mem_before);

    assert!(elapsed.as_secs() < 10,
        "Parallel simulation too slow: {:?}", elapsed);
}

#[test]
#[ignore]
fn bench_release_sim_throughput_busy_loop() {
    // Simulation throughput: intense signal activity
    // Many signals toggling every cycle
    let source = r#"
module bench_busy;
    reg [63:0] ctr1, ctr2, ctr3, ctr4;
    reg clk = 0;
    always #1 clk = ~clk;
    always_ff @(posedge clk) begin
        ctr1 <= ctr1 + 1;
        ctr2 <= ctr2 + 2;
        ctr3 <= ctr3 + 3;
        ctr4 <= ctr4 + 4;
    end
    initial begin
        repeat (50000) @(posedge clk);
        $finish;
    end
endmodule
"#;

    let design = compile_str(source).unwrap();
    let mut engine = crate::simulator::SimulationEngine::new(design, 200_000);

    let _mem_before = peak_vmem_mb();
    let start = Instant::now();
    engine.run().unwrap();
    let elapsed = start.elapsed();
    let mem_after = peak_vmem_mb();

    let cycles_per_sec = 50_000.0 / elapsed.as_secs_f64();

    eprintln!("═══ Simulation Throughput (Busy Loop, 4×64bit counters) ═══");
    eprintln!("  Cycles:          50000");
    eprintln!("  Elapsed:         {:?}", elapsed);
    eprintln!("  Throughput:      {:.0} cycles/sec", cycles_per_sec);
    eprintln!("  Sim time:        {}", engine.state.time);
    eprintln!("  Peak VMem after: {:.1} MB", mem_after);

    assert!(cycles_per_sec > 5_000.0,
        "Busy loop sim too slow: {:.0} cycles/sec", cycles_per_sec);
}

#[test]
#[ignore]
fn bench_release_elaborate_1000_modules() {
    // Measure elaboration time for 1000 modules separately
    let src = gen_elab_design(1000);

    let mem_before = peak_vmem_mb();
    let start = Instant::now();
    let design = compile_str(&src).unwrap();
    let elapsed = start.elapsed();
    let mem_after = peak_vmem_mb();

    eprintln!("═══ Compile & Elaboration (1000 modules) ═══");
    eprintln!("  Modules:         {} ({} in top)", design.modules.len() + 1, design.modules.len());
    eprintln!("  Elapsed:         {:?}", elapsed);
    eprintln!("  Peak VMem:       {:.1} MB", mem_before);
    eprintln!("  Peak VMem after: {:.1} MB", mem_after);
    eprintln!("  Mem delta:       {:.1} MB", mem_after - mem_before);

    assert!(elapsed.as_secs() < 30, "1000 modules too slow: {:?}", elapsed);
}

#[test]
#[ignore]
fn bench_release_elaborate_100_modules() {
    // Measure elaboration time for 100 modules
    let src = gen_elab_design(100);

    let _mem_before = peak_vmem_mb();
    let start = Instant::now();
    let design = compile_str(&src).unwrap();
    let elapsed = start.elapsed();
    let mem_after = peak_vmem_mb();

    eprintln!("═══ Compile & Elaboration (100 modules) ═══");
    eprintln!("  Modules:         {} ({} in top)", design.modules.len() + 1, design.modules.len());
    eprintln!("  Elapsed:         {:?}", elapsed);
    eprintln!("  Rate:            {:.0} modules/sec",
        100.0 / elapsed.as_secs_f64());
    eprintln!("  Peak VMem after: {:.1} MB", mem_after);

    assert!(elapsed.as_secs() < 5, "100 modules too slow: {:?}", elapsed);
}

#[test]
#[ignore]
fn bench_release_elaborate_10000_modules() {
    // Stress test: elaborate 10000 modules (large design)
    let src = gen_elab_design(10_000);

    let _mem_before = peak_vmem_mb();
    let start = Instant::now();
    let design = compile_str(&src).unwrap();
    let elapsed = start.elapsed();
    let mem_after = peak_vmem_mb();

    let modules_per_sec = (design.modules.len() + 1) as f64 / elapsed.as_secs_f64();

    eprintln!("═══ Compile & Elaboration (10000 modules) ═══");
    eprintln!("  Modules:         {} ({} in top)", design.modules.len() + 1, design.modules.len());
    eprintln!("  Elapsed:         {:?}", elapsed);
    eprintln!("  Rate:            {:.0} modules/sec", modules_per_sec);
    eprintln!("  Peak VMem after: {:.1} MB", mem_after);

    assert!(elapsed.as_secs() < 120, "10000 modules too slow: {:?}", elapsed);
}

#[test]
#[ignore]
fn bench_release_memory_open_titan() {
    // Memory usage benchmark: compile OpenTitan (if available), track peak memory
    let file_list_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("opentitan_rtl.f");
    if !file_list_path.exists() {
        eprintln!("OpenTitan file list not found, skipping memory benchmark");
        return;
    }
    let content = std::fs::read_to_string(&file_list_path)
        .expect("opentitan_rtl.f not found");
    let manifest = env!("CARGO_MANIFEST_DIR");
    let sources: Vec<String> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| std::path::Path::new(manifest).join(l).to_string_lossy().to_string())
        .collect();

    eprintln!("OpenTitan RTL files: {}", sources.len());

    // Measure memory before
    let mem_before = peak_vmem_mb();
    let rss_before = current_rss_mb();

    let start = Instant::now();
    match crate::compile_files(&sources) {
        Ok(design) => {
            let elapsed = start.elapsed();
            let mem_after = peak_vmem_mb();
            let rss_after = current_rss_mb();

            eprintln!("═══ Memory Usage (OpenTitan) ═══");
            eprintln!("  Elapsed:         {:?}", elapsed);
            eprintln!("  Modules:         {}", design.modules.len());
            eprintln!("  Classes:         {}", design.classes.len());
            eprintln!("  Peak VMem:       {:.1} MB", mem_before);
            eprintln!("  Peak VMem after: {:.1} MB", mem_after);
            eprintln!("  VMem delta:      {:.1} MB", mem_after - mem_before);
            eprintln!("  RSS before:      {:.1} MB", rss_before);
            eprintln!("  RSS after:       {:.1} MB", rss_after);
            eprintln!("  RSS delta:       {:.1} MB", rss_after - rss_before);
        }
        Err(e) => {
            let elapsed = start.elapsed();
            let mem_after = peak_vmem_mb();
            eprintln!("OpenTitan compile partially failed after {:?}: {:?}", elapsed, e);
            eprintln!("Peak VMem before: {:.1} MB, after: {:.1} MB", mem_before, mem_after);
        }
    }
}

#[test]
fn bench_release_memory_stress() {
    // Memory stress test: compile 1000 modules and track memory.
    // (1000 di debug ~10s — tetap valid untuk memori per-module; 5000 > 50s
    // membuat `cargo test --lib` tidak selesai < 1 menit.)
    let src = gen_elab_design(1_000);

    let mem_before = peak_vmem_mb();
    let rss_before = current_rss_mb();

    let start = Instant::now();
    let design = compile_str(&src).unwrap();
    let elapsed = start.elapsed();

    let mem_after = peak_vmem_mb();
    let rss_after = current_rss_mb();

    let mem_per_module = (mem_after - mem_before) / 1000.0;

    eprintln!("═══ Memory Stress (1000 modules) ═══");
    eprintln!("  Modules:         {} ({} in top)", design.modules.len() + 1, design.modules.len());
    eprintln!("  Elapsed:         {:?}", elapsed);
    eprintln!("  Peak VMem:       {:.1} MB", mem_before);
    eprintln!("  Peak VMem after: {:.1} MB", mem_after);
    eprintln!("  VMem delta:      {:.1} MB", mem_after - mem_before);
    eprintln!("  KB per module:   {:.1} KB", mem_per_module * 1024.0);
    eprintln!("  RSS before:      {:.1} MB", rss_before);
    eprintln!("  RSS after:       {:.1} MB", rss_after);
    eprintln!("  RSS delta:       {:.1} MB", rss_after - rss_before);
}

#[test]
fn bench_release_compile_counter() {
    let src = include_str!("../../../../test/counter.sv");
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
