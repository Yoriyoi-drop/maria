//! Stress test — compile large synthetic SystemVerilog projects.
//!
//! Run with: cargo test -- --ignored stress_tests::
//! These tests validate Maria's architecture at scale.

use crate::compile_str;
use crate::frontend::compile_session::{CompileSession, SessionConfig};
use std::time::Instant;

/// Generate N simple counter modules.
fn generate_n_modules(count: usize) -> String {
    let mut source = String::with_capacity(count * 200);
    for i in 0..count {
        source.push_str(&format!(
            r#"module counter_{}(
    input clk,
    input rst_n,
    output reg [7:0] count
);
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n)
            count <= 8'h00;
        else
            count <= count + 8'h01;
    end
endmodule
"#, i));
    }
    source
}

/// Generate N modules with inter-dependencies (leaf → mid → top).
fn generate_dep_modules(count: usize) -> String {
    let mut source = String::with_capacity(count * 300);
    // Create hierarchical tree: each module instantiates 2 sub-modules
    let mut m = 0;
    while m < count {
        let c1 = if m + 1 < count { m + 1 } else { 0 };
        let c2 = if m + 2 < count { m + 2 } else { 0 };
        source.push_str(&format!(
            r#"module hier_{}(
    input clk,
    input rst_n,
    output reg [7:0] out
);
    wire [7:0] w1, w2;
    hier_{} u1 (.clk(clk), .rst_n(rst_n), .out(w1));
    hier_{} u2 (.clk(clk), .rst_n(rst_n), .out(w2));
    always_comb out = w1 + w2;
endmodule
"#, m, c1, c2));
        m += 1;
        if m >= count { break; }
    }
    source
}

#[test]
#[ignore]
fn test_stress_100_modules() {
    let source = generate_n_modules(100);
    let start = Instant::now();
    let design = compile_str(&source).unwrap();
    let elapsed = start.elapsed();
    eprintln!("Stress 100 modules: {:?} ({} modules)", elapsed, design.modules.len());
    assert_eq!(design.modules.len(), 100);
    assert!(elapsed.as_secs() < 5, "100 modules took too long: {:?}", elapsed);
}

#[test]
#[ignore]
fn test_stress_1000_modules() {
    let source = generate_n_modules(1000);
    let start = Instant::now();
    let design = compile_str(&source).unwrap();
    let elapsed = start.elapsed();
    eprintln!("Stress 1000 modules: {:?} ({} modules)", elapsed, design.modules.len());
    assert_eq!(design.modules.len(), 1000);
    assert!(elapsed.as_secs() < 30, "1000 modules took too long: {:?}", elapsed);
}

#[test]
#[ignore]
fn test_stress_symbol_table() {
    // Verify DashMap-based string table scales
    use crate::intern::Symbol;
    let start = Instant::now();
    for i in 0..5000 {
        let s = format!("sym_{}", i);
        let sym = Symbol::intern(&s);
        assert_eq!(sym.as_str(), s);
    }
    let elapsed = start.elapsed();
    eprintln!("5K symbols interned in {:?}", elapsed);
    assert!(elapsed.as_secs() < 60, "5K symbols too slow: {:?}", elapsed);
}

#[test]
#[ignore]
fn test_stress_parallel_compile_session() {
    // Generate 10 small files, compile via CompileSession
    let dir = std::env::temp_dir().join("maria_stress_session");
    let _ = std::fs::create_dir_all(&dir);

    let mut sources = Vec::new();
    for i in 0..50 {
        let path = dir.join(format!("mod_{}.sv", i));
        let content = format!(
            "module mod_{}(input clk, output reg [3:0] q);\n\
             always_ff @(posedge clk) q <= q + 4'h1;\n\
             endmodule\n", i);
        std::fs::write(&path, &content).unwrap();
        sources.push(path);
    }

    let config = SessionConfig {
        sources,
        incdirs: vec![dir.clone()],
        ..Default::default()
    };
    let mut session = CompileSession::new(config);
    let start = Instant::now();
    let (design, index) = session.compile().unwrap();
    let elapsed = start.elapsed();
    eprintln!("CompileSession 50 files: {:?} ({} modules, {} idx)", elapsed, design.modules.len(), index.len());
    assert!(design.modules.len() >= 50);
    assert!(elapsed.as_secs() < 30, "50 files took too long: {:?}", elapsed);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn test_stress_mmap_io() {
    use crate::frontend::io::MmapFile;
    let dir = std::env::temp_dir().join("maria_mmap_stress");
    let _ = std::fs::create_dir_all(&dir);

    // Create a large file (>4KB to trigger mmap)
    let path = dir.join("large.sv");
    let mut content = String::with_capacity(100_000);
    for i in 0..500 {
        content.push_str(&format!("// line {} - this is a long comment to fill up space for mmap testing\n", i));
    }
    std::fs::write(&path, &content).unwrap();

    // Read via mmap
    let start = Instant::now();
    for _ in 0..100 {
        let mf = MmapFile::open(&path).unwrap();
        assert!(mf.len() > 0);
    }
    let elapsed = start.elapsed();
    eprintln!("Mmap 100 reads of {:?} file: {:?}", path, elapsed);
    assert!(elapsed.as_secs() < 10, "mmap too slow: {:?}", elapsed);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_stress_incremental_tracker() {
    use crate::scheduler::incremental::IncrementalTracker;
    use crate::scheduler::dag::DependencyGraph;
    use crate::scheduler::work_stealing::Task;
    use std::path::Path;

    let tracker = IncrementalTracker::new();
    let graph = DependencyGraph::new();

    // Create 1000 module nodes with dependencies
    let mut nodes = Vec::new();
    for i in 0..1000 {
        let node = graph.add_node(Task::ParseFile(format!("mod_{}.sv", i)));
        nodes.push(node);
    }

    // Register with tracker
    for (i, node) in nodes.iter().enumerate() {
        let path_str = format!("/tmp/mod_{}.sv", i);
        let path = Path::new(&path_str);
        tracker.register_file(path, vec![*node], i as u64);
    }

    // Mark one file changed
    let start = Instant::now();
    tracker.mark_changed(Path::new("/tmp/mod_0.sv"));
    let elapsed = start.elapsed();
    eprintln!("Incremental mark 1 of 1000 nodes: {:?}", elapsed);
    assert!(elapsed.as_nanos() < 1_000_000_000, "mark_changed too slow: {:?}", elapsed);
}
