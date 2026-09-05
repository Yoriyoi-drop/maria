//! Fuzz differential multidimensional packed array operations — bitwise ops,
//! shifts, and assignments on `reg [N:0][M:0] x`.
//!
//! Blind spot: fuzzer existing menguji 1D packed arrays dan part-select,
//! tapi operasi pada packed arrays multidimensional (2D) belum terekspos.

const MD_WIDTHS: [(u32, u32); 4] = [(3, 3), (3, 7), (7, 3), (7, 7)];

fn mask_of128(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("md-partsel-sim".to_string())
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
fn md_bitwise_and_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let (nw, mw) = MD_WIDTHS[seed as usize % MD_WIDTHS.len()];
        let total = nw * mw;
        if total > 64 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_11);
        let m = mask_of128(total);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;

        let expected = a & b;

        let a_lit = format!("{}'h{:x}", total, a);
        let b_lit = format!("{}'h{:x}", total, b);
        let src = format!(
            "module md_partsel_fuzz_mod;\n\
             \x20   reg [{nhi}:0][{mhi}:0] a;\n\
             \x20   reg [{nhi}:0][{mhi}:0] b;\n\
             \x20   wire [{total}:0] y;\n\
             \x20   assign y = a & b;\n\
             \x20   initial begin\n\
             \x20       a = {a_lit};\n\
             \x20       b = {b_lit};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            nhi = nw - 1,
            mhi = mw - 1,
            total = total,
            a_lit = a_lit,
            b_lit = b_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} {}x{} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, nw, mw, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch md AND:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn md_bitwise_or_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let (nw, mw) = MD_WIDTHS[seed as usize % MD_WIDTHS.len()];
        let total = nw * mw;
        if total > 64 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xBB_22);
        let m = mask_of128(total);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;

        let expected = a | b;

        let a_lit = format!("{}'h{:x}", total, a);
        let b_lit = format!("{}'h{:x}", total, b);
        let src = format!(
            "module md_partsel_fuzz_mod;\n\
             \x20   reg [{nhi}:0][{mhi}:0] a;\n\
             \x20   reg [{nhi}:0][{mhi}:0] b;\n\
             \x20   wire [{total}:0] y;\n\
             \x20   assign y = a | b;\n\
             \x20   initial begin\n\
             \x20       a = {a_lit};\n\
             \x20       b = {b_lit};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            nhi = nw - 1,
            mhi = mw - 1,
            total = total,
            a_lit = a_lit,
            b_lit = b_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} {}x{} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, nw, mw, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch md OR:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn md_bitwise_xor_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let (nw, mw) = MD_WIDTHS[seed as usize % MD_WIDTHS.len()];
        let total = nw * mw;
        if total > 64 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_33);
        let m = mask_of128(total);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;

        let expected = a ^ b;

        let a_lit = format!("{}'h{:x}", total, a);
        let b_lit = format!("{}'h{:x}", total, b);
        let src = format!(
            "module md_partsel_fuzz_mod;\n\
             \x20   reg [{nhi}:0][{mhi}:0] a;\n\
             \x20   reg [{nhi}:0][{mhi}:0] b;\n\
             \x20   wire [{total}:0] y;\n\
             \x20   assign y = a ^ b;\n\
             \x20   initial begin\n\
             \x20       a = {a_lit};\n\
             \x20       b = {b_lit};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            nhi = nw - 1,
            mhi = mw - 1,
            total = total,
            a_lit = a_lit,
            b_lit = b_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} {}x{} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, nw, mw, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch md XOR:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn md_shift_left_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let (nw, mw) = MD_WIDTHS[seed as usize % MD_WIDTHS.len()];
        let total = nw * mw;
        if total > 32 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xDD_44);
        let m = mask_of128(total);
        let a = rng.u128(..) & m;
        let shift = rng.u128(..) % total as u128;

        let expected = if shift >= total as u128 {
            0u64
        } else {
            ((a << shift) & m) as u64
        };

        let a_lit = format!("{}'h{:x}", total, a);
        let s_lit = format!("{}'h{:x}", total, shift);
        let src = format!(
            "module md_partsel_fuzz_mod;\n\
             \x20   reg [{nhi}:0][{mhi}:0] a;\n\
             \x20   wire [{total}:0] y;\n\
             \x20   assign y = a << {s_lit};\n\
             \x20   initial begin\n\
             \x20       a = {a_lit};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            nhi = nw - 1,
            mhi = mw - 1,
            total = total,
            s_lit = s_lit,
            a_lit = a_lit,
        );

        let actual = run_sim(src);

        // Mask actual ke wire width — LogicVec state bisa menyimpan
        // bit lebih dari wire width untuk packed array shift result.
        let actual_masked = actual.map(|v| v & ((1u64 << total) - 1));

        if actual_masked != Some(expected) {
            mismatch.push(format!(
                "seed={} {}x{} a={:#x} sh={} harap={:#x} dapat={:?}",
                seed, nw, mw, a, shift, expected, actual_masked
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch md shift left:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn md_bit_select_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let (nw, mw) = MD_WIDTHS[seed as usize % MD_WIDTHS.len()];
        let total = nw * mw;
        if total > 32 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE_55);
        let m = mask_of128(total);
        let a = rng.u128(..) & m;
        let bit_idx = rng.u32(0..total);

        let expected = ((a >> bit_idx) & 1) as u64;

        let a_lit = format!("{}'h{:x}", total, a);
        let src = format!(
            "module md_bit_sel_mod;\n\
             \x20   reg [{nhi}:0][{mhi}:0] a;\n\
             \x20   wire y;\n\
             \x20   assign y = a[{bit}];\n\
             \x20   initial begin\n\
             \x20       a = {a_lit};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            nhi = nw - 1,
            mhi = mw - 1,
            bit = bit_idx,
            a_lit = a_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} {}x{} a={:#x} bit={} harap={} dapat={:?}",
                seed, nw, mw, a, bit_idx, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch md bit select:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
