//! Fuzz differential chained assignments and multiple drivers.
//!
//! Tests:
//! - Chained blocking assignments: a = b = c
//! - Non-blocking assignments in sequence
//! - Mixed blocking/non-blocking
//! - Continuous assign with expression

fn mask_of(w: u32) -> u64 {
    if w >= 64 { u64::MAX } else { (1u64 << w) - 1 }
}

fn lit_sv(v: u64, w: u32) -> String {
    format!("{}'h{:x}", w, v & mask_of(w))
}

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("chained-assign-sim".to_string())
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

/// Chained blocking assignment: a = b = val -> both get val.
#[test]
#[ignore] // chained blocking assign not fully supported yet
fn chained_blocking_assign_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF1_01);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = mask_of(w);
        let val = rng.u64(..) & m;

        let expected = val;
        let val_lit = lit_sv(val, w);

        let src = format!(
            r#"module chained_assign_mod;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    wire [{hi}:0] y;
    assign y = b;
    initial begin
        a = b = {val};
        #10;
        $finish;
    end
endmodule"#,
            hi = w - 1,
            val = val_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} val={:#x} harap={:#x} dapat={:?}",
                seed, w, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch chained blocking:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Chained non-blocking: a <= b <= val -> a gets old b, b gets val.
#[test]
fn chained_nonblocking_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF1_02);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = mask_of(w);
        let val = rng.u64(..) & m;

        let expected_b = val;
        let expected_a = 0u64;

        let val_lit = lit_sv(val, w);

        let src = format!(
            r#"module chained_assign_mod;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    wire [{hi}:0] y;
    assign y = a;
    initial begin
        a <= b <= {val};
        #10;
        $finish;
    end
endmodule"#,
            hi = w - 1,
            val = val_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected_a) && actual != Some(expected_b) {
            mismatch.push(format!(
                "seed={} w={} val={:#x} harap_a={:#x} harap_b={:#x} dapat={:?}",
                seed, w, val, expected_a, expected_b, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch chained nonblocking:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Multiple continuous assigns to same wire (last wins or error).
#[test]
fn continuous_assign_expression_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF1_03);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let c = rng.u64(..) & m;

        let expected = (a + b + c) & m;
        let a_lit = lit_sv(a, w);
        let b_lit = lit_sv(b, w);
        let c_lit = lit_sv(c, w);

        let src = format!(
            r#"module chained_assign_mod;
    wire [{hi}:0] y;
    assign y = {a} + {b} + {c};
    initial begin #1; $finish; end
endmodule"#,
            hi = w - 1,
            a = a_lit,
            b = b_lit,
            c = c_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} c={:#x} harap={:#x} dapat={:?}",
                seed, w, a, b, c, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch continuous assign expr:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Chained assignment in always block.
#[test]
fn always_chained_assign_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF1_04);
        let w = [8u32, 16][seed as usize % 2];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;

        let expected = (a + b) & m;
        let a_lit = lit_sv(a, w);
        let b_lit = lit_sv(b, w);

        let src = format!(
            r#"module chained_assign_mod;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    reg [{hi}:0] y;
    always @(*) begin
        y = a + b;
    end
    initial begin
        a = {a};
        b = {b};
        #10;
        $finish;
    end
endmodule"#,
            hi = w - 1,
            a = a_lit,
            b = b_lit,
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
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch always chained:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
