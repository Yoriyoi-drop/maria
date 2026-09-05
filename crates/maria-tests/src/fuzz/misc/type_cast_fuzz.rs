//! Fuzz differential type casting — `$signed`, `$unsigned`, width casting.
//!
//! Blind spot: fuzzer existing menguji mixed_sign, tapi type casting
//! `$signed(x)` / `$unsigned(x)` dalam expression chains belum terekspos.
//! Edge cases:
//! - `$signed(8'hFF)` — harus bernilai -1 (signed)
//! - `$unsigned(-1)` — harus bernilai MAX unsigned
//! - Cast dalam expression chain: `$signed(a) + b`
//! - Cast pada hasil expression: `$signed(a + b)`

const TC_WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

fn mask_of128(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("type-cast-sim".to_string())
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

#[test]
fn tc_unsigned_passthrough_matches() {
    // `$unsigned(x)` — passthrough, x tetap unsigned.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = TC_WIDTHS[seed as usize % TC_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x11_11);
        let m = mask_of128(w);
        let val = rng.u128(..) & m;

        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module tc_unsigned_mod;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = $unsigned({val});\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            val = val_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} val={:#x} harap={:#x} dapat={:?}",
                seed, w, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch $unsigned:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn tc_signed_passthrough_matches() {
    // `$signed(x)` — passthrough, x tetap same bits.
    // Bit pattern tidak berubah, hanya interpretasi signedness.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = TC_WIDTHS[seed as usize % TC_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x22_22);
        let m = mask_of128(w);
        let val = rng.u128(..) & m;

        // $signed doesn't change the bit pattern
        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module tc_signed_mod;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = $signed({val});\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            val = val_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} val={:#x} harap={:#x} dapat={:?}",
                seed, w, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch $signed:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn tc_signed_unsigned_add_matches_golden() {
    // `$signed(a) + $unsigned(b)` — mixed signedness addition.
    // LRM §11.8.2: ada operand unsigned → hasil unsigned.
    // Tetapi $signed(a) membuat a signed, $unsigned(b) membuat b unsigned.
    // Result: unsigned (karena b unsigned → hasil unsigned).
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = TC_WIDTHS[seed as usize % TC_WIDTHS.len()];
        if w > 32 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x33_33);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;

        // Result is unsigned addition masked to w bits
        let expected = (a + b) & m;

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);
        let src = format!(
            "module tc_mixed_mod;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = $signed({a}) + $unsigned({b});\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            a = a_lit,
            b = b_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch $signed+$unsigned:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
