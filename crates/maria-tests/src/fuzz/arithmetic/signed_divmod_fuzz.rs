//! Fuzz differential signed division and remainder.
//!
//! Tests: signed div, signed mod, negative operand behavior.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("signed-divmod-fuzz-sim".to_string())
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

/// Signed division: $signed(a) / $signed(b).
#[test]
fn signed_div_basic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF2_01);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let half = 1u64 << (w - 1);

        let a_raw = rng.u64(..) & m;
        let b_raw = rng.u64(1..=m);
        if b_raw == 0 { continue; }

        // Interpret as signed
        let a_s = if a_raw >= half { (a_raw as i64) - (1i64 << w) } else { a_raw as i64 };
        let b_s = if b_raw >= half { (b_raw as i64) - (1i64 << w) } else { b_raw as i64 };

        // Signed division result, reinterpreted as unsigned w-bit
        let res_s = a_s.wrapping_div(b_s);
        let expected = (res_s as u64) & m;

        let a_lit = format!("$signed({w}'h{a_raw:x})", w = w, a_raw = a_raw);
        let b_lit = format!("$signed({w}'h{b_raw:x})", w = w, b_raw = b_raw);

        let src = format!(
            "module test;\n\
             \x20   wire signed [{hi}:0] y;\n\
             \x20   assign y = {a} / {b};\n\
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
                seed, w, a_raw, b_raw, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch signed div:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Signed modulo: $signed(a) % $signed(b).
#[test]
fn signed_mod_basic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF2_02);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let half = 1u64 << (w - 1);

        let a_raw = rng.u64(..) & m;
        let b_raw = rng.u64(1..=m);
        if b_raw == 0 { continue; }

        let a_s = if a_raw >= half { (a_raw as i64) - (1i64 << w) } else { a_raw as i64 };
        let b_s = if b_raw >= half { (b_raw as i64) - (1i64 << w) } else { b_raw as i64 };

        let res_s = a_s.wrapping_rem(b_s);
        let expected = (res_s as u64) & m;

        let a_lit = format!("$signed({w}'h{a_raw:x})", w = w, a_raw = a_raw);
        let b_lit = format!("$signed({w}'h{b_raw:x})", w = w, b_raw = b_raw);

        let src = format!(
            "module test;\n\
             \x20   wire signed [{hi}:0] y;\n\
             \x20   assign y = {a} % {b};\n\
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
                seed, w, a_raw, b_raw, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch signed mod:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
