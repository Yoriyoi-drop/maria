//! Fuzz differential packed array assignments — blocking assign, non-blocking
//! assign (NBA), dan part-select assignments.
//!
//! Blind spot: fuzzer existing menguji expression evaluation, tapi assignment
//! ke sliced packed array (`a[3:0] = val`) belum terekspos secara systematic.
//! Edge cases:
//! - Assignment ke keseluruhan packed array
//! - Assignment ke part-select range
//! - Non-blocking assign ke packed array
//! - Assignment truncation (value wider than target)
//! - chained assignments

const PA_WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

fn mask_of128(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("packed-array-sim".to_string())
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

#[test]
fn pa_blocking_assign_whole_matches_golden() {
    // `reg [W:0] a; a = val; assign y = a;` — full packed assign.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let w = PA_WIDTHS[seed as usize % PA_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x11_AA);
        let m = mask_of128(w);
        let val = rng.u128(..) & m;

        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module pa_assign_mod;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = a;\n\
             \x20   initial begin\n\
             \x20       a = {val};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            val = val_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} val={:#x} harap={:#x} dapat={:?}",
                seed, w, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch pa whole assign:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn pa_blocking_assign_part_select_matches_golden() {
    // `a[hi:lo] = val` — part-select assignment.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let w = PA_WIDTHS[seed as usize % PA_WIDTHS.len()];
        if w < 8 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x22_BB);
        let m = mask_of128(w);

        // Pick random part-select range
        let lo = rng.u32(0..w / 2);
        let sw = rng.u32(1..=(w / 2).min(8));
        let hi = (lo + sw - 1).min(w - 1);
        let pw = hi - lo + 1;

        let a_init = rng.u128(..) & m;
        let val = rng.u128(..) & mask_of128(pw);

        // Expected: clear bits [hi:lo], set to val
        let clear_mask = !(mask_of128(pw) << lo);
        let expected = (a_init & clear_mask) | (val << lo);

        let a_init_lit = format!("{}'h{:x}", w, a_init);
        let val_lit = format!("{}'h{:x}", pw, val);
        let src = format!(
            "module pa_partsel_assign_mod;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = a;\n\
             \x20   initial begin\n\
             \x20       a = {a_init};\n\
             \x20       a[{phi}:{plo}] = {val};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            a_init = a_init_lit,
            phi = hi,
            plo = lo,
            val = val_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} [{}:{}] a={:#x} val={:#x} harap={:#x} dapat={:?}",
                seed, w, hi, lo, a_init, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch pa partsel assign:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn pa_nba_assign_matches_golden() {
    // Non-blocking assign (`<=`) ke packed array.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let w = PA_WIDTHS[seed as usize % PA_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x33_CC);
        let m = mask_of128(w);
        let a_val = rng.u128(..) & m;
        let b_val = rng.u128(..) & m;

        // a <= b; y = a;
        // After NBA: a should be b's value
        let expected = b_val;

        let a_lit = format!("{}'h{:x}", w, a_val);
        let b_lit = format!("{}'h{:x}", w, b_val);
        let src = format!(
            "module pa_nba_mod;\n\
             \x20   reg [{hi}:0] a, b;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = a;\n\
             \x20   initial begin\n\
             \x20       a = {a_val};\n\
             \x20       b = {b_val};\n\
             \x20       a <= b;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            a_val = a_lit,
            b_val = b_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w, a_val, b_val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch pa NBA:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn pa_truncation_matches_golden() {
    // Assign value wider than target → truncation.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let w = PA_WIDTHS[seed as usize % PA_WIDTHS.len()];
        if w < 8 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x44_DD);
        let m = mask_of128(w);
        // Generate value larger than w bits
        let large_val = rng.u128(..);
        let expected = large_val & m;

        let val_lit = format!("{}'h{:x}", w + 8, large_val);
        let src = format!(
            "module pa_trunc_mod;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = a;\n\
             \x20   initial begin\n\
             \x20       a = {val};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            val = val_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} val={:#x} harap={:#x} dapat={:?}",
                seed, w, large_val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch pa truncation:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn pa_sequence_assign_matches_golden() {
    // Sequential blocking assign: a = val1; a = val2; → a = val2
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let w = PA_WIDTHS[seed as usize % PA_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x55_EE);
        let m = mask_of128(w);
        let val1 = rng.u128(..) & m;
        let val2 = rng.u128(..) & m;

        // a = val1; a = val2; y = a → y = val2
        let expected = val2;

        let v1_lit = format!("{}'h{:x}", w, val1);
        let v2_lit = format!("{}'h{:x}", w, val2);
        let src = format!(
            "module pa_seq_mod;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = a;\n\
             \x20   initial begin\n\
             \x20       a = {v1};\n\
             \x20       a = {v2};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            v1 = v1_lit,
            v2 = v2_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} v1={:#x} v2={:#x} harap={:#x} dapat={:?}",
                seed, w, val1, val2, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch pa sequence:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
