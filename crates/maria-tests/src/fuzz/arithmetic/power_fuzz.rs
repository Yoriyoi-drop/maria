//! Fuzz differential operator `**` (eksponensiasi) — edge cases & boundary.
//!
//! Blind spot: fuzzer existing menghasilkan `**` jarang, dan emas u128
//! sudah diperbaiki (dulu `y >= 64 → 0` — salah vs Icarus). Test ini
//! mengeksplorasi boundary yang sering memicu bug:
//! - `0**0 = 1` (SV LRM §11.4.12, tapi banyak simulator salah)
//! - `0**N = 0` untuk N > 0
//! - `1**N = 1`
//! - `base**0 = 1`
//! - `base**1 = base`
//! - `MAX**2` (overflow modular)
//! - Exponen > lebar (self-determined UNSIGNED, LRM §11.4.10)
//! - Lebar >64 bit (jalur u128 power)

use crate::fuzz::gen::{generate, lit_sv, mask_of, WIDTH_CHOICES};

/// Lebar yang diuji — boundary u64 dan multi-word.
const POWER_WIDTHS: [u32; 8] = [2, 4, 8, 16, 31, 32, 63, 64];

fn mask_of128(w: u32) -> u128 {
    if w >= 128 { u128::MAX } else { (1u128 << w) - 1 }
}

/// Golden: square-and-multiply modular power (u128).
fn golden_power(base: u128, exp: u128, w: u32) -> u128 {
    let m = mask_of128(w);
    let b = base & m;
    let mut acc: u128 = 1;
    let mut base = b;
    let mut e = exp;
    while e > 0 {
        if e & 1 == 1 {
            acc = acc.wrapping_mul(base) & m;
        }
        e >>= 1;
        if e > 0 {
            base = base.wrapping_mul(base) & m;
        }
    }
    acc & m
}

/// Stimulus boundary — termasuk 0, 1, all-ones, MSB-set.
fn pick_val(w: u32, rng: &mut fastrand::Rng) -> u128 {
    let m = mask_of128(w);
    if rng.usize(0..10) < 3 {
        match rng.usize(0..6) {
            0 => 0,
            1 => 1,
            2 => m,
            3 => if w > 0 { 1u128 << (w - 1) } else { 0 }, // MSB set
            4 => m >> 1, // half
            _ => rng.u128(..) & m,
        }
    } else {
        rng.u128(..) & m
    }
}

#[test]
fn power_special_cases_match_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    let n_seeds: u64 = std::env::var("MARIA_POWER_FUZZ_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);

    for seed in 0..n_seeds {
        let w = POWER_WIDTHS[seed as usize % POWER_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAB_CD_E);

        let base = pick_val(w, &mut rng);
        // Exponen: ~20% special cases (0, 1, w-1, w, w+1), 80% acak.
        let exp = if rng.usize(0..10) < 2 {
            match rng.usize(0..5) {
                0 => 0,
                1 => 1,
                2 => (w as u128).saturating_sub(1),
                3 => w as u128,
                _ => (w as u128).saturating_add(1),
            }
        } else {
            pick_val(w, &mut rng)
        };

        let expected = golden_power(base, exp, w);

        // Bangun source: `assign y = (base_lit ** exp_lit);`
        let base_lit = format!("{}'h{:x}", w, base & mask_of128(w));
        let exp_lit = format!("{}'h{:x}", w, exp & mask_of128(w));
        let src = format!(
            "module power_fuzz_mod;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   reg [{hi}:0] b;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = ({base} ** {exp});\n\
             \x20   initial begin\n\
             \x20       a = 0;\n\
             \x20       b = 0;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            base = base_lit,
            exp = exp_lit,
        );

        let actual = std::thread::Builder::new()
            .name("power-fuzz-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
                move || {
                    crate::simulate_signals(&src, 30)
                        .ok()
                        .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "y").map(|(_, v)| v.to_u64()))
                }
            })
            .expect("spawn power-fuzz-sim")
            .join()
            .expect("sim panic");

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} base={} exp={} harap={:#x} dapat={:?}\n{}",
                seed, w, base, exp, expected, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 60, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch power:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}

#[test]
fn power_with_signal_operands_matches_golden() {
    // Power dengan base/exp dari variabel a/b — melatih jalur elaborator
    // resolve width + engine evaluasi power.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let input = generate(seed ^ 0x54_32_1);
        if input.w < 2 || input.w > 64 {
            continue;
        }
        if input.expr.eval_has_x(input.w, input.a, input.b) {
            continue;
        }
        let w = input.w;
        let mask = mask_of(w);

        // `a ** b` — golden menghitung langsung dari a, b.
        let base_val = input.a as u128 & mask_of128(w);
        let exp_val = input.b as u128 & mask_of128(w);
        let expected = golden_power(base_val, exp_val, w);

        let a_lit = lit_sv(input.a, w);
        let b_lit = lit_sv(input.b, input.wb);
        let src = format!(
            "module power_fuzz_mod;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   reg [{bhi}:0] b;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = (a ** b);\n\
             \x20   initial begin\n\
             \x20       a = {a};\n\
             \x20       b = {b};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            bhi = input.wb - 1,
            a = a_lit,
            b = b_lit,
        );

        let actual = std::thread::Builder::new()
            .name("power-sig-sim".to_string())
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

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={:#x} dapat={:?}\n{}",
                seed, w, input.a, input.b, expected, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch power signal:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}

#[test]
fn power_chained_matches_golden() {
    // `a ** b ** 2` — chain power, right-associative.
    // Model emas: golden_power(a, golden_power(b, 2, w), w)
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let w = POWER_WIDTHS[seed as usize % POWER_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x99_77_55);
        let base = pick_val(w, &mut rng);
        let mid_exp = pick_val(w, &mut rng);

        let final_exp = golden_power(mid_exp, 2, w);
        let expected = golden_power(base, final_exp, w);

        let base_lit = format!("{}'h{:x}", w, base & mask_of128(w));
        let mid_lit = format!("{}'h{:x}", w, mid_exp & mask_of128(w));
        let src = format!(
            "module power_fuzz_mod;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   reg [{hi}:0] b;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = ({base} ** ({mid} ** 2'h2));\n\
             \x20   initial begin\n\
             \x20       a = 0;\n\
             \x20       b = 0;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            base = base_lit,
            mid = mid_lit,
        );

        let actual = std::thread::Builder::new()
            .name("power-chain-sim".to_string())
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

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} harap={:#x} dapat={:?}\n{}",
                seed, w, expected, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch power chain:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}
