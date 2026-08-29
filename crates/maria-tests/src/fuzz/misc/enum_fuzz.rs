//! Fuzz differential enum operations — increment, decrement, comparison.
//!
//! Tests:
//! - Enum increment (wrapping)
//! - Enum comparison
//! - Enum in case statement
//! - Enum in ternary

fn mask_of(w: u32) -> u64 {
    if w >= 64 { u64::MAX } else { (1u64 << w) - 1 }
}

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("enum-fuzz-sim".to_string())
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

/// Enum increment: NEXT = current + 1 (wrapping).
#[test]
fn enum_increment_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xD1_01);
        let val = rng.u64(0..4);

        let expected = (val + 1) & 0x3;

        let src = format!(
            r#"module enum_fuzz_mod;
    typedef enum logic [1:0] {{
        A = 2'd0,
        B = 2'd1,
        C = 2'd2,
        D = 2'd3
    }} state_t;
    state_t current;
    state_t next_val;
    wire [1:0] y;
    assign y = next_val;
    initial begin
        current = state_t'({val});
        case (current)
            A: next_val = B;
            B: next_val = C;
            C: next_val = D;
            D: next_val = A;
            default: next_val = A;
        endcase
        #10;
        $finish;
    end
endmodule"#,
            val = val,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} val={} harap={} dapat={:?}",
                seed, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch enum increment:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Enum comparison: compare two enum values.
#[test]
fn enum_comparison_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xD1_02);
        let a = rng.u64(0..4);
        let b = rng.u64(0..4);

        let expected = if a > b { 1u64 } else { 0 };

        let src = format!(
            r#"module enum_fuzz_mod;
    typedef enum logic [1:0] {{
        A = 2'd0,
        B = 2'd1,
        C = 2'd2,
        D = 2'd3
    }} state_t;
    state_t sa;
    state_t sb;
    wire y;
    assign y = (sa > sb);
    initial begin
        sa = state_t'({a});
        sb = state_t'({b});
        #10;
        $finish;
    end
endmodule"#,
            a = a,
            b = b,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} b={} harap={} dapat={:?}",
                seed, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch enum comparison:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Enum in case: decode enum to value.
#[test]
fn enum_case_decode_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xD1_03);
        let val = rng.u64(0..4);

        let expected = match val {
            0 => 10u64,
            1 => 20,
            2 => 30,
            3 => 40,
            _ => 0,
        };

        let src = format!(
            r#"module enum_fuzz_mod;
    typedef enum logic [1:0] {{
        A = 2'd0,
        B = 2'd1,
        C = 2'd2,
        D = 2'd3
    }} state_t;
    state_t s;
    wire [5:0] y;
    reg [5:0] decoded;
    assign y = decoded;
    initial begin
        s = state_t'({val});
        case (s)
            A: decoded = 6'd10;
            B: decoded = 6'd20;
            C: decoded = 6'd30;
            D: decoded = 6'd40;
            default: decoded = 6'd0;
        endcase
        #10;
        $finish;
    end
endmodule"#,
            val = val,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} val={} harap={} dapat={:?}",
                seed, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch enum case decode:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
