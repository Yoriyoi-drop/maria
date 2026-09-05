//! Fuzz differential shift chains — `(a << b) >> c`, `a <<< b >>> c`,
//! `(a + b) << c`, dll.
//!
//! Blind spot: fuzzer existing menguji shift tunggal, tapi rantai shift
//! (shift-of-shift, shift hasil aritmetika) belum terekspos. Edge cases:
//! - Total shift > width (hasil = 0 atau all-ones tergantung signedness)
//! - Shift amount dari hasil operasi lain
//! - Mixed `<<<` + `>>>` (signedness berubah di tengah rantai)
//! - Shift hasil comparison (1-bit << 3)

use crate::fuzz::gen::{generate, lit_sv, mask_of};

const SHIFT_CHAIN_WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

fn mask_of128(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

fn sign_ext(p: u128, w: u32) -> i128 {
    if w > 0 && w < 128 && ((p >> (w - 1)) & 1) == 1 {
        (p as i128).wrapping_sub(1i128 << w)
    } else {
        p as i128
    }
}

/// Golden: `lhs << amount` mod 2^w (unsigned).
fn golden_shl(lhs: u128, amt: u128, w: u32) -> u128 {
    let m = mask_of128(w);
    let a = amt.min(w as u128) as u32;
    if a >= w {
        0
    } else {
        (lhs << a) & m
    }
}

/// Golden: `lhs >> amount` mod 2^w (unsigned/logical).
fn golden_shr(lhs: u128, amt: u128, w: u32) -> u128 {
    let m = mask_of128(w);
    let a = amt.min(w as u128) as u32;
    if a >= w {
        0
    } else {
        (lhs >> a) & m
    }
}

/// Golden: `lhs >>> amount` mod 2^w (arithmetic/right-fill sign).
fn golden_sshr(lhs: u128, amt: u128, w: u32, lhs_signed: bool) -> u128 {
    let m = mask_of128(w);
    let a = amt.min(w as u128) as u32;
    if !lhs_signed {
        // LHS unsigned → `>>>` = `>>` (logical, zero-fill)
        return if a >= w { 0 } else { (lhs >> a) & m };
    }
    let sv = sign_ext(lhs, w);
    if a >= w {
        if sv < 0 {
            m
        } else {
            0
        }
    } else {
        ((sv >> a) as u128) & m
    }
}

/// Build source: `assign y = {expr};`
fn chain_source(expr_sv: &str, w: u32, aval: &str, bval: &str) -> String {
    format!(
        "module shift_chain_fuzz_mod;\n\
         \x20   reg [{hi}:0] a;\n\
         \x20   reg [{hi}:0] b;\n\
         \x20   wire [{hi}:0] y;\n\
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
fn shift_chain_shl_then_shr_matches_golden() {
    // `(a << b) >> c` — shift kiri lalu kanan.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let w = SHIFT_CHAIN_WIDTHS[seed as usize % SHIFT_CHAIN_WIDTHS.len()];
        if w > 64 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x11_22_33);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;
        let c = rng.u128(..) & m;

        let shifted = golden_shl(a, b, w);
        let expected = golden_shr(shifted, c, w);

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let c_lit = format!("{}'h{:x}", w, c);
        let src = chain_source(
            &format!("(({} << {}) >> {})", a_lit, b_lit, c_lit),
            w,
            "0",
            "0",
        );

        let actual = std::thread::Builder::new()
            .name("shift-chain-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
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
            .expect("sim panic");

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} c={:#x} harap={:#x} dapat={:?}\n{}",
                seed, w, a, b, c, expected, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch shl>>shr:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}

#[test]
fn shift_chain_sshr_of_shl_matches_golden() {
    // `(a << 1) >>> b` — shift kiri lalu arithmetic shift kanan.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = SHIFT_CHAIN_WIDTHS[seed as usize % SHIFT_CHAIN_WIDTHS.len()];
        if w > 64 || w < 4 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x44_55_66);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let shift_amt = rng.u128(..) & m;

        let shifted_left = golden_shl(a, 1, w);
        // LHS of >>> adalah (a << 1) di mana a = 4'sh... (signed),
        // jadi << mengikuti signedness → hasil signed → >>> = arithmetic.
        let expected = golden_sshr(shifted_left, shift_amt, w, true);

        // Paksa LHS signed agar `>>>` = arithmetic (sign-fill),
        // bukan logical (zero-fill). LHS unsigned → `>>>` = `>>`.
        let a_lit = format!("{}'sh{:x}", w, a);
        let s_lit = format!("{}'h{:x}", w, shift_amt);
        let src = chain_source(&format!("(({} << 1) >>> {})", a_lit, s_lit), w, "0", "0");

        let actual = std::thread::Builder::new()
            .name("sshr-chain-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
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
            .expect("sim panic");

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} sh={:#x} harap={:#x} dapat={:?}\n{}",
                seed, w, a, shift_amt, expected, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch sshr-of-shl:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}

#[test]
fn shift_by_addition_matches_golden() {
    // `a << (b + c)` — shift amount dari hasil aritmetika.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let w = SHIFT_CHAIN_WIDTHS[seed as usize % SHIFT_CHAIN_WIDTHS.len()];
        if w > 32 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x77_88_99);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u32(0..w);
        let c = rng.u32(0..w);

        let total_shift = (b + c) as u128;
        let expected = golden_shl(a, total_shift, w);

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b as u128);
        let c_lit = format!("{}'h{:x}", w, c as u128);
        let src = chain_source(
            &format!("({} << ({} + {}))", a_lit, b_lit, c_lit),
            w,
            "0",
            "0",
        );

        let actual = std::thread::Builder::new()
            .name("shift-add-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
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
            .expect("sim panic");

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={} c={} harap={:#x} dapat={:?}\n{}",
                seed, w, a, b, c, expected, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch shift-by-add:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}
