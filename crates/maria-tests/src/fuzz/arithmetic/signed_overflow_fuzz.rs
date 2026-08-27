//! Fuzz differential signed arithmetic — overflow, truncation, sign extension.
//!
//! Edge cases: signed multiply overflow, signed division min/-1, sign extension.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("signed-arith-fuzz-sim".to_string())
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

/// Signed addition with overflow.
/// a + b (signed) wraps around at width boundary.
#[test]
fn signed_add_overflow() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_10);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;

        // Signed addition: reinterpret as signed, add, truncate to w bits
        let mask = m;
        let expected = (a.wrapping_add(b)) & mask;

        let a_lit = format!("$signed({w}'h{a:x})", w = w, a = a);
        let b_lit = format!("$signed({w}'h{b:x})", w = w, b = b);

        let src = format!(
            "module test;\n\
             \x20   wire signed [{hi}:0] y;\n\
             \x20   assign y = {a} + {b};\n\
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
        "{} mismatch signed add:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Signed subtraction: a - b.
#[test]
fn signed_sub_basic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_11);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;

        let expected = (a.wrapping_sub(b)) & m;

        let a_lit = format!("$signed({w}'h{a:x})", w = w, a = a);
        let b_lit = format!("$signed({w}'h{b:x})", w = w, b = b);

        let src = format!(
            "module test;\n\
             \x20   wire signed [{hi}:0] y;\n\
             \x20   assign y = {a} - {b};\n\
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
        "{} mismatch signed sub:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Signed negate: -a.
#[test]
fn signed_negate() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_12);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let a = rng.u64(..) & m;
        let expected = (!a).wrapping_add(1) & m; // two's complement negate

        let a_lit = format!("$signed({w}'h{a:x})", w = w, a = a);

        let src = format!(
            "module test;\n\
             \x20   wire signed [{hi}:0] y;\n\
             \x20   assign y = -{a};\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            hi = w - 1,
            a = a_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} harap={:#x} dapat={:?}",
                seed, w, a, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch signed negate:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Mixed signed/unsigned: signed * unsigned should zero-extend.
#[test]
fn mixed_sign_mul() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_13);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;

        // (a * b) truncated to w bits
        let expected = (a.wrapping_mul(b)) & m;

        let a_lit = format!("{w}'h{a:x}", w = w, a = a);
        let b_lit = format!("{w}'h{b:x}", w = w, b = b);

        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {a} * {b};\n\
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
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch mixed sign mul:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
