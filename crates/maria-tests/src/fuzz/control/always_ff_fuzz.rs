//! Fuzz differential always_ff/always_comb sensitivity edge cases.
//!
//! Tests: clock-edge detection, blocking vs non-blocking, sensitivity list
//! completeness, reset behavior.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("always-ff-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn({
            move || {
                crate::simulate_signals(&src, 50)
                    .ok()
                    .and_then(|sigs| {
                        sigs.iter()
                            .find(|(n, _)| *n == "y")
                            .map(|(_, v)| v.to_u64())
                    })
            }
        })
        .expect("spawn")
        .join()
        .expect("sim panic")
}

/// always_ff posedge: register captures value on rising clock edge.
#[test]
fn always_ff_posedge_capture() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_01);
        let w = [4u32, 8, 16, 32][seed as usize % 4];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        // Generate different input values for each clock cycle
        let v0 = rng.u64(..) & m;
        let v1 = rng.u64(..) & m;
        let v2 = rng.u64(..) & m;

        // The register should capture v2 (last assigned before $finish)
        let expected = v2;

        let v0_lit = format!("{}'h{:x}", w, v0);
        let v1_lit = format!("{}'h{:x}", w, v1);
        let v2_lit = format!("{}'h{:x}", w, v2);

        let src = format!(
            "module test;\n\
             \x20   reg clk = 0;\n\
             \x20   reg [{hi}:0] d;\n\
             \x20   reg [{hi}:0] y;\n\
             \x20   always_ff @(posedge clk) y <= d;\n\
             \x20   initial begin\n\
             \x20       d = {v0}; clk = 1; #1; // posedge 1: capture v0\n\
             \x20       d = {v1}; clk = 0; #1; // negedge\n\
             \x20       clk = 1; #1;            // posedge 2: capture v1\n\
             \x20       d = {v2}; clk = 0; #1; // negedge\n\
             \x20       clk = 1; #1;            // posedge 3: capture v2\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            v0 = v0_lit,
            v1 = v1_lit,
            v2 = v2_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} v0={:#x} v1={:#x} v2={:#x} harap={:#x} dapat={:?}",
                seed, w, v0, v1, v2, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch always_ff posedge:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// always_comb: combinational sensitivity — output tracks input changes.
#[test]
fn always_comb_tracks_input() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xBB_02);
        let w = [4u32, 8, 16, 32][seed as usize % 4];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let a_val = rng.u64(..) & m;
        let b_val = rng.u64(..) & m;
        let expected = a_val ^ b_val; // XOR

        let a_lit = format!("{}'h{:x}", w, a_val);
        let b_lit = format!("{}'h{:x}", w, b_val);

        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] a;\n\
             \x20   wire [{hi}:0] b;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   reg [{hi}:0] ra, rb;\n\
             \x20   assign a = ra;\n\
             \x20   assign b = rb;\n\
             \x20   always_comb y = a ^ b;\n\
             \x20   initial begin\n\
             \x20       ra = {a}; rb = {b}; #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            a = a_lit,
            b = b_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w, a_val, b_val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch always_comb:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Non-blocking assign (<=) vs blocking (=) ordering in always_ff.
#[test]
fn nonblocking_vs_blocking_order() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_03);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let v0 = rng.u64(..) & m;
        let v1 = rng.u64(..) & m;

        // Non-blocking: y gets v0 (captured before update to v1)
        let expected = v0;

        let v0_lit = format!("{}'h{:x}", w, v0);
        let v1_lit = format!("{}'h{:x}", w, v1);

        let src = format!(
            "module test;\n\
             \x20   reg clk = 0;\n\
             \x20   reg [{hi}:0] d;\n\
             \x20   reg [{hi}:0] y;\n\
             \x20   always_ff @(posedge clk) begin\n\
             \x20       y <= d;     // capture current d\n\
             \x20       d <= {v1};  // update d (non-blocking: takes effect next cycle)\n\
             \x20   end\n\
             \x20   initial begin\n\
             \x20       d = {v0};\n\
             \x20       clk = 1; #1; // posedge: y <= v0, d <= v1\n\
             \x20       clk = 0; #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            v0 = v0_lit,
            v1 = v1_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} v0={:#x} v1={:#x} harap={:#x} dapat={:?}",
                seed, w, v0, v1, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch non-blocking order:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Reset signal: async reset clears register.
#[test]
fn async_reset_behavior() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xDD_04);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let v0 = rng.u64(..) & m;
        // After reset, register should be 0
        let expected = 0u64;

        let v0_lit = format!("{}'h{:x}", w, v0);

        let src = format!(
            "module test;\n\
             \x20   reg clk = 0;\n\
             \x20   reg rst_n = 0;\n\
             \x20   reg [{hi}:0] d;\n\
             \x20   reg [{hi}:0] y;\n\
             \x20   always_ff @(posedge clk or negedge rst_n)\n\
             \x20       if (!rst_n) y <= 0;\n\
             \x20       else y <= d;\n\
             \x20   initial begin\n\
             \x20       d = {v0};\n\
             \x20       clk = 1; #1; // posedge but rst_n=0 → y=0\n\
             \x20       clk = 0; #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            v0 = v0_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} v0={:#x} harap={:#x} dapat={:?}",
                seed, w, v0, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch async reset:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
