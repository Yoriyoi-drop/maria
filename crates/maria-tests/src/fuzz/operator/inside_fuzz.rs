//! Fuzz differential operator `inside` — set membership.
//!
//! Blind spot: tidak ada fuzzer yang menguji `inside { set }` secara
//! ekstensif. Operator ini melibatkan pencocokan LHS terhadap daftar
//! item dalam kurung kurawal, dengan ekstensi konteks untuk perbandingan.
//! Edge cases:
//! - `inside` dengan range `[lo:hi]`
//! - `inside` dengan banyak item
//! - `inside` dengan item berbeda width
//! - LHS bernilai X/Z (tidak ada yang cocok → 0)

use crate::fuzz::gen::{generate, lit_sv, mask_of};

const INSIDE_WIDTHS: [u32; 5] = [4, 8, 12, 16, 32];

fn mask_of128(w: u32) -> u128 {
    if w >= 128 { u128::MAX } else { (1u128 << w) - 1 }
}

/// Bangun source dengan `assign y = (lhs inside {items});`
fn inside_source(lhs_sv: &str, w: u32, items: &[u64], aval: &str, bval: &str) -> String {
    let items_sv: Vec<String> = items.iter().map(|v| format!("{}'h{:x}", w, v & mask_of(w))).collect();
    let set = items_sv.join(", ");
    format!(
        "module inside_fuzz_mod;\n\
         \x20   reg [{hi}:0] a;\n\
         \x20   reg [{hi}:0] b;\n\
         \x20   wire y;\n\
         \x20   assign y = ({lhs} inside {{{set}}});\n\
         \x20   initial begin\n\
         \x20       a = {aval};\n\
         \x20       b = {bval};\n\
         \x20       #10;\n\
         \x20       $finish;\n\
         \x20   end\n\
         endmodule\n",
        hi = w - 1,
        lhs = lhs_sv,
        set = set,
        aval = aval,
        bval = bval,
    )
}

/// Golden: apakah `lhs_val` ada di dalam `items`?
fn golden_inside(lhs_val: u64, w: u32, items: &[u64]) -> bool {
    let m = mask_of128(w);
    let lv = lhs_val as u128 & m;
    items.iter().any(|&item| (item as u128 & m) == lv)
}

#[test]
fn inside_literal_items_match_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    let n_seeds: u64 = std::env::var("MARIA_INSIDE_FUZZ_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(150);

    for seed in 0..n_seeds {
        let w = INSIDE_WIDTHS[seed as usize % INSIDE_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_DD_EE);
        let m = mask_of128(w);

        // Generate items: 2-5 random values + edge cases.
        let n_items = rng.usize(2..=5);
        let mut items: Vec<u64> = Vec::with_capacity(n_items + 3);
        for _ in 0..n_items {
            items.push((rng.u64(0..) & mask_of(w)) as u64);
        }
        // Edge: 0, all-ones, random
        items.push(0);
        items.push(mask_of(w));
        items.push(rng.u64(0..) & mask_of(w));

        // Test with each item as LHS (should match) and random non-matching.
        for &test_val in &items {
            let lhs_sv = format!("{}'h{:x}", w, test_val & mask_of(w));
            let src = inside_source(&lhs_sv, w, &items, "0", "0");

            let expected = golden_inside(test_val, w, &items);
            let actual = std::thread::Builder::new()
                .name("inside-sim".to_string())
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
                    "seed={} w={} val={:#x} items={:?} harap={} dapat={:?}\n{}",
                    seed, w, test_val, items.iter().map(|x| format!("{:#x}", x)).collect::<Vec<_>>(),
                    expected_bit, actual, src
                ));
            }
            checked += 1;
        }
    }
    assert!(checked > 100, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch inside:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}

#[test]
fn inside_with_signal_as_lhs() {
    // `a inside { item1, item2, ... }` — LHS dari variabel.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let input = generate(seed ^ 0xAA_BB_11);
        if input.w > 32 { continue; }
        if input.expr.eval_has_x(input.w, input.a, input.b) { continue; }

        let w = input.w;
        let sel = input.expr.eval(input.w, input.a, input.b) & mask_of(w);

        let mut rng = fastrand::Rng::with_seed(seed ^ 0x22_33_44);
        let n_items = rng.usize(2..=4);
        let items: Vec<u64> = (0..n_items)
            .map(|_| rng.u64(0..) & mask_of(w))
            .collect();

        let expected = golden_inside(sel, w, &items);
        let items_sv: Vec<String> = items.iter().map(|v| format!("{}'h{:x}", w, v)).collect();
        let expr_sv = input.expr.to_sv(w);
        let src = format!(
            "module inside_fuzz_mod;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   reg [{bhi}:0] b;\n\
             \x20   wire y;\n\
             \x20   assign y = ({expr} inside {{{set}}});\n\
             \x20   initial begin\n\
             \x20       a = {aval};\n\
             \x20       b = {bval};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            bhi = input.wb - 1,
            expr = expr_sv,
            set = items_sv.join(", "),
            aval = lit_sv(input.a, w),
            bval = lit_sv(input.b, input.wb),
        );

        let actual = std::thread::Builder::new()
            .name("inside-sig-sim".to_string())
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
                "seed={} w={} sel={:#x} harap={} dapat={:?}\n{}",
                seed, w, sel, expected_bit, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch inside signal:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}
