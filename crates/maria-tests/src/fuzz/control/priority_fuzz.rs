//! Fuzz differential unique/priority case/if — priority resolution.
//!
//! unique case: priority to first match, no overlap allowed.
//! priority if: first true branch wins.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("priority-fuzz-sim".to_string())
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

/// unique case — first match wins, all cases mutually exclusive.
#[test]
fn unique_case_basic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xDD_01);
        let sel = rng.u64(0..4); // 0..3
        let expected = match sel {
            0 => 0xA,
            1 => 0xB,
            2 => 0xC,
            _ => 0xD,
        };

        let src = format!(
            "module test;\n\
             \x20   reg [1:0] sel;\n\
             \x20   reg [3:0] y;\n\
             \x20   initial begin\n\
             \x20       sel = {sel};\n\
             \x20       unique case (sel)\n\
             \x20           2'b00: y = 4'hA;\n\
             \x20           2'b01: y = 4'hB;\n\
             \x20           2'b10: y = 4'hC;\n\
             \x20           2'b11: y = 4'hD;\n\
             \x20       endcase\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            sel = sel,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} sel={} harap={:#x} dapat={:?}",
                seed, sel, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch unique case:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// priority if — first true branch wins.
#[test]
fn priority_if_first_match() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xDD_02);
        let a: u8 = rng.u8(..);
        let b: u8 = rng.u8(..);
        let c: u8 = rng.u8(..);

        // priority if: first matching condition wins
        let expected = if a > 100 {
            1u64
        } else if b > 100 {
            2u64
        } else if c > 100 {
            3u64
        } else {
            0u64
        };

        let src = format!(
            "module test;\n\
             \x20   reg [7:0] a = {a}, b = {b}, c = {c};\n\
             \x20   reg [1:0] y;\n\
             \x20   initial begin\n\
             \x20       priority if (a > 100) y = 1;\n\
             \x20       else if (b > 100) y = 2;\n\
             \x20       else if (c > 100) y = 3;\n\
             \x20       else y = 0;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a = a,
            b = b,
            c = c,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} b={} c={} harap={} dapat={:?}",
                seed, a, b, c, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch priority if:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// unique if — first true branch, no overlap.
#[test]
fn unique_if_basic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xDD_03);
        let sel: u64 = rng.u64(0..3);

        let expected = match sel {
            0 => 0xAA,
            1 => 0xBB,
            _ => 0xCC,
        };

        let src = format!(
            "module test;\n\
             \x20   reg [1:0] sel;\n\
             \x20   reg [7:0] y;\n\
             \x20   initial begin\n\
             \x20       sel = {sel};\n\
             \x20       unique if (sel == 0) y = 8'hAA;\n\
             \x20       else if (sel == 1) y = 8'hBB;\n\
             \x20       else y = 8'hCC;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            sel = sel,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} sel={} harap={:#x} dapat={:?}",
                seed, sel, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch unique if:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// priority case — first matching item wins.
#[test]
fn priority_case_basic() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xDD_04);
        let sel = rng.u64(0..4);

        let expected = match sel {
            0 => 0x10,
            1 => 0x20,
            2 => 0x30,
            _ => 0x40,
        };

        let src = format!(
            "module test;\n\
             \x20   reg [1:0] sel;\n\
             \x20   reg [7:0] y;\n\
             \x20   initial begin\n\
             \x20       sel = {sel};\n\
             \x20       priority case (sel)\n\
             \x20           2'b00: y = 8'h10;\n\
             \x20           2'b01: y = 8'h20;\n\
             \x20           2'b10: y = 8'h30;\n\
             \x20           default: y = 8'h40;\n\
             \x20       endcase\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            sel = sel,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} sel={} harap={:#x} dapat={:?}",
                seed, sel, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch priority case:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
