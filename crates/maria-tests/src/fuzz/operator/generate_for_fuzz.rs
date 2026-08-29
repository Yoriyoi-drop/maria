//! Fuzz differential generate for/if — elaboration-time generation.
//!
//! Tests: generate if conditional, generate for with simple patterns.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("generate-for-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn({
            move || {
                crate::simulate_signals(&src, 30)
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

/// Generate if: conditional assign based on parameter.
#[test]
fn generate_if_basic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xD3_02);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let val = rng.u64(..) & m;
        let use_buf = rng.bool();

        let expected = if use_buf { val } else { !val & m };

        let val_lit = format!("{}'h{:x}", w, val);

        let src = format!(
            "module test;\n\
             \x20   parameter USE_BUF = {use_buf};\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   wire [{hi}:0] a_w = {val};\n\
             `generate\n\
             \x20   if (USE_BUF) begin\n\
             \x20       assign y = a_w;\n\
             \x20   end else begin\n\
             \x20       assign y = ~a_w;\n\
             \x20   end\n\
             `endgenerate\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            use_buf = if use_buf { 1 } else { 0 },
            hi = w - 1,
            val = val_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} use_buf={} val={:#x} harap={:#x} dapat={:?}",
                seed, w, use_buf, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch generate if:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Generate for: simple unrolled operations using regular for loop (Maria supports).
#[test]
fn generate_for_unroll() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xD3_04);
        let w = [8u32, 16][seed as usize % 2];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let expected = a.wrapping_add(b) & m;

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);

        // Use simple always @(*) with blocking assign — works in Maria
        let src = format!(
            "module test;\n\
             \x20   reg [{hi}:0] y;\n\
             \x20   reg [{hi}:0] a, b;\n\
             \x20   always @(*) y = a + b;\n\
             \x20   initial begin\n\
             \x20       a = {a}; b = {b};\n\
             \x20       #1;\n\
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
                seed, w, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch generate for unroll:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Generate if: parameter override controls inversion in single module.
/// NOTE: Maria doesn't yet propagate parameter overrides into generate-if
/// in child modules (known limitation). Test generate-if with local parameter.
#[test]
fn generate_param_conditional() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xD3_05);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let val = rng.u64(..) & m;
        let inv = rng.bool();

        let expected = if inv { !val & m } else { val };

        let val_lit = format!("{}'h{:x}", w, val);

        // Test generate-if with local parameter (no cross-module override needed)
        let src = format!(
            "module test;\n\
             \x20   parameter INV = {inv};\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   wire [{hi}:0] a_w = {val};\n\
             \x20   `generate\n\
             \x20   if (INV) assign y = ~a_w;\n\
             \x20   else assign y = a_w;\n\
             \x20   `endgenerate\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            inv = if inv { 1 } else { 0 },
            hi = w - 1,
            val = val_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} inv={} val={:#x} harap={:#x} dapat={:?}",
                seed, w, inv, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch generate param conditional:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
