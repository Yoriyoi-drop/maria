//! Fuzz differential parenthesization — verify redundant parentheses don't
//! change semantics.

fn mask_of(w: u32) -> u64 {
    if w >= 64 { u64::MAX } else { (1u64 << w) - 1 }
}

fn lit_sv(v: u64, w: u32) -> String {
    format!("{}'h{:x}", w, v & mask_of(w))
}

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("paren-fuzz-sim".to_string())
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
fn redundant_parens_addition() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xE1_01);
        let w = [8u32, 16][seed as usize % 2];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let expected = (a + b) & m;
        let a_lit = lit_sv(a, w);
        let b_lit = lit_sv(b, w);

        for (i, expr) in [
            format!("{} + {}", a_lit, b_lit),
            format!("({} + {})", a_lit, b_lit),
            format!("(({} + {}))", a_lit, b_lit),
        ].iter().enumerate() {
            let src = format!(
                r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    wire [{hi}:0] y;
    assign y = {expr};
    initial begin
        a = {av};
        b = {bv};
        #1; $finish;
    end
endmodule
"#,
                hi = w - 1, expr = expr, av = a_lit, bv = b_lit,
            );
            let actual = run_sim(src);
            if actual != Some(expected) {
                mismatch.push(format!("seed={} w={} form={} a={:#x} b={:#x} harap={:#x} dapat={:?}", seed, w, i, a, b, expected, actual));
            }
        }
        checked += 1;
    }
    assert!(checked > 40);
    assert!(mismatch.is_empty(), "{} mismatch:\n{}", mismatch.len(), mismatch.join("\n"));
}

#[test]
fn parens_multiply_add() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xE1_02);
        let w = [8u32, 16][seed as usize % 2];
        if w > 16 { continue; }
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let c = rng.u64(..) & m;
        let a_lit = lit_sv(a, w);
        let b_lit = lit_sv(b, w);
        let c_lit = lit_sv(c, w);

        let expected_mul_add = ((a * b) + c) & m;
        for (i, expr) in [
            format!("{} * {} + {}", a_lit, b_lit, c_lit),
            format!("({} * {}) + {}", a_lit, b_lit, c_lit),
        ].iter().enumerate() {
            let src = format!(
                r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    reg [{hi}:0] c;
    wire [{hi}:0] y;
    assign y = {expr};
    initial begin
        a = {av}; b = {bv}; c = {cv};
        #1; $finish;
    end
endmodule
"#,
                hi = w - 1, expr = expr, av = a_lit, bv = b_lit, cv = c_lit,
            );
            let actual = run_sim(src);
            if actual != Some(expected_mul_add) {
                mismatch.push(format!("seed={} w={} form={} harap={:#x} dapat={:?}", seed, w, i, expected_mul_add, actual));
            }
        }

        let expected_mul_of_add = (a * ((b + c) & m)) & m;
        let expr = format!("{} * ({} + {})", a_lit, b_lit, c_lit);
        let src = format!(
            r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    reg [{hi}:0] c;
    wire [{hi}:0] y;
    assign y = {expr};
    initial begin
        a = {av}; b = {bv}; c = {cv};
        #1; $finish;
    end
endmodule
"#,
            hi = w - 1, expr = expr, av = a_lit, bv = b_lit, cv = c_lit,
        );
        let actual = run_sim(src);
        if actual != Some(expected_mul_of_add) {
            mismatch.push(format!("seed={} w={} form=mul_of_add harap={:#x} dapat={:?}", seed, w, expected_mul_of_add, actual));
        }
        checked += 1;
    }
    assert!(checked > 40);
    assert!(mismatch.is_empty(), "{} mismatch:\n{}", mismatch.len(), mismatch.join("\n"));
}

#[test]
fn nested_parens_depth() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xE1_03);
        let w = [8u32, 16][seed as usize % 2];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let a_lit = lit_sv(a, w);
        let b_lit = lit_sv(b, w);
        let expected = (a ^ b) & m;

        for depth in 1..=5 {
            let mut expr = format!("{} ^ {}", a_lit, b_lit);
            for _ in 0..depth {
                expr = format!("({})", expr);
            }
            let src = format!(
                r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    wire [{hi}:0] y;
    assign y = {expr};
    initial begin
        a = {av}; b = {bv};
        #1; $finish;
    end
endmodule
"#,
                hi = w - 1, expr = expr, av = a_lit, bv = b_lit,
            );
            let actual = run_sim(src);
            if actual != Some(expected) {
                mismatch.push(format!("seed={} w={} depth={} harap={:#x} dapat={:?}", seed, w, depth, expected, actual));
            }
        }
        checked += 1;
    }
    assert!(checked > 40);
    assert!(mismatch.is_empty(), "{} mismatch:\n{}", mismatch.len(), mismatch.join("\n"));
}
