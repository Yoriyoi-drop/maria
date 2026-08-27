//! Fuzz differential shift edge cases.
//!
//! Tests: shift by amount >= width, zero shift, shift by variable, chained shifts.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("shift-edge-fuzz-sim".to_string())
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

/// Left shift by amount >= width → result is 0.
#[test]
fn shift_left_by_ge_width() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xE1_01);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let val = rng.u64(..) & m;
        let sh = (w as u64) + (seed as u64 % 8); // shift >= width

        let expected = 0u64;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {val} << {sh};\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            hi = w - 1,
            val = val_lit,
            sh = sh,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} val={:#x} sh={} harap=0 dapat={:?}",
                seed, w, val, sh, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch shift left >= width:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Right shift by amount >= width → result is 0.
#[test]
fn shift_right_by_ge_width() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xE1_02);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let val = rng.u64(..) & m;
        let sh = (w as u64) + (seed as u64 % 8);

        let expected = 0u64;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {val} >> {sh};\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            hi = w - 1,
            val = val_lit,
            sh = sh,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} val={:#x} sh={} harap=0 dapat={:?}",
                seed, w, val, sh, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch shift right >= width:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Shift by 0 → identity.
#[test]
fn shift_by_zero() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xE1_03);
        let w = [4u32, 8, 16, 32][seed as usize % 4];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let val = rng.u64(..) & m;
        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {val} << 0;\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            hi = w - 1,
            val = val_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} val={:#x} harap={:#x} dapat={:?}",
                seed, w, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch shift by zero:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Chained left shift: (a << b) << c = a << (b + c).
#[test]
fn chained_left_shift() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xE1_04);
        let w = [8u32, 16][seed as usize % 2];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let val = rng.u64(..) & m;
        let b = rng.u64(0..4);
        let c = rng.u64(0..4);

        let expected = (val << b) << c & m;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = ({val} << {b}) << {c};\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            hi = w - 1,
            val = val_lit,
            b = b,
            c = c,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} val={:#x} b={} c={} harap={:#x} dapat={:?}",
                seed, w, val, b, c, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch chained left shift:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
