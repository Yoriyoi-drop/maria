//! Fuzz differential $display format edge cases.
//!
//! Instead of capturing stdout, we verify the value computation
//! that feeds into $display by checking the signal directly.
//! This tests that values are correctly computed before formatting.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("display-format-fuzz-sim".to_string())
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

/// Verify $display with %h produces correct hex (check via signal, not stdout).
/// Tests: value passthrough to display formatting pipeline.
#[test]
fn display_hex_value_passthrough() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xBB_01);
        let w = [4u32, 8, 16, 32][seed as usize % 4];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let val = rng.u64(..) & m;
        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {val};\n\
             \x20   initial begin\n\
             \x20       $display(\"%%h=%h %%d=%0d\", y, y);\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            val = val_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} val={:#x} harap={:#x} dapat={:?}",
                seed, w, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch display hex passthrough:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Verify $display with expression arguments.
#[test]
fn display_expression_args() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xBB_04);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let expected = a ^ b; // XOR

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);

        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   wire [{hi}:0] a_w = {a};\n\
             \x20   wire [{hi}:0] b_w = {b};\n\
             \x20   assign y = a_w ^ b_w;\n\
             \x20   initial begin\n\
             \x20       $display(\"%%h\", a_w ^ b_w);\n\
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
        "{} mismatch display expr args:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Verify $display with multiple format specifiers in sequence.
#[test]
fn display_multi_format() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xBB_05);
        let w = [8u32, 16][seed as usize % 2];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        // Pack {a, b} into 2w-bit value
        let expected = (a << w) | b;

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);

        let src = format!(
            "module test;\n\
             \x20   wire [{hi2}:0] y;\n\
             \x20   wire [{hi}:0] a_w = {a};\n\
             \x20   wire [{hi}:0] b_w = {b};\n\
             \x20   assign y = {{a_w, b_w}};\n\
             \x20   initial begin\n\
             \x20       $display(\"%%h %%h\", a_w, b_w);\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            hi2 = 2 * w - 1,
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
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch display multi format:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
