//! Fuzz differential assignment operators — `+=`, `-=`, `*=`, `/=`, `%=`,
//! `<<=`, `>>=`.
//!
//! Blind spot: fuzzer existing menguji continuous assign (`=`) dan blocking
//! assign, tapi compound assignment operators (`+=`, dll.) belum terekspos.
//! Edge cases:
//! - Signed vs unsigned operand
//! - Width truncation pada `+=` (overflow)
//! - Shift assign dengan amount >= width
//! - chained compound assigns

const ASSIGN_OP_WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

fn mask_of128(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

/// Build source for compound assignment test.
fn assign_op_source(op: &str, w: u32, a_val: &str, b_val: &str) -> String {
    format!(
        "module assign_op_fuzz_mod;\n\
         \x20   reg [{hi}:0] a;\n\
         \x20   reg [{hi}:0] b;\n\
         \x20   wire [{hi}:0] y;\n\
         \x20   assign y = a;\n\
         \x20   initial begin\n\
         \x20       a = {aval};\n\
         \x20       b = {bval};\n\
         \x20       a {op}= b;\n\
         \x20       #10;\n\
         \x20       $finish;\n\
         \x20   end\n\
         endmodule\n",
        hi = w - 1,
        aval = a_val,
        bval = b_val,
        op = op,
    )
}

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("assign-op-sim".to_string())
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
        .expect("sim panic")
}

#[test]
fn assign_op_add_matches_golden() {
    // `a += b` — penjumlahan compound assign.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let w = ASSIGN_OP_WIDTHS[seed as usize % ASSIGN_OP_WIDTHS.len()];
        if w > 32 {
            continue;
        } // limit i64 overflow
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_01);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;

        let expected = (a + b) & m;

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let src = assign_op_source("+", w, &a_lit, &b_lit);

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch +=:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn assign_op_sub_matches_golden() {
    // `a -= b` — pengurangan compound assign.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let w = ASSIGN_OP_WIDTHS[seed as usize % ASSIGN_OP_WIDTHS.len()];
        if w > 32 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xBB_02);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;

        let expected = (a.wrapping_sub(b)) & m;

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let src = assign_op_source("-", w, &a_lit, &b_lit);

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch -=:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn assign_op_mul_matches_golden() {
    // `a *= b` — perkalian compound assign.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let w = ASSIGN_OP_WIDTHS[seed as usize % ASSIGN_OP_WIDTHS.len()];
        if w > 16 {
            continue;
        } // mul overflow
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_03);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;

        let expected = (a * b) & m;

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let src = assign_op_source("*", w, &a_lit, &b_lit);

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch *=:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn assign_op_div_matches_golden() {
    // `a /= b` — pembagian compound assign (unsigned).
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let w = ASSIGN_OP_WIDTHS[seed as usize % ASSIGN_OP_WIDTHS.len()];
        if w > 16 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xDD_04);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;
        if b == 0 {
            continue;
        } // skip div-by-zero

        let expected = a / b;

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let src = assign_op_source("/", w, &a_lit, &b_lit);

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch /=:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn assign_op_mod_matches_golden() {
    // `a %= b` — modulus compound assign (unsigned).
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let w = ASSIGN_OP_WIDTHS[seed as usize % ASSIGN_OP_WIDTHS.len()];
        if w > 16 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE_05);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;
        if b == 0 {
            continue;
        }

        let expected = a % b;

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let src = assign_op_source("%", w, &a_lit, &b_lit);

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch %%=\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn assign_op_shl_matches_golden() {
    // `a <<= b` — shift left compound assign.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let w = ASSIGN_OP_WIDTHS[seed as usize % ASSIGN_OP_WIDTHS.len()];
        if w > 32 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_06);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) % w as u128;

        let expected = if b >= w as u128 { 0 } else { (a << b) & m };

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let src = assign_op_source("<<", w, &a_lit, &b_lit);

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch <<=\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn assign_op_shr_matches_golden() {
    // `a >>= b` — shift right compound assign (unsigned/logical).
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..120u64 {
        let w = ASSIGN_OP_WIDTHS[seed as usize % ASSIGN_OP_WIDTHS.len()];
        if w > 32 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x10_07);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) % w as u128;

        let expected = if b >= w as u128 { 0 } else { (a >> b) & m };

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let src = assign_op_source(">>", w, &a_lit, &b_lit);

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch >>=:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
