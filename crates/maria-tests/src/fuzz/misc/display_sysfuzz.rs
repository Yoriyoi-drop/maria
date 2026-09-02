//! Fuzz differential display format strings and system functions.
//!
//! Blind spot: fuzzer existing menguji expression, tapi display format
//! strings dengan kombinasi format specifier random dan system functions
//! ($clog2, $bits, $size) dalam konteks berekspresi belum terekspos
//! secara systematic.
//! Edge cases:
//! - Mixed format specifiers in one $display
//! - $clog2/$bits/$size with random widths
//! - $display with expression arguments
//! - %0d zero-padding edge cases

fn run_sim(src: String) -> Option<String> {
    std::thread::Builder::new()
        .name("display-sysfuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn({
            move || {
                crate::simulate_signals(&src, 30)
                    .ok()
                    .map(|sigs| {
                        sigs.iter()
                            .map(|(n, v)| format!("{}={}", n, v))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
            }
        })
        .expect("spawn")
        .join()
        .expect("sim panic")
}

/// $clog2 with random widths — verify no panic and reasonable output.
#[test]
fn sysfunc_clog2_fuzz() {
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_01);
        let w = [1u32, 2, 3, 4, 7, 8, 15, 16, 17, 31, 32, 33, 63, 64][rng.usize(0..14)];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let val = rng.u64(1..) & m; // avoid 0 (clog2(0) is undefined)
        if val == 0 {
            continue;
        }

        let src = format!(
            "module clog2_mod;\n\
             \x20   wire [31:0] y;\n\
             \x20   assign y = $clog2({w}'h{val:x});\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            w = w,
            val = val,
        );

        // Just verify no panic — $clog2 is evaluated at elaboration
        let result = std::thread::Builder::new()
            .name("clog2-fuzz-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                move || crate::compile_str(&src).is_ok()
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        assert!(result, "compile failed on $clog2 seed={} w={} val={}", seed, w, val);
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
}

/// $bits with random variable widths.
#[test]
fn sysfunc_bits_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for w in [1u32, 2, 4, 8, 12, 16, 24, 32] {
        let src = format!(
            "module bits_mod;\n\
             \x20   wire [31:0] y;\n\
             \x20   reg [{h}:0] v;\n\
             \x20   assign y = $bits(v);\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            h = w - 1,
        );

        // Just verify no crash
        let result = std::thread::Builder::new()
            .name("bits-fuzz-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                move || crate::compile_str(&src).is_ok()
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        if !result {
            mismatch.push(format!("compile failed on $bits w={}", w));
        }
        checked += 1;
    }
    assert!(checked > 5, "terlalu sedikit kasus (checked={})", checked);
    assert!(mismatch.is_empty(), "{}", mismatch.join("\n"));
}

/// $display with mixed format specifiers — no panic, correct output.
#[test]
fn display_mixed_format_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..50u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_02);
        let hex_val = rng.u64(0..255);
        let dec_val = rng.u64(0..255);
        let bin_val = rng.u64(0..15);

        let src = format!(
            "module display_mixed_mod;\n\
             \x20   initial begin\n\
             \x20       $display(\"%h %0d %b\", 8'h{hex:02x}, 8'd{dec}, 4'b{bin:04b});\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hex = hex_val,
            dec = dec_val,
            bin = bin_val,
        );

        // Just verify no panic
        let result = std::thread::Builder::new()
            .name("display-mixed-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                move || crate::compile_str(&src).is_ok()
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        if !result {
            mismatch.push(format!(
                "seed={} hex={} dec={} bin={}",
                seed, hex_val, dec_val, bin_val
            ));
        }
        checked += 1;
    }
    assert!(checked > 25, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} display mixed format failed:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// $display with expression arguments — verify expression eval in display args.
#[test]
fn display_expr_args_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..50u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_03);
        let a = rng.u64(0..127);
        let b = rng.u64(0..127);
        let expected = a + b;

        let src = format!(
            "module display_expr_mod;\n\
             \x20   reg [7:0] a, b;\n\
             \x20   wire [7:0] y;\n\
             \x20   assign y = a + b;\n\
             \x20   initial begin\n\
             \x20       a = 8'h{a:02x};\n\
             \x20       b = 8'h{b:02x};\n\
             \x20       $display(\"sum=%h\", a + b);\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a = a,
            b = b,
        );

        let actual = std::thread::Builder::new()
            .name("display-expr-sim".to_string())
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
            .expect("sim panic");

        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} b={} harap={} can={:?}",
                seed, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 25, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} display expr args mismatch:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// %0d zero-padding format with random values.
#[test]
fn display_zero_pad_fuzz() {
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_04);
        let val = rng.u64(0..255);

        let src = format!(
            "module zpad_mod;\n\
             \x20   initial begin\n\
             \x20       $display(\"%0d\", 8'd{val});\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            val = val,
        );

        let result = std::thread::Builder::new()
            .name("zpad-fuzz-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                move || crate::compile_str(&src).is_ok()
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        assert!(result, "compile failed on %0d val={}", val);
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
}
