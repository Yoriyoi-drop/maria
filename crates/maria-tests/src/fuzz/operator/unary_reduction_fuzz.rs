//! Fuzz differential unary reduction operators — `&a`, `|a`, `^a`, `~&a`, `~|a`, `~^a`.

fn mask_of(w: u32) -> u64 {
    if w >= 64 {
        u64::MAX
    } else {
        (1u64 << w) - 1
    }
}

fn lit_sv(v: u64, w: u32) -> String {
    format!("{}'h{:x}", w, v & mask_of(w))
}

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("unary-red-sim".to_string())
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

/// Reduction AND: `&a` — 1 iff all bits are 1.
#[test]
fn reduction_and_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA1_01);
        let w = [4u32, 8, 16, 32][seed as usize % 4];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let expected = if a == m { 1u64 } else { 0 };
        let src = format!(
            "module test;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire y;\n\
             \x20   assign y = &a;\n\
             \x20   initial begin\n\
             \x20       a = {av};\n\
             \x20       #1; $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            av = lit_sv(a, w),
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
    assert!(checked > 40);
    assert!(
        mismatch.is_empty(),
        "{} mismatch:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Reduction OR: `|a` — 1 iff at least one bit is 1.
#[test]
fn reduction_or_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA1_02);
        let w = [4u32, 8, 16, 32][seed as usize % 4];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let expected = if a != 0 { 1u64 } else { 0 };
        let src = format!(
            "module test;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire y;\n\
             \x20   assign y = |a;\n\
             \x20   initial begin\n\
             \x20       a = {av};\n\
             \x20       #1; $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            av = lit_sv(a, w),
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
    assert!(checked > 40);
    assert!(
        mismatch.is_empty(),
        "{} mismatch:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Reduction XOR: `^a` — 1 iff odd number of 1-bits.
#[test]
fn reduction_xor_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA1_03);
        let w = [4u32, 8, 16, 32][seed as usize % 4];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let expected = if a.count_ones() % 2 == 1 { 1u64 } else { 0 };
        let src = format!(
            "module test;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire y;\n\
             \x20   assign y = ^a;\n\
             \x20   initial begin\n\
             \x20       a = {av};\n\
             \x20       #1; $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            av = lit_sv(a, w),
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
    assert!(checked > 40);
    assert!(
        mismatch.is_empty(),
        "{} mismatch:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// NAND reduction: `~&a`
#[test]
fn reduction_nand_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA1_04);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let expected = if a == m { 0u64 } else { 1 };
        let src = format!(
            "module test;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire y;\n\
             \x20   assign y = ~&a;\n\
             \x20   initial begin\n\
             \x20       a = {av};\n\
             \x20       #1; $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            av = lit_sv(a, w),
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
    assert!(checked > 40);
    assert!(
        mismatch.is_empty(),
        "{} mismatch:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// NOR reduction: `~|a`
#[test]
fn reduction_nor_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA1_05);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let expected = if a == 0 { 1u64 } else { 0 };
        let src = format!(
            "module test;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire y;\n\
             \x20   assign y = ~|a;\n\
             \x20   initial begin\n\
             \x20       a = {av};\n\
             \x20       #1; $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            av = lit_sv(a, w),
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
    assert!(checked > 40);
    assert!(
        mismatch.is_empty(),
        "{} mismatch:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// XNOR reduction: `~^a`
#[test]
fn reduction_xnor_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA1_06);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let expected = if a.count_ones() % 2 == 0 { 1u64 } else { 0 };
        let src = format!(
            "module test;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   wire y;\n\
             \x20   assign y = ~^a;\n\
             \x20   initial begin\n\
             \x20       a = {av};\n\
             \x20       #1; $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            av = lit_sv(a, w),
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
    assert!(checked > 40);
    assert!(
        mismatch.is_empty(),
        "{} mismatch:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
