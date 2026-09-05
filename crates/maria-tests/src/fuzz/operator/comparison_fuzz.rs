//! Fuzz differential comparison operators with edge cases.

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
        .name("comparison-fuzz-sim".to_string())
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
fn equality_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA2_01);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let expected = if a == b { 1u64 } else { 0 };
        let src = format!(
            r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    wire y;
    assign y = (a == b);
    initial begin
        a = {av}; b = {bv};
        #1; $finish;
    end
endmodule"#,
            hi = w - 1,
            av = lit_sv(a, w),
            bv = lit_sv(b, w),
        );
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={} dapat={:?}",
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
fn inequality_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA2_02);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let expected = if a != b { 1u64 } else { 0 };
        let src = format!(
            r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    wire y;
    assign y = (a != b);
    initial begin
        a = {av}; b = {bv};
        #1; $finish;
    end
endmodule"#,
            hi = w - 1,
            av = lit_sv(a, w),
            bv = lit_sv(b, w),
        );
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={} dapat={:?}",
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
fn less_than_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA2_03);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let expected = if a < b { 1u64 } else { 0 };
        let src = format!(
            r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    wire y;
    assign y = (a < b);
    initial begin
        a = {av}; b = {bv};
        #1; $finish;
    end
endmodule"#,
            hi = w - 1,
            av = lit_sv(a, w),
            bv = lit_sv(b, w),
        );
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={} dapat={:?}",
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
fn greater_equal_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA2_04);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let expected = if a >= b { 1u64 } else { 0 };
        let src = format!(
            r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    wire y;
    assign y = (a >= b);
    initial begin
        a = {av}; b = {bv};
        #1; $finish;
    end
endmodule"#,
            hi = w - 1,
            av = lit_sv(a, w),
            bv = lit_sv(b, w),
        );
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={} dapat={:?}",
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
fn chained_comparison_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA2_05);
        let w = [8u32, 16][seed as usize % 2];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let c = rng.u64(..) & m;
        let expected = if a < b && b < c { 1u64 } else { 0 };
        let src = format!(
            r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    reg [{hi}:0] c;
    wire y;
    assign y = (a < b) && (b < c);
    initial begin
        a = {av}; b = {bv}; c = {cv};
        #1; $finish;
    end
endmodule"#,
            hi = w - 1,
            av = lit_sv(a, w),
            bv = lit_sv(b, w),
            cv = lit_sv(c, w),
        );
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} c={:#x} harap={} dapat={:?}",
                seed, w, a, b, c, expected, actual
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
fn comparison_edge_cases_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA2_06);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = mask_of(w);
        let pairs: Vec<(u64, u64)> = vec![
            (0, 0),
            (m, m),
            (0, m),
            (1, 0),
            (0, 1),
            (m - 1, m),
            (m, m - 1),
        ];
        for (a, b) in &pairs {
            let expected_eq = if *a == *b { 1u64 } else { 0 };
            let src = format!(
                r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    wire y;
    assign y = (a == b);
    initial begin
        a = {av}; b = {bv};
        #1; $finish;
    end
endmodule"#,
                hi = w - 1,
                av = lit_sv(*a, w),
                bv = lit_sv(*b, w),
            );
            let actual = run_sim(src);
            if actual != Some(expected_eq) {
                mismatch.push(format!(
                    "seed={} w={} == a={:#x} b={:#x} harap={} dapat={:?}",
                    seed, w, a, b, expected_eq, actual
                ));
            }

            let expected_lt = if *a < *b { 1u64 } else { 0 };
            let src = format!(
                r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    wire y;
    assign y = (a < b);
    initial begin
        a = {av}; b = {bv};
        #1; $finish;
    end
endmodule"#,
                hi = w - 1,
                av = lit_sv(*a, w),
                bv = lit_sv(*b, w),
            );
            let actual = run_sim(src);
            if actual != Some(expected_lt) {
                mismatch.push(format!(
                    "seed={} w={} < a={:#x} b={:#x} harap={} dapat={:?}",
                    seed, w, a, b, expected_lt, actual
                ));
            }

            let expected_gt = if *a > *b { 1u64 } else { 0 };
            let src = format!(
                r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    wire y;
    assign y = (a > b);
    initial begin
        a = {av}; b = {bv};
        #1; $finish;
    end
endmodule"#,
                hi = w - 1,
                av = lit_sv(*a, w),
                bv = lit_sv(*b, w),
            );
            let actual = run_sim(src);
            if actual != Some(expected_gt) {
                mismatch.push(format!(
                    "seed={} w={} > a={:#x} b={:#x} harap={} dapat={:?}",
                    seed, w, a, b, expected_gt, actual
                ));
            }
        }
        checked += 1;
    }
    assert!(checked > 30);
    assert!(
        mismatch.is_empty(),
        "{} mismatch:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
