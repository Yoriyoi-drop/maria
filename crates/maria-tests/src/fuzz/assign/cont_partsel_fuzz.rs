//! Fuzz differential continuous assignment to part-select.
//!
//! Tests: assign to range, assign to bit, multiple part-select assigns.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("cont-partsel-fuzz-sim".to_string())
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

/// Reg part-select write in initial block.
#[test]
fn reg_range_select_write() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA2_01);
        let lo = rng.u64(..) & 0xF;
        let hi = rng.u64(..) & 0xF;
        let base = rng.u64(..) & 0xFF;

        // Write lo to [3:0] and hi to [7:4]
        let expected = (hi << 4) | lo;

        let lo_lit = format!("4'h{:x}", lo);
        let hi_lit = format!("4'h{:x}", hi);
        let base_lit = format!("8'h{:x}", base);

        let src = format!(
            "module test;\n\
             \x20   reg [7:0] a;\n\
             \x20   wire [7:0] y;\n\
             \x20   assign y = a;\n\
             \x20   initial begin\n\
             \x20       a = {base_val};\n\
             \x20       a[3:0] = {lo_val};\n\
             \x20       a[7:4] = {hi_val};\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            base_val = base_lit,
            lo_val = lo_lit,
            hi_val = hi_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} lo={:#x} hi={:#x} harap={:#x} dapat={:?}",
                seed, lo, hi, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch reg range write:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Reg bit select write in initial block.
#[test]
fn reg_bit_select_write() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA2_02);
        let mut base = rng.u64(..) & 0xFF;
        let bit_idx = seed as usize % 8;
        let bit_val = if rng.bool() { 1u64 } else { 0u64 };

        // Write bit
        base = (base & !(1u64 << bit_idx)) | (bit_val << bit_idx);
        let expected = base;

        let base_orig = base ^ (1u64 << bit_idx);
        let base_lit = format!("8'h{:x}", base_orig);
        let bit_val_lit = format!("1'b{}", bit_val);

        let src = format!(
            "module test;\n\
             \x20   reg [7:0] a;\n\
             \x20   wire [7:0] y;\n\
             \x20   assign y = a;\n\
             \x20   initial begin\n\
             \x20       a = {base_v};\n\
             \x20       a[{idx}] = {bit_v};\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            base_v = base_lit,
            bit_v = bit_val_lit,
            idx = bit_idx,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} idx={} bit={} base={:#x} harap={:#x} dapat={:?}",
                seed, bit_idx, bit_val, base_orig, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch reg bit select:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
