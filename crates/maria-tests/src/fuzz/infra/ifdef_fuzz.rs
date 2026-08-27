//! Fuzz differential conditional compilation — `ifdef, `define, `ifndef.
//!
//! Tests: ifdef/ifndef toggling, nested ifdef, define propagation.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("ifdef-fuzz-sim".to_string())
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

/// `ifdef with define present → use true branch.
#[test]
fn ifdef_defined() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA1_01);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let val = rng.u64(..) & m;
        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "`define MODE_A\n\
             module test;\n\
             \x20   wire [{hi}:0] y;\n\
             `ifdef MODE_A\n\
             \x20   assign y = {val};\n\
             `else\n\
             \x20   assign y = 0;\n\
             `endif\n\
             \x20   initial begin #1; $finish; end\n\
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
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch ifdef defined:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// `ifndef without define → use true branch.
#[test]
fn ifndef_undefined() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA1_02);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let val = rng.u64(..) & m;
        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
             `ifndef UNDEFINED_SYMBOL\n\
             \x20   assign y = {val};\n\
             `else\n\
             \x20   assign y = 0;\n\
             `endif\n\
             \x20   initial begin #1; $finish; end\n\
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
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch ifndef undefined:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// `ifdef without define → use else branch.
#[test]
fn ifdef_undefined() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA1_03);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let val = rng.u64(..) & m;
        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
             `ifdef UNDEFINED_SYMBOL\n\
             \x20   assign y = 0;\n\
             `else\n\
             \x20   assign y = {val};\n\
             `endif\n\
             \x20   initial begin #1; $finish; end\n\
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
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch ifdef undefined:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
