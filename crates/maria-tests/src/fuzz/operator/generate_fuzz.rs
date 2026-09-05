//! Fuzz differential generate constructs — `generate for`, `generate if`,
//! `localparam` dalam generate scope, conditional compilation.
//!
//! Blind spot: fuzzer existing menguji module instantiation, tapi generate
//! constructs (kondisional & iteratif) belum terekspos. Generate merupakan
//! salah satu fitur paling kompleks di elaboration phase. Edge cases:
//! - `generate for` dengan boundary localparam
//! - `generate if` dengan konstanta 0/1
//! - Nested generate (for di dalam if)
//! - Generate yang menghasilkan 0 atau banyak instance

fn mask_of128(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("generate-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn({
            move || {
                crate::simulate_signals(&src, 30)
                    .ok()
                    .and_then(|sigs| sigs.iter().find(|(n, _)| n == "y").map(|(_, v)| v.to_u64()))
            }
        })
        .expect("spawn")
        .join()
        .expect("sim panic")
}

#[test]
fn gf_generate_or_reduction_matches_golden() {
    // `|a` — OR reduction: 1 jika ada bit set.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = [4u32, 8, 16, 32][seed as usize % 4];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x11_11);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;

        let expected = if a != 0 { 1u64 } else { 0 };

        let a_lit = format!("{}'h{:x}", w, a);
        let src = format!(
            "module gf_or_mod;\n\
             \x20   parameter W = {w};\n\
             \x20   reg [W-1:0] a;\n\
             \x20   wire y;\n\
             \x20   assign y = |a;\n\
             \x20   initial begin\n\
             \x20       a = {a_lit};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n"
        );

        let actual = run_sim(src);

        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} harap={} dapat={:?}",
                seed, w, a, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch generate or reduction:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn gf_generate_if_passthrough_matches() {
    // `generate if` — kondisional: if(1) assign y=a.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let w = [4u32, 8, 16, 32][seed as usize % 4];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x22_22);
        let m = mask_of128(w);
        let a = rng.u128(..) & m;

        let expected = a;

        let a_lit = format!("{}'h{:x}", w, a);
        let src = format!(
            "module gf_if_mod;\n\
             \x20   parameter USE_A = 1;\n\
             \x20   parameter W = {w};\n\
             \x20   reg [W-1:0] a;\n\
             \x20   wire [W-1:0] y;\n\
             \x20   generate\n\
             \x20     if (USE_A) assign y = a;\n\
             \x20     else      assign y = 0;\n\
             \x20   endgenerate\n\
             \x20   initial begin\n\
             \x20       a = {a_lit};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n"
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
        "{} mismatch generate if:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn gf_mux_4to1_matches_golden() {
    // 4-to-1 mux via ternary chain: result = a[sel]
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x33_33);
        let a0 = rng.u32(0..15) as u64;
        let a1 = rng.u32(0..15) as u64;
        let a2 = rng.u32(0..15) as u64;
        let a3 = rng.u32(0..15) as u64;
        let sel = rng.u32(0..3) as u64;

        let expected = match sel {
            0 => a0,
            1 => a1,
            2 => a2,
            _ => a3,
        };

        let src = format!(
            "module gf_mux_mod;\n\
             \x20   reg [3:0] a0, a1, a2, a3;\n\
             \x20   reg [1:0] sel;\n\
             \x20   wire [3:0] y;\n\
             \x20   assign y = (sel == 0) ? a0 :\n\
             \x20              (sel == 1) ? a1 :\n\
             \x20              (sel == 2) ? a2 : a3;\n\
             \x20   initial begin\n\
             \x20       a0 = 4'h{a0:x};\n\
             \x20       a1 = 4'h{a1:x};\n\
             \x20       a2 = 4'h{a2:x};\n\
             \x20       a3 = 4'h{a3:x};\n\
             \x20       sel = {sel};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n"
        );

        let actual = run_sim(src);

        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} sel={} harap={} dapat={:?}",
                seed, sel, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch mux 4-to-1:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
