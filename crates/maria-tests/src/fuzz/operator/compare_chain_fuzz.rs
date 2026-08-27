//! Fuzz differential compare chains — `(a < b) && (b < c)`, `a <= b || b >= c`,
//! `(a == b) ^ (b != c)`, dll.
//!
//! Blind spot: fuzzer existing menguji comparison tunggal, tapi rantai
//! perbandingan dengan operator logis belum terekspos secara mendalam.
//! Edge cases:
//! - Range check: `lo <= a && a <= hi`
//! - Boundary: compare tepat di batas width (0, MAX, MSB)
//! - Mixed signedness dalam rantai
//! - Result comparison digunakan sebagai operand bitwise

use crate::fuzz::gen::{generate, lit_sv, mask_of};

const CMP_CHAIN_WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

fn mask_of128(w: u32) -> u128 {
    if w >= 128 { u128::MAX } else { (1u128 << w) - 1 }
}

fn sign_ext(p: u128, w: u32) -> i128 {
    if w > 0 && w < 128 && ((p >> (w - 1)) & 1) == 1 {
        (p as i128).wrapping_sub(1i128 << w)
    } else {
        p as i128
    }
}

/// Range check: `lo <= x && x <= hi`.
fn golden_range(x: u128, lo: u128, hi: u128, w: u32) -> bool {
    let m = mask_of128(w);
    (x & m) >= (lo & m) && (x & m) <= (hi & m)
}

/// Build source with a complex comparison expression.
fn cmp_chain_source(expr_sv: &str, w: u32, aval: &str, bval: &str) -> String {
    format!(
        "module compare_chain_fuzz_mod;\n\
         \x20   reg [{hi}:0] a;\n\
         \x20   reg [{hi}:0] b;\n\
         \x20   wire y;\n\
         \x20   assign y = {expr};\n\
         \x20   initial begin\n\
         \x20       a = {aval};\n\
         \x20       b = {bval};\n\
         \x20       #10;\n\
         \x20       $finish;\n\
         \x20   end\n\
         endmodule\n",
        hi = w - 1,
        expr = expr_sv,
        aval = aval,
        bval = bval,
    )
}

#[test]
fn range_check_matches_golden() {
    // `lo <= a && a <= hi` — range check.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = CMP_CHAIN_WIDTHS[seed as usize % CMP_CHAIN_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_11_BB);
        let m = mask_of128(w);

        let a_val = rng.u128(..) & m;
        let lo = rng.u128(..) & m;
        let hi = rng.u128(..) & m;
        // Ensure lo <= hi for meaningful range check.
        let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };

        let expected = golden_range(a_val, lo, hi, w);

        let a_lit = format!("{}'h{:x}", w, a_val);
        let lo_lit = format!("{}'h{:x}", w, lo);
        let hi_lit = format!("{}'h{:x}", w, hi);
        let src = cmp_chain_source(
            &format!("(({} <= {}) && ({} <= {}))", lo_lit, a_lit, a_lit, hi_lit),
            w, "0", "0",
        );

        let actual = std::thread::Builder::new()
            .name("range-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
                move || {
                    crate::simulate_signals(&src, 30)
                        .ok()
                        .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "y").map(|(_, v)| v.to_u64()))
                }
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        let expected_bit = if expected { 1u64 } else { 0 };
        if actual != Some(expected_bit) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} lo={:#x} hi={:#x} harap={} dapat={:?}\n{}",
                seed, w, a_val, lo, hi, expected_bit, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch range check:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}

#[test]
fn chained_logical_ops_matches_golden() {
    // `(a == b) && (a != 0)` — combined comparisons.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = CMP_CHAIN_WIDTHS[seed as usize % CMP_CHAIN_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_DD_11);
        let m = mask_of128(w);

        let a_val = rng.u128(..) & m;
        let b_val = rng.u128(..) & m;

        // `(a == b) && (a != 0)`
        let expected = (a_val == b_val) && (a_val != 0);

        let a_lit = format!("{}'h{:x}", w, a_val);
        let b_lit = format!("{}'h{:x}", w, b_val);
        let src = cmp_chain_source(
            &format!("(({} == {}) && ({} != 0))", a_lit, b_lit, a_lit),
            w, "0", "0",
        );

        let actual = std::thread::Builder::new()
            .name("chain-cmp-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
                move || {
                    crate::simulate_signals(&src, 30)
                        .ok()
                        .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "y").map(|(_, v)| v.to_u64()))
                }
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        let expected_bit = if expected { 1u64 } else { 0 };
        if actual != Some(expected_bit) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={} dapat={:?}\n{}",
                seed, w, a_val, b_val, expected_bit, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch chained cmp:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}

#[test]
fn xor_of_comparisons_matches_golden() {
    // `(a < b) ^ (a > b)` — XOR of comparisons (should be same as a != b for unsigned).
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let w = CMP_CHAIN_WIDTHS[seed as usize % CMP_CHAIN_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE_FF_22);
        let m = mask_of128(w);

        let a_val = rng.u128(..) & m;
        let b_val = rng.u128(..) & m;

        let expected = ((a_val < b_val) ^ (a_val > b_val)) as u64;

        let a_lit = format!("{}'h{:x}", w, a_val);
        let b_lit = format!("{}'h{:x}", w, b_val);
        let src = cmp_chain_source(
            &format!("(({} < {}) ^ ({} > {}))", a_lit, b_lit, a_lit, b_lit),
            w, "0", "0",
        );

        let actual = std::thread::Builder::new()
            .name("xor-cmp-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
                move || {
                    crate::simulate_signals(&src, 30)
                        .ok()
                        .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "y").map(|(_, v)| v.to_u64()))
                }
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={} dapat={:?}\n{}",
                seed, w, a_val, b_val, expected, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch xor cmp:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}
