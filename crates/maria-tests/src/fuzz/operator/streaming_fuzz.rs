//! Fuzz differential streaming concatenation operator.
//!
//! Tests: `{<<N{expr}}` (left streaming) and `{>>N{expr}}` (right streaming)
//! with various widths and slice sizes.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("streaming-fuzz-sim".to_string())
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

/// Simple left streaming: {<<N{a}} reverses the bit order in slices of N.
/// For 8-bit a = a7..a0, {<<8{a}} = {a7..a0} (no change with slice=width).
#[test]
fn streaming_left_identity() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_01);
        let w = [8u32, 16, 32][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let val = rng.u64(..) & m;
        // {<<8{a}} with slice_size=8 on 8-bit = identity (no reorder within 8-bit slice)
        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module test;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {{<<{slice}{{a}}}};\n\
             \x20   initial begin\n\
             \x20       a = {val};\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            slice = w,
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
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch streaming left identity:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Right streaming: {>>8{a}} with slice_size=8 on 8-bit = identity.
#[test]
fn streaming_right_identity() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_02);
        let w = [8u32, 16, 32][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let val = rng.u64(..) & m;
        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module test;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {{>>{slice}{{a}}}};\n\
             \x20   initial begin\n\
             \x20       a = {val};\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            slice = w,
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
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch streaming right identity:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Left streaming with slice=1: full bit reversal.
/// For 8-bit a = a7..a0, {<<1{a}} = a0..a7.
#[test]
fn streaming_left_bit_reverse() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_03);
        let w = 8u32; // fixed 8-bit for bit-reverse test
        let m = (1u64 << w) - 1;

        let val = rng.u64(..) & m;

        // Manual bit reverse
        let mut expected = 0u64;
        for i in 0..w {
            if val & (1u64 << i) != 0 {
                expected |= 1u64 << (w - 1 - i);
            }
        }
        expected &= m;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module test;\n\
             \x20   reg [7:0] a;\n\
             \x20   wire [7:0] y;\n\
             \x20   assign y = {{<<1{{a}}}};\n\
             \x20   initial begin\n\
             \x20       a = {val};\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            val = val_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} val={:#010b} harap={:#010b} dapat={:?}",
                seed, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch streaming bit reverse:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
