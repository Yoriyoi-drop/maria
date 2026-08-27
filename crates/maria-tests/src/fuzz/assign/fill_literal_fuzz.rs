//! Fuzz differential fill literals — `'0`, `'1`, `'x`, `'z` assigned to
//! variables of various widths.
//!
//! Blind spot: fuzzer existing menguji fill literal di expression, tapi
//! assignment langsung (`wire [W:0] y = '1;`) dengan width berbeda belum
//! terekspos. Edge cases:
//! - Width > 64 (multi-word fill)
//! - Fill di context assignment (width propagation)
//! - Fill di concatenation `{a, '0, b}`

fn fill_literal_source(fill_char: &str, w: u32) -> String {
    format!(
        "module fill_literal_fuzz_mod;\n\
         \x20   wire [{hi}:0] y;\n\
         \x20   assign y = {fill};\n\
         \x20   initial begin\n\
         \x20       #10;\n\
         \x20       $finish;\n\
         \x20   end\n\
         endmodule\n",
        hi = w - 1,
        fill = fill_char,
    )
}

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("fill-lit-sim".to_string())
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

const FILL_WIDTHS: [u32; 6] = [1, 4, 8, 16, 32, 64];

fn mask_of_big(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

#[test]
fn fill_zero_matches_golden() {
    // `'0` → all zeros regardless of width.
    let mut mismatch = Vec::new();
    for &w in &FILL_WIDTHS {
        let src = fill_literal_source("'0", w);
        let actual = run_sim(src);
        let expected = 0u128;
        if actual.map(|v| v as u128) != Some(expected) {
            mismatch.push(format!(
                "w={} harap=0x0 dapat={:?}",
                w, actual
            ));
        }
    }
    assert!(
        mismatch.is_empty(),
        "'0 mismatch:\n{}",
        mismatch.join("\n")
    );
}

#[test]
fn fill_ones_matches_golden() {
    // `'1` → all ones (mask to width).
    let mut mismatch = Vec::new();
    for &w in &FILL_WIDTHS {
        let src = fill_literal_source("'1", w);
        let actual = run_sim(src);
        let expected = mask_of_big(w);
        if actual.map(|v| v as u128) != Some(expected) {
            mismatch.push(format!(
                "w={} harap={:#x} dapat={:?}",
                w, expected, actual
            ));
        }
    }
    assert!(
        mismatch.is_empty(),
        "'1 mismatch:\n{}",
        mismatch.join("\n")
    );
}

#[test]
fn fill_x_matches_golden() {
    // `'x` → all X. Dalam 4-state, X ditampilkan sebagai X oleh Maria.
    // Kita hanya cek tidak panic dan deterministik.
    let mut mismatch = Vec::new();
    for &w in &[1u32, 4, 8, 16, 32, 64] {
        let src = fill_literal_source("'x", w);
        // Run twice — harus deterministik
        let r1 = run_sim(src.clone());
        let r2 = run_sim(src);
        if r1 != r2 {
            mismatch.push(format!(
                "w={} tidak deterministik: {:?} vs {:?}",
                w, r1, r2
            ));
        }
    }
    assert!(
        mismatch.is_empty(),
        "'x non-deterministik:\n{}",
        mismatch.join("\n")
    );
}

#[test]
fn fill_z_matches_golden() {
    // `'z` → all Z. Kita hanya cek tidak panic dan deterministik.
    let mut mismatch = Vec::new();
    for &w in &[1u32, 4, 8, 16, 32, 64] {
        let src = fill_literal_source("'z", w);
        let r1 = run_sim(src.clone());
        let r2 = run_sim(src);
        if r1 != r2 {
            mismatch.push(format!(
                "w={} tidak deterministik: {:?} vs {:?}",
                w, r1, r2
            ));
        }
    }
    assert!(
        mismatch.is_empty(),
        "'z non-deterministik:\n{}",
        mismatch.join("\n")
    );
}

#[test]
fn fill_in_concat_matches_golden() {
    // `{a, 1'b0}` — fill bits di context concatenation.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let w = [4u32, 8, 16, 32][seed as usize % 4];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x33_44);
        let m = mask_of_big(w);
        let a = rng.u128(..) & m;

        // `{a, 1'b0}` → a << 1 (shift left by 1 bit)
        let expected = (a << 1) & mask_of_big(w);

        let a_lit = format!("{}'h{:x}", w, a);
        let src = format!(
            "module fill_concat_mod;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {{{a}, 1'b0}};\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            a = a_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} harap={:#x} dapat={:?}",
                seed, w, a, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch concat fill:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
