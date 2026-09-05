//! Fuzz differential nested if-else chains.

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
        .name("nested-if-sim".to_string())
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
fn nested_if_4level_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF2_01);
        let a = rng.u64(0..20);
        let expected = if a < 5 {
            0u64
        } else if a < 10 {
            1
        } else if a < 15 {
            2
        } else {
            3
        };
        let src = format!(
            r#"module test;
    reg [4:0] a;
    reg [1:0] y;
    initial begin
        a = {a};
        if (a < 5)
            y = 2'd0;
        else if (a < 10)
            y = 2'd1;
        else if (a < 15)
            y = 2'd2;
        else
            y = 2'd3;
        #10;
        $finish;
    end
endmodule"#,
            a = a,
        );
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} harap={} dapat={:?}",
                seed, a, expected, actual
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

#[test]
fn if_without_else_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF2_02);
        let w = [4u32, 8][seed as usize % 2];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let expected = if a > b { (a - b) & m } else { a };
        let src = format!(
            r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    reg [{hi}:0] y;
    initial begin
        a = {av};
        b = {bv};
        y = a;
        if (a > b) y = a - b;
        #10;
        $finish;
    end
endmodule"#,
            hi = w - 1,
            av = lit_sv(a, w),
            bv = lit_sv(b, w),
        );
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w, a, b, expected, actual
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

#[test]
fn if_else_arithmetic_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF2_03);
        let w = [8u32, 16][seed as usize % 2];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let sel = rng.bool();
        let expected = if sel {
            (a + b) & m
        } else {
            a.wrapping_sub(b) & m
        };
        let src = format!(
            r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    reg sel;
    reg [{hi}:0] y;
    initial begin
        a = {av};
        b = {bv};
        sel = {sel};
        if (sel) y = a + b;
        else y = a - b;
        #10;
        $finish;
    end
endmodule"#,
            hi = w - 1,
            av = lit_sv(a, w),
            bv = lit_sv(b, w),
            sel = if sel { 1 } else { 0 },
        );
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} sel={} harap={:#x} dapat={:?}",
                seed, w, a, b, sel, expected, actual
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

#[test]
fn if_else_bitwise_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF2_04);
        let w = [8u32, 16][seed as usize % 2];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let sel = rng.usize(0..4);
        let expected = match sel {
            0 => a & b,
            1 => a | b,
            2 => a ^ b,
            _ => !(a | b) & m,
        };
        let src = format!(
            r#"module test;
    reg [1:0] sel;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    reg [{hi}:0] y;
    initial begin
        a = {av};
        b = {bv};
        sel = {sel};
        if (sel == 0) y = a & b;
        else if (sel == 1) y = a | b;
        else if (sel == 2) y = a ^ b;
        else y = ~(a | b);
        #10;
        $finish;
    end
endmodule"#,
            hi = w - 1,
            av = lit_sv(a, w),
            bv = lit_sv(b, w),
            sel = sel,
        );
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} sel={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w, sel, a, b, expected, actual
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
