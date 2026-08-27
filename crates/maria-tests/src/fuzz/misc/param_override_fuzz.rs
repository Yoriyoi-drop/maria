//! Fuzz differential cross-module parameter override.
//!
//! Tests: #(.PARAM(val)) override, localparam, parameter width.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("param-override-fuzz-sim".to_string())
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

/// Basic parameter override: child module uses param for width, parent overrides.
#[test]
fn param_override_basic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE_01);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let val = rng.u64(..) & m;
        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);

        let src = format!(
            "module child #(parameter W = 8) (input [W-1:0] a, output [W-1:0] y);\n\
             \x20   assign y = a;\n\
             endmodule\n\
             \n\
             module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   child #(.W({w})) inst (.a({val}), .y(y));\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            hi = w - 1,
            w = w,
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
        "{} mismatch param override basic:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Parameter used in expression: child computes a + PARAM.
#[test]
fn param_in_expression() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE_02);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let val = rng.u64(..) & m;
        let offset = rng.u64(..) & 0xF; // small offset
        let expected = val.wrapping_add(offset) & m;

        let val_lit = format!("{}'h{:x}", w, val);
        let off_lit = format!("{}'h{:x}", w, offset);

        let src = format!(
            "module child #(parameter W = 8, parameter OFFSET = 0) (input [W-1:0] a, output [W-1:0] y);\n\
             \x20   assign y = a + OFFSET;\n\
             endmodule\n\
             \n\
             module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   child #(.W({w}), .OFFSET({off})) inst (.a({val}), .y(y));\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            hi = w - 1,
            w = w,
            off = off_lit,
            val = val_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} val={:#x} off={:#x} harap={:#x} dapat={:?}",
                seed, w, val, offset, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch param in expression:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Localparam: constant computed inside module.
#[test]
fn localparam_basic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE_03);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let base = rng.u64(..) & m;
        let mask = rng.u64(..) & m;
        let expected = base & mask;

        let base_lit = format!("{}'h{:x}", w, base);
        let mask_lit = format!("{}'h{:x}", w, mask);

        let src = format!(
            "module test;\n\
             \x20   localparam [{hi}:0] BASE = {base};\n\
             \x20   localparam [{hi}:0] MASK = {mask};\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = BASE & MASK;\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            hi = w - 1,
            base = base_lit,
            mask = mask_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} base={:#x} mask={:#x} harap={:#x} dapat={:?}",
                seed, w, base, mask, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch localparam basic:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
