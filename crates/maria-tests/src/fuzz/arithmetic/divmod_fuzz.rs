//! Fuzz differential division and modulo — edge cases.
//!
//! Tests: unsigned div, unsigned mod, div by power of 2, mod near boundary.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("divmod-fuzz-sim".to_string())
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

/// Unsigned division: a / b (no division by zero).
#[test]
fn unsigned_div_basic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF1_01);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let a = rng.u64(..) & m;
        let b = if m == 0 { 1 } else { rng.u64(1..=m) };

        let expected = a / b;

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);

        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
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
                seed, w, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch unsigned div:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Unsigned modulo: a % b.
#[test]
fn unsigned_mod_basic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF1_02);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let a = rng.u64(..) & m;
        let b = if m == 0 { 1 } else { rng.u64(1..=m) };

        let expected = a % b;

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);

        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
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
                seed, w, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch unsigned mod:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Division by power of 2 — compiler may optimize to shift.
#[test]
fn div_power_of_two() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF1_03);
        let w = [8u32, 16][seed as usize % 2];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let val = rng.u64(..) & m;
        let shift = (seed as u64 % (w as u64 - 1)) + 1; // 1..w-1
        let divisor = 1u64 << shift;

        let expected = val / divisor;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {val} / {div};\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            hi = w - 1,
            val = val_lit,
            div = divisor,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} val={:#x} div={} harap={:#x} dapat={:?}",
                seed, w, val, divisor, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch div power of 2:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Modulo near boundary: a % (a+1) = a for a < width.
#[test]
fn mod_near_boundary() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF1_04);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let a = rng.u64(0..m);
        let b = a + 1; // a % (a+1) = a when a < width and a+1 doesn't overflow

        let expected = a % b;

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);

        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
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
                seed, w, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch mod near boundary:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
