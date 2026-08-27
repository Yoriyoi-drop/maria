//! Fuzz differential packed array assignment — part-select write.
//!
//! Tests: bit-range write, index write, multiple packed dims write.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("packed-write-fuzz-sim".to_string())
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

/// Range select write: a[3:0] = val; update lower 4 bits.
#[test]
fn packed_range_select_write() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE_01);
        let w = [8u32, 16, 32][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let base = rng.u64(..) & m;
        let new_lo = rng.u64(..) & 0xF; // 4-bit value to write

        // After a[3:0] = new_lo, lower 4 bits replaced, upper bits preserved
        let expected = (base & !0xF) | new_lo;

        let base_lit = format!("{}'h{:x}", w, base);
        let new_lo_lit = format!("4'h{:x}", new_lo);

        let src = format!(
            "module test;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = a;\n\
             \x20   initial begin\n\
             \x20       a = {base};\n\
             \x20       a[3:0] = {new_lo};\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            base = base_lit,
            new_lo = new_lo_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} base={:#x} new_lo={:#x} harap={:#x} dapat={:?}",
                seed, w, base, new_lo, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch packed range write:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Bit select write: a[idx] = val; update single bit.
#[test]
fn packed_bit_select_write() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE_02);
        let w = [8u32, 16, 32][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let base = rng.u64(..) & m;
        let bit_idx = (seed as u64 % w as u64) as usize;
        let bit_val = if rng.bool() { 1u64 } else { 0u64 };

        let expected = (base & !(1u64 << bit_idx)) | (bit_val << bit_idx);

        let base_lit = format!("{}'h{:x}", w, base);
        let bit_val_lit = format!("1'b{}", bit_val);

        let src = format!(
            "module test;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = a;\n\
             \x20   initial begin\n\
             \x20       a = {base};\n\
             \x20       a[{idx}] = {val};\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            base = base_lit,
            idx = bit_idx,
            val = bit_val_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} base={:#x} idx={} val={} harap={:#x} dapat={:?}",
                seed, w, base, bit_idx, bit_val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch packed bit write:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Two separate part-select writes: a[7:4] = hi; a[3:0] = lo;
#[test]
fn packed_dual_range_write() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE_03);
        let w = [8u32, 16, 32][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let base = rng.u64(..) & m;
        let new_hi = rng.u64(..) & 0xF;
        let new_lo = rng.u64(..) & 0xF;

        // First write base, then overwrite hi and lo separately
        // Only lower 8 bits are affected: base upper bits preserved
        let expected = (base & !0xFF) | (new_hi << 4) | new_lo;

        let base_lit = format!("{}'h{:x}", w, base);
        let new_hi_lit = format!("4'h{:x}", new_hi);
        let new_lo_lit = format!("4'h{:x}", new_lo);

        let src = format!(
            "module test;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = a;\n\
             \x20   initial begin\n\
             \x20       a = {base};\n\
             \x20       a[7:4] = {new_hi};\n\
             \x20       a[3:0] = {new_lo};\n\
             \x20       #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            base = base_lit,
            new_hi = new_hi_lit,
            new_lo = new_lo_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} base={:#x} hi={:#x} lo={:#x} harap={:#x} dapat={:?}",
                seed, w, base, new_hi, new_lo, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch dual range write:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
