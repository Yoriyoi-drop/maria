//! Fuzz differential inside operator with range lists.
//!
//! Tests: inside {[a:b]} range match, inside {val1, val2} discrete match.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("inside-range-fuzz-sim".to_string())
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

/// inside with range: val inside {[lo:hi]} → 1 if lo <= val <= hi.
#[test]
fn inside_range_match() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_01);
        let val = rng.u64(0..256);
        let lo = rng.u64(0..128);
        let hi = lo + rng.u64(1..128).min(255 - lo);

        let expected = if val >= lo && val <= hi { 1u64 } else { 0u64 };

        let src = format!(
            "module test;\n\
             \x20   reg [7:0] val;\n\
             \x20   wire y;\n\
             \x20   assign y = val inside {{[{lo}:{hi}]}};\n\
             \x20   initial begin\n\
             \x20       val = {val};\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            lo = lo,
            hi = hi,
            val = val,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} val={} lo={} hi={} harap={} dapat={:?}",
                seed, val, lo, hi, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch inside range:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// inside with discrete values: val inside {1, 3, 5, 7}.
#[test]
fn inside_discrete_match() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_02);
        let val = rng.u64(0..16);
        let list = [1u64, 3, 5, 7, 9, 11, 13, 15];
        let expected = if list.contains(&val) { 1u64 } else { 0u64 };

        let list_str = list
            .iter()
            .map(|v| format!("{}", v))
            .collect::<Vec<_>>()
            .join(",");

        let src = format!(
            "module test;\n\
             \x20   reg [3:0] val;\n\
             \x20   wire y;\n\
             \x20   assign y = val inside {{{list}}};\n\
             \x20   initial begin\n\
             \x20       val = {val};\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            list = list_str,
            val = val,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} val={} harap={} dapat={:?}",
                seed, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch inside discrete:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// inside with mixed ranges and values: val inside {[0:3], 7, [10:15]}.
#[test]
fn inside_mixed_match() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_03);
        let val = rng.u64(0..20);
        // Match if val in [0:3] or val == 7 or val in [10:15]
        let expected = if (val <= 3) || val == 7 || (val >= 10 && val <= 15) {
            1u64
        } else {
            0u64
        };

        let src = format!(
            "module test;\n\
             \x20   reg [4:0] val;\n\
             \x20   wire y;\n\
             \x20   assign y = val inside {{[0:3], 7, [10:15]}};\n\
             \x20   initial begin\n\
             \x20       val = {val};\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            val = val,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} val={} harap={} dapat={:?}",
                seed, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch inside mixed:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
