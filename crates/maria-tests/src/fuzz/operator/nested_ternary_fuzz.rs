//! Fuzz differential nested ternary — `a ? b ? c : d : e`, chained ternary
//! `a ? b : c ? d : e`, dan mixed width ternary.
//!
//! Blind spot: fuzzer existing (ternary_fuzz) menguji ternary tunggal, tapi
//! nesting dan chaining belum terekspos. Edge cases:
//! - Ternary di condition: `(a ? b : c) ? d : e`
//! - Ternary di true branch: `a ? (b ? c : d) : e`
//! - Ternary di false branch: `a ? b : (c ? d : e)`
//! - Triple nesting: `a ? b ? c ? d : e : f : g`

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("nested-ternary-sim".to_string())
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

fn mask_of128(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

const TERNARY_WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

#[test]
fn nested_ternary_in_condition_matches_golden() {
    // `(a ? b : c) ? 1 : 0` — ternary sebagai condition.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = TERNARY_WIDTHS[seed as usize % TERNARY_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_01);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;
        let c = rng.u128(..) & m;

        let cond_val = if a != 0 { b } else { c };
        let expected = if cond_val != 0 { 1u128 } else { 0u128 };

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let c_lit = format!("{}'h{:x}", w, c);
        let expr = format!("({a} ? {b} : {c}) ? {}'h1 : {}'h0", w, w);

        let full_src = format!(
            "module t;\n\
             \x20   reg [{hi}:0] a, b, c;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {expr};\n\
             \x20   initial begin\n\
             \x20       a = {a};\n\
             \x20       b = {b};\n\
             \x20       c = {c};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            expr = expr,
            a = a_lit,
            b = b_lit,
            c = c_lit,
        );

        let actual = run_sim(full_src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} c={:#x} harap={:#x} dapat={:?}",
                seed, w, a, b, c, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch nested ternary (condition):\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn nested_ternary_in_true_branch_matches_golden() {
    // `a ? (b ? 2 : 3) : 1` — ternary di true branch.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = TERNARY_WIDTHS[seed as usize % TERNARY_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xDD_02);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;

        let inner_val = if b != 0 { 2u128 } else { 3u128 };
        let expected = if a != 0 { inner_val } else { 1u128 };

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let expr = format!("{a} ? ({b} ? {w}'h2 : {w}'h3) : {w}'h1");

        let full_src = format!(
            "module t;\n\
             \x20   reg [{hi}:0] a, b;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {expr};\n\
             \x20   initial begin\n\
             \x20       a = {a};\n\
             \x20       b = {b};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            expr = expr,
            a = a_lit,
            b = b_lit,
        );

        let actual = run_sim(full_src);

        if actual != Some(expected as u64) {
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
        "{} mismatch nested ternary (true branch):\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn nested_ternary_in_false_branch_matches_golden() {
    // `a ? 1 : (b ? 2 : 3)` — ternary di false branch.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = TERNARY_WIDTHS[seed as usize % TERNARY_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE_03);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;

        let inner_val = if b != 0 { 2u128 } else { 3u128 };
        let expected = if a != 0 { 1u128 } else { inner_val };

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let expr = format!("{a} ? {w}'h1 : ({b} ? {w}'h2 : {w}'h3)");

        let full_src = format!(
            "module t;\n\
             \x20   reg [{hi}:0] a, b;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {expr};\n\
             \x20   initial begin\n\
             \x20       a = {a};\n\
             \x20       b = {b};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            expr = expr,
            a = a_lit,
            b = b_lit,
        );

        let actual = run_sim(full_src);

        if actual != Some(expected as u64) {
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
        "{} mismatch nested ternary (false branch):\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn chained_ternary_matches_golden() {
    // `a ? b : c ? d : e` — chained ternary (mengurutkan dari kiri).
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = TERNARY_WIDTHS[seed as usize % TERNARY_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_04);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;
        let c = rng.u128(..) & m;
        let d = rng.u128(..) & m;
        let e = rng.u128(..) & m;

        // `a ? b : (c ? d : e)`
        let expected = if a != 0 {
            b
        } else if c != 0 {
            d
        } else {
            e
        };

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let c_lit = format!("{}'h{:x}", w, c);
        let d_lit = format!("{}'h{:x}", w, d);
        let e_lit = format!("{}'h{:x}", w, e);
        let expr = format!("{a} ? {b} : ({c} ? {d} : {e})");

        let full_src = format!(
            "module t;\n\
             \x20   reg [{hi}:0] a, b, c, d, e;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {expr};\n\
             \x20   initial begin\n\
             \x20       a = {a};\n\
             \x20       b = {b};\n\
             \x20       c = {c};\n\
             \x20       d = {d};\n\
             \x20       e = {e};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            expr = expr,
            a = a_lit,
            b = b_lit,
            c = c_lit,
            d = d_lit,
            e = e_lit,
        );

        let actual = run_sim(full_src);

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
        "{} mismatch chained ternary:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn triple_nested_ternary_matches_golden() {
    // `a ? (b ? (c ? 3 : 4) : 2) : 1` — triple nesting.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = TERNARY_WIDTHS[seed as usize % TERNARY_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_05);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;
        let c = rng.u128(..) & m;

        let expected = if a == 0 {
            1u128
        } else if b == 0 {
            2
        } else if c == 0 {
            4 // c ? 3 : 4 → c==0 => 4
        } else {
            3 // c ? 3 : 4 → c!=0 => 3
        };

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let c_lit = format!("{}'h{:x}", w, c);
        let expr = format!("{a} ? ({b} ? ({c} ? {w}'h3 : {w}'h4) : {w}'h2) : {w}'h1");

        let full_src = format!(
            "module t;\n\
             \x20   reg [{hi}:0] a, b, c;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {expr};\n\
             \x20   initial begin\n\
             \x20       a = {a};\n\
             \x20       b = {b};\n\
             \x20       c = {c};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            expr = expr,
            a = a_lit,
            b = b_lit,
            c = c_lit,
        );

        let actual = run_sim(full_src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} c={:#x} harap={} dapat={:?}",
                seed, w, a, b, c, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch triple nested ternary:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
