//! Fuzz differential chained ternary and nested conditional expressions.
//!
//! Tests: chained ternary, nested ternary with complex conditions.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("chained-ternary-fuzz-sim".to_string())
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

/// Chained ternary: a > 10 ? 1 : a > 5 ? 2 : a > 0 ? 3 : 0
#[test]
fn chained_ternary_basic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xC3_01);
        let a = rng.u64(0..20);
        let expected = if a > 10 { 1u64 } else if a > 5 { 2 } else if a > 0 { 3 } else { 0 };

        let src = format!(
            "module test;\n\
             \x20   reg [4:0] a;\n\
             \x20   reg [1:0] y;\n\
             \x20   initial begin\n\
             \x20       a = {a};\n\
             \x20       y = a > 10 ? 2'd1 : a > 5 ? 2'd2 : a > 0 ? 2'd3 : 2'd0;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a = a,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} harap={} dapat={:?}",
                seed, a, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch chained ternary:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Nested ternary with arithmetic in branches.
#[test]
fn nested_ternary_arithmetic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xC3_02);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let c = rng.u64(..) & m;

        // y = a > b ? (a - b) : (b - a) = |a - b|
        let expected = if a > b { a.wrapping_sub(b) & m } else { b.wrapping_sub(a) & m };

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);

        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   wire [{hi}:0] a_w = {a};\n\
             \x20   wire [{hi}:0] b_w = {b};\n\
             \x20   assign y = a_w > b_w ? a_w - b_w : b_w - a_w;\n\
             \x20   initial begin #1; $finish; end\n\
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
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch nested ternary arithmetic:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Ternary with bitwise operations.
#[test]
fn ternary_bitwise() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xC3_03);
        let w = [8u32, 16][seed as usize % 2];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let sel = rng.u64(0..4);
        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;

        // y = sel[1] ? (sel[0] ? (a & b) : (a | b)) : (sel[0] ? (a ^ b) : ~(a | b))
        let expected = if sel & 2 != 0 {
            if sel & 1 != 0 { a & b } else { a | b }
        } else {
            if sel & 1 != 0 { a ^ b } else { !(a | b) & m }
        };

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);

        let src = format!(
            "module test;\n\
             \x20   reg [1:0] sel;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   wire [{hi}:0] a_w = {a};\n\
             \x20   wire [{hi}:0] b_w = {b};\n\
             \x20   assign y = sel[1] ? (sel[0] ? (a_w & b_w) : (a_w | b_w))\n\
             \x20                  : (sel[0] ? (a_w ^ b_w) : ~(a_w | b_w));\n\
             \x20   initial begin\n\
             \x20       sel = {sel};\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            a = a_lit,
            b = b_lit,
            sel = sel,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} sel={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w, sel, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch ternary bitwise:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
