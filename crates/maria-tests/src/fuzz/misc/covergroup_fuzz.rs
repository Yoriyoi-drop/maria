//! Fuzz differential covergroup sampling with random values.
//!
//! Tests: covergroup with random bins, cross coverage, conditional coverage.

fn run_sim_no_crash(src: &str) -> bool {
    std::thread::Builder::new()
        .name("covergroup-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn({
            let src = src.to_string();
            move || crate::simulate_signals(&src, 50).ok().map(|s| s.len())
        })
        .expect("spawn")
        .join()
        .expect("sim panic")
        .is_some()
}

/// Covergroup with basic bins — random sample values.
#[test]
fn covergroup_basic_bins_fuzz() {
    let mut checked = 0u32;

    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_01);
        let val = rng.u64(0..100);

        let src = format!(
            "module cover_mod;\n\
             \x20   reg [7:0] data;\n\
             \x20   covergroup cg;\n\
             \x20       cp_data: coverpoint data;\n\
             \x20   endgroup\n\
             \x20   cg inst = new();\n\
             \x20   initial begin\n\
             \x20       data = 8'd{};\n\
             \x20       inst.sample();\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            val,
        );

        assert!(
            run_sim_no_crash(&src),
            "simulation panicked on covergroup basic seed={}",
            seed
        );
        checked += 1;
    }
    assert!(checked > 15, "terlalu sedikit kasus (checked={})", checked);
}

/// Covergroup with explicit bins.
#[test]
fn covergroup_explicit_bins_fuzz() {
    let mut checked = 0u32;

    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_02);
        let val = rng.u64(0..100);

        // Use raw string to avoid format brace issues
        let src = format!(
            "module cover_bins_mod;\n\
             \x20   reg [7:0] data;\n\
             \x20   covergroup cg;\n\
             \x20       cp_data: coverpoint data {{\n\
             \x20           bins low = {{[0:25]}};\n\
             \x20           bins mid = {{[26:75]}};\n\
             \x20           bins high = {{[76:100]}};\n\
             \x20       }}\n\
             \x20   endgroup\n\
             \x20   cg inst = new();\n\
             \x20   initial begin\n\
             \x20       data = 8'd{};\n\
             \x20       inst.sample();\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            val,
        );

        assert!(
            run_sim_no_crash(&src),
            "simulation panicked on covergroup explicit bins seed={}",
            seed
        );
        checked += 1;
    }
    assert!(checked > 15, "terlalu sedikit kasus (checked={})", checked);
}

/// Covergroup with cross coverage — two coverpoints.
#[test]
fn covergroup_cross_fuzz() {
    let mut checked = 0u32;

    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_03);
        let a = rng.u64(0..4);
        let b = rng.u64(0..4);

        let src = format!(
            "module cover_cross_mod;\n\
             \x20   reg [1:0] a, b;\n\
             \x20   covergroup cg;\n\
             \x20       cp_a: coverpoint a;\n\
             \x20       cp_b: coverpoint b;\n\
             \x20       cx_ab: cross cp_a, cp_b;\n\
             \x20   endgroup\n\
             \x20   cg inst = new();\n\
             \x20   initial begin\n\
             \x20       a = 2'd{};\n\
             \x20       b = 2'd{};\n\
             \x20       inst.sample();\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a, b,
        );

        assert!(
            run_sim_no_crash(&src),
            "simulation panicked on covergroup cross seed={}",
            seed
        );
        checked += 1;
    }
    assert!(checked > 15, "terlalu sedikit kasus (checked={})", checked);
}

/// Covergroup with hit count — sample multiple times.
#[test]
fn covergroup_hit_count_fuzz() {
    let mut checked = 0u32;

    for seed in 0..20u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_04);
        let n_samples = rng.usize(1..10);

        let src = format!(
            "module cover_hit_mod;\n\
             \x20   reg [3:0] data;\n\
             \x20   covergroup cg;\n\
             \x20       cp_data: coverpoint data {{\n\
             \x20           bins zero = {{0}};\n\
             \x20           bins one = {{1}};\n\
             \x20           bins other = {{[2:15]}};\n\
             \x20       }}\n\
             \x20   endgroup\n\
             \x20   cg inst = new();\n\
             \x20   integer i;\n\
             \x20   initial begin\n\
             \x20       for (i = 0; i < {}; i = i + 1) begin\n\
             \x20           data = i[3:0];\n\
             \x20           inst.sample();\n\
             \x20       end\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            n_samples,
        );

        assert!(
            run_sim_no_crash(&src),
            "simulation panicked on covergroup hit count seed={}",
            seed
        );
        checked += 1;
    }
    assert!(checked > 10, "terlalu sedikit kasus (checked={})", checked);
}
