//! Fuzz differential force/release — `force x = val; ... release x;`
//!
//! Blind spot: fuzzer existing tidak menguji force/release sama sekali.
//! Force/release digunakan di UVM dan verifikasi hirarki. Edge cases:
//! - Force constant ke wire, release → wire kembali ke driver
//! - Force ke reg, release → reg tetap di force value (LRM)
//! - Force di dalam initial block
//! - Force value = 0, MAX, alternating bits

const FR_WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

fn mask_of128(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("force-release-sim".to_string())
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

#[test]
fn fr_force_constant_wire_release_matches_golden() {
    // `force y = val; ... release y;` — force ke wire, release.
    // After release, wire kembali ke driver (assign y = 0 → y = 0).
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let w = FR_WIDTHS[seed as usize % FR_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x66_FF);
        let m = mask_of128(w);
        let force_val = rng.u128(..) & m;

        // After release, wire returns to its continuous assign value = 0
        let expected = 0u64;

        let fval_lit = format!("{}'h{:x}", w, force_val);
        let src = format!(
            "module fr_wire_mod;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {w}'h0;\n\
             \x20   initial begin\n\
             \x20       force y = {fval};\n\
             \x20       #10;\n\
             \x20       release y;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            w = w,
            fval = fval_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} fval={:#x} harap=0x0 dapat={:?}",
                seed, w, force_val, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch force/release wire:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn fr_force_reg_keeps_value_after_release() {
    // `reg [W:0] y; ... force y = val; release y;` — force ke reg.
    // After release, reg KEEPS the forced value (LRM §10.6.2).
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let w = FR_WIDTHS[seed as usize % FR_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x77_77);
        let m = mask_of128(w);
        let init_val = rng.u128(..) & m;
        let force_val = rng.u128(..) & m;

        // After release, reg keeps forced value
        let expected = force_val;

        let ival_lit = format!("{}'h{:x}", w, init_val);
        let fval_lit = format!("{}'h{:x}", w, force_val);
        let src = format!(
            "module fr_reg_mod;\n\
             \x20   reg [{hi}:0] y;\n\
             \x20   initial begin\n\
             \x20       y = {ival};\n\
             \x20       force y = {fval};\n\
             \x20       release y;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            ival = ival_lit,
            fval = fval_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} init={:#x} force={:#x} harap={:#x} dapat={:?}",
                seed, w, init_val, force_val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch force/release reg:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn fr_force_value_zero_matches() {
    // Force y = 0 → value is 0.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for &w in &FR_WIDTHS {
        let src = format!(
            "module fr_zero_mod;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {w}'hFF;\n\
             \x20   initial begin\n\
             \x20       force y = {w}'h0;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            w = w,
        );

        let actual = run_sim(src);

        if actual != Some(0) {
            mismatch.push(format!(
                "w={} harap=0x0 dapat={:?}",
                w, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 2, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch force zero:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
