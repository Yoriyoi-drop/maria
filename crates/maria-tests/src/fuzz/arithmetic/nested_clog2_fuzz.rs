//! Fuzz differential nested $clog2 and system function edge cases.
//!
//! Tests: $clog2($clog2(N)), $clog2 on powers of 2, $clog2(1), $bits edge cases.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("nested-clog2-fuzz-sim".to_string())
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

/// Compute $clog2 equivalent in Rust.
/// $clog2(1)=0, $clog2(2)=1, $clog2(3)=2, $clog2(4)=2, $clog2(5)=3
fn my_clog2(v: u64) -> u64 {
    if v <= 1 {
        0
    } else if v.is_power_of_two() {
        (v.trailing_zeros()) as u64
    } else {
        (64u32 - v.leading_zeros()) as u64
    }
}

/// $clog2 on small values — known correct values.
#[test]
fn clog2_known_values() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    let known = [
        (1u64, 0u64), (2, 1), (3, 2), (4, 2), (5, 3), (6, 3), (7, 3),
        (8, 3), (9, 4), (10, 4), (15, 4), (16, 4), (17, 5), (31, 5),
        (32, 5), (33, 6), (63, 6), (64, 6), (65, 7), (127, 7), (128, 7),
        (255, 8), (256, 8), (257, 9), (1023, 10), (1024, 10),
    ];

    for &(val, expected) in &known {
        let src = format!(
            "module test;\n\
             \x20   wire [31:0] y;\n\
             \x20   assign y = $clog2({val});\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            val = val,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "$clog2({}) harap={} dapat={:?}",
                val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 10, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch clog2 known:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Nested $clog2: $clog2($clog2(N)).
#[test]
fn clog2_nested() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for &val in &[4u64, 8, 16, 32, 64, 128, 256, 512, 1024] {
        let clog2_val = my_clog2(val);
        let expected = my_clog2(clog2_val);

        let src = format!(
            "module test;\n\
             \x20   wire [31:0] y;\n\
             \x20   assign y = $clog2($clog2({val}));\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            val = val,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "$clog2($clog2({})) harap={} dapat={:?}",
                val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 5, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch nested clog2:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// $clog2 on parameterized width.
#[test]
fn clog2_parameterized() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF2_03);
        let w = rng.u64(1..=256);
        let expected = my_clog2(w);

        let src = format!(
            "module test;\n\
             \x20   parameter N = {w};\n\
             \x20   wire [31:0] y;\n\
             \x20   assign y = $clog2(N);\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            w = w,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} $clog2({}) harap={} dapat={:?}",
                seed, w, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch clog2 parameterized:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
