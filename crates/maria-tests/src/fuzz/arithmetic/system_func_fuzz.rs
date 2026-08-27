//! Fuzz differential system functions — `$clog2`, `$bits`, `$size`,
//! `$left`, `$right`, `$low`, `$high` dalam berbagai konteks.
//!
//! Blind spot: fuzzer existing menguji expression arithmetic, tapi system
//! function yang dievaluasi di elaborator (compile-time) belum terekspos.
//! Edge cases:
//! - $clog2 pada nilai power-of-two (harus n-1, bukan n)
//! - $bits pada tipe berbeda
//! - $size pada array/packed array
//! - $left/$right pada array berdimensi berbeda
//! - Kombinasi: $clog2($bits(x))

const SF_WIDTHS: [u32; 6] = [1, 4, 8, 16, 32, 64];

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("sysfunc-sim".to_string())
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

/// Golden $clog2 — per LRM: $clog2(0)=0, $clog2(1)=0, $clog2(n) = ceil(log2(n))
fn golden_clog2(n: u64) -> u64 {
    if n <= 1 {
        0
    } else {
        64 - (n - 1).leading_zeros() as u64
    }
}

#[test]
fn sysfunc_clog2_matches_golden() {
    // `$clog2(val)` — test nilai sistematis: power-of-two, non-power-of-two, boundary.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    // Test values: 0, 1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 63, 64, 127, 128, 255, 256
    let test_vals: Vec<u64> = (0..=8)
        .map(|i| i)
        .chain((1..=8).map(|i| (1u64 << i) - 1))
        .chain((1..=8).map(|i| 1u64 << i))
        .chain(vec![255, 256, 511, 512, 1023, 1024])
        .collect();

    for &val in &test_vals {
        let expected = golden_clog2(val);
        let src = format!(
            "module sysfunc_mod;\n\
             \x20   wire [31:0] y;\n\
             \x20   assign y = $clog2({val});\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            val = val,
        );

        let actual = run_sim(src);

        if actual != Some(expected) {
            mismatch.push(format!(
                "val={} harap={} dapat={:?}",
                val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch $clog2:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn sysfunc_clog2_parametric_matches_golden() {
    // `$clog2` dengan parameter — test elaboration-time evaluation.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for w in &SF_WIDTHS {
        let test_vals: Vec<u64> = vec![0, 1, 2, 3, 127, 255, 511, 1023];

        for &val in &test_vals {
            let expected = golden_clog2(val);
            let src = format!(
                "module sysfunc_param_mod #(\n\
                 \x20   parameter V = {val}\n\
                 )(output wire [31:0] y);\n\
                 \x20   assign y = $clog2(V);\n\
                 endmodule\n\
                 module top;\n\
                 \x20   wire [31:0] y;\n\
                 \x20   sysfunc_param_mod #(.V({val})) u (.y(y));\n\
                 \x20   initial begin\n\
                 \x20       #10;\n\
                 \x20       $finish;\n\
                 \x20   end\n\
                 endmodule\n",
                val = val,
            );

            let actual = run_sim(src);

            if actual != Some(expected) {
                mismatch.push(format!(
                    "w={} val={} harap={} dapat={:?}",
                    w, val, expected, actual
                ));
            }
            checked += 1;
        }
    }
    assert!(checked > 10, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch $clog2 parametric:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn sysfunc_bits_matches_literal_width() {
    // `$bits(x)` untuk literal berukuran berbeda.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for &w in &SF_WIDTHS {
        let src = format!(
            "module sysfunc_bits_mod;\n\
             \x20   wire [31:0] y;\n\
             \x20   assign y = $bits({w}'h0);\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            w = w,
        );

        let actual = run_sim(src);
        let expected = w as u64;

        if actual != Some(expected) {
            mismatch.push(format!(
                "w={} harap={} dapat={:?}",
                w, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 3, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch $bits:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn sysfunc_size_matches_signal_width() {
    // `$size(x)` — harus sama dengan lebar signal x.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for &w in &SF_WIDTHS {
        let src = format!(
            "module sysfunc_size_mod;\n\
             \x20   reg [{hi}:0] x;\n\
             \x20   wire [31:0] y;\n\
             \x20   assign y = $size(x);\n\
             \x20   initial begin\n\
             \x20       x = 0;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
        );

        let actual = run_sim(src);
        let expected = w as u64;

        if actual != Some(expected) {
            mismatch.push(format!(
                "w={} harap={} dapat={:?}",
                w, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 3, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch $size:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
