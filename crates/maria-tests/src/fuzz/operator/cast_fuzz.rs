//! Fuzz differential type casting edge cases.
//!
//! Tests: $signed, $unsigned, width casting, concatenation width.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("cast-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn({
            move || {
                crate::simulate_signals(&src, 30).ok().and_then(|sigs| {
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

/// $unsigned cast on signed value: preserves bit pattern.
#[test]
fn unsigned_cast_preserves_bits() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xC1_01);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let val = rng.u64(..) & m;
        let expected = val;

        let val_lit = format!("$signed({w}'h{val:x})", w = w, val = val);
        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = $unsigned({val});\n\
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
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch unsigned cast:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Concatenation width: {a, b} = (a << w(b)) | b.
#[test]
fn concat_width_basic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xC1_02);
        let w1 = [4u32, 8][seed as usize % 2];
        let w2 = [4u32, 8][seed as usize % 2];
        let m1 = if w1 >= 64 { u64::MAX } else { (1u64 << w1) - 1 };
        let m2 = if w2 >= 64 { u64::MAX } else { (1u64 << w2) - 1 };

        let a = rng.u64(..) & m1;
        let b = rng.u64(..) & m2;
        let expected = (a << w2) | b;

        let a_lit = format!("{}'h{:x}", w1, a);
        let b_lit = format!("{}'h{:x}", w2, b);

        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {{{{{a}, {b}}}}};\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            hi = w1 + w2 - 1,
            a = a_lit,
            b = b_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w1={} w2={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w1, w2, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch concat width:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Zero extension: assign wider wire from narrower reg.
#[test]
fn zero_extension_basic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xC1_03);
        let w窄 = [4u32, 8][seed as usize % 2];
        let w宽 = w窄 * 2;
        let m窄 = if w窄 >= 64 {
            u64::MAX
        } else {
            (1u64 << w窄) - 1
        };

        let val = rng.u64(..) & m窄;
        let expected = val; // zero-extended

        let val_lit = format!("{}'h{:x}", w窄, val);
        let src = format!(
            "module test;\n\
             \x20   reg [{w窄h}:0] a;\n\
             \x20   wire [{w宽h}:0] y;\n\
             \x20   assign y = a;\n\
             \x20   initial begin\n\
             \x20       a = {val};\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            w窄h = w窄 - 1,
            w宽h = w宽 - 1,
            val = val_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w窄={} w宽={} val={:#x} harap={:#x} dapat={:?}",
                seed, w窄, w宽, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch zero extension:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
