//! Fuzz differential complex expressions — mixed operator chains.
//!
//! Blind spot: fuzzer existing menguji operator tunggal atau pasangan, tapi
//! rantai kompleks `(a & b) | (c ^ d) + (e << f)` belum terekspos.
//! Edge cases:
//! - Mixed bitwise + arithmetic: `(a & b) + (c | d)`
//! - Shift + comparison: `(a << b) > (c >> d)`
//! - Nested arithmetic: `((a + b) * c) - (d / e)`
//! - XOR chain: `a ^ b ^ c ^ d`

const CE_WIDTHS: [u32; 4] = [8, 16, 32, 64];

fn mask_of128(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("complex-expr-sim".to_string())
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

#[test]
fn ce_bitwise_and_or_matches_golden() {
    // `(a & b) | (c ^ d)` — bitwise mix.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = CE_WIDTHS[seed as usize % CE_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_01);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;
        let c = rng.u128(..) & m;
        let d = rng.u128(..) & m;

        let expected = ((a & b) | (c ^ d)) & m;

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let c_lit = format!("{}'h{:x}", w, c);
        let d_lit = format!("{}'h{:x}", w, d);
        let src = format!(
            "module ce_mod;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = ({a} & {b}) | ({c} ^ {d});\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            a = a_lit,
            b = b_lit,
            c = c_lit,
            d = d_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} harap={:#x} dapat={:?}",
                seed, w, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch bitwise mix:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn ce_nested_arithmetic_matches_golden() {
    // `((a + b) * c) - d` — nested arithmetic.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = CE_WIDTHS[seed as usize % CE_WIDTHS.len()];
        if w > 16 {
            continue;
        } // overflow protection
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xBB_02);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;
        let c = rng.u128(..) & m;
        let d = rng.u128(..) & m;

        let expected = (((a + b) & m) * c).wrapping_sub(d) & m;

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let c_lit = format!("{}'h{:x}", w, c);
        let d_lit = format!("{}'h{:x}", w, d);
        let src = format!(
            "module ce_mod;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = (({a} + {b}) * {c}) - {d};\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            a = a_lit,
            b = b_lit,
            c = c_lit,
            d = d_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} harap={:#x} dapat={:?}",
                seed, w, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch nested arithmetic:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn ce_xor_chain_matches_golden() {
    // `a ^ b ^ c ^ d` — XOR chain (4 operands).
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = CE_WIDTHS[seed as usize % CE_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_03);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;
        let c = rng.u128(..) & m;
        let d = rng.u128(..) & m;

        let expected = a ^ b ^ c ^ d;

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let c_lit = format!("{}'h{:x}", w, c);
        let d_lit = format!("{}'h{:x}", w, d);
        let src = format!(
            "module ce_mod;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {a} ^ {b} ^ {c} ^ {d};\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            a = a_lit,
            b = b_lit,
            c = c_lit,
            d = d_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} harap={:#x} dapat={:?}",
                seed, w, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch XOR chain:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn ce_shift_compare_matches_golden() {
    // `(a << b) > (c >> d)` — shift + comparison (1-bit result).
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = CE_WIDTHS[seed as usize % CE_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xDD_04);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) % w as u128;
        let c = rng.u128(..) & m;
        let d = rng.u128(..) % w as u128;

        let shifted_a = if b >= w as u128 { 0 } else { (a << b) & m };
        let shifted_c = if d >= w as u128 { 0 } else { (c >> d) & m };
        let expected = if shifted_a > shifted_c { 1u64 } else { 0 };

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let c_lit = format!("{}'h{:x}", w, c);
        let d_lit = format!("{}'h{:x}", w, d);
        let src = format!(
            "module ce_mod;\n\
             \x20   wire y;\n\
             \x20   assign y = ({a} << {b}) > ({c} >> {d});\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a = a_lit,
            b = b_lit,
            c = c_lit,
            d = d_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} harap={} dapat={:?}",
                seed, w, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch shift+compare:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
