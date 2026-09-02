//! Fuzz: comparison context-width regression + edge cases.
//! Tests that comparison operators don't inflate operand widths via assignment context.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("cmp-ctx-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn({
            move || {
                crate::simulate_signals(&src, 100)
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

/// Comparison inside shift: ternary condition + shift LHS should not inflate
#[test]
fn cmp_ctx_ternary_shift_fuzz() {
    let mut mismatch = Vec::new();
    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xBB_01);
        let cond_val = rng.u32(0..3);
        let true_val = rng.u32(0..7);
        let shift_amt = rng.u32(0..4);
        let op = rng.u32(0..4);

        // Build expression: (cond ? true_val : 0) op (~(cond_val_mask) << shift_amt)
        let cond = format!("3'b{:03b}", cond_val);
        let tv = format!("3'b{:03b}", true_val);
        let mask = format!("3'b{:03b}", cond_val);

        // Golden: manually evaluate
        let ternary_result = if cond_val != 0 { true_val as u64 } else { 0 };
        let bitnot_mask = (!cond_val) & 0x7;
        let shift_result = (bitnot_mask << shift_amt) & 0x7;
        let expected = match op {
            0 => ((ternary_result < shift_result as u64) as u64) & 0x7,
            1 => ((ternary_result > shift_result as u64) as u64) & 0x7,
            2 => ((ternary_result <= shift_result as u64) as u64) & 0x7,
            3 => ((ternary_result >= shift_result as u64) as u64) & 0x7,
            _ => unreachable!(),
        };
        let cmp_op = match op { 0 => "<", 1 => ">", 2 => "<=", 3 => ">=", _ => unreachable!() };

        let src = format!(
            "module m;\n\
             \x20 wire [2:0] y;\n\
             \x20 assign y = (({cond} ? {tv} : 3'd0) {cmp_op} (~({mask}) << {shift_amt}));\n\
             \x20 initial begin #10; $finish; end\n\
             endmodule\n",
            cond = cond, tv = tv, mask = mask, shift_amt = shift_amt, cmp_op = cmp_op,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} cond={} tv={} mask={} shamt={} op={} harap={} can={:?}",
                seed, cond_val, true_val, cond_val, shift_amt, cmp_op, expected, actual
            ));
        }
    }
    assert!(mismatch.is_empty(),
        "{} mismatch cmp_ctx_ternary_shift:\n{}", mismatch.len(), mismatch.join("\n"));
}

/// Deeply nested comparison: ternary in comparison in ternary
#[test]
fn cmp_ctx_nested_ternary_fuzz() {
    let mut mismatch = Vec::new();
    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xBB_02);
        let a = rng.u64(0..7);
        let b = rng.u64(0..7);
        let cond = rng.u32(0..3);

        // y = (cond ? (a < b) : (a > b))
        let expected = if cond != 0 {
            ((a < b) as u64) & 0x7
        } else {
            ((a > b) as u64) & 0x7
        };

        let src = format!(
            "module m;\n\
             \x20 reg [2:0] aa, bb;\n\
             \x20 wire [2:0] y;\n\
             \x20 assign y = (3'd{cond} ? (aa < bb) : (aa > bb));\n\
             \x20 initial begin\n\
             \x20     aa = 3'd{a}; bb = 3'd{b};\n\
             \x20     #10; $finish;\n\
             \x20 end\n\
             endmodule\n",
            cond = cond, a = a, b = b,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} b={} cond={} harap={} can={:?}",
                seed, a, b, cond, expected, actual
            ));
        }
    }
    assert!(mismatch.is_empty(),
        "{} mismatch cmp_ctx_nested:\n{}", mismatch.len(), mismatch.join("\n"));
}

/// BitNot in comparison RHS with wider LHS
#[test]
fn cmp_ctx_bitnot_rhs_fuzz() {
    let mut mismatch = Vec::new();
    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xBB_03);
        let a = rng.u64(0..7);
        let mask = rng.u64(0..7);

        // y = (a < (~mask))
        let bitnot = (!mask) & 0x7;
        let expected = ((a < bitnot) as u64) & 0x7;

        let src = format!(
            "module m;\n\
             \x20 reg [2:0] aa;\n\
             \x20 wire [2:0] y;\n\
             \x20 assign y = (aa < ~(3'd{mask}));\n\
             \x20 initial begin aa = 3'd{a}; #10; $finish; end\n\
             endmodule\n",
            a = a, mask = mask,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} mask={} harap={} can={:?}",
                seed, a, mask, expected, actual
            ));
        }
    }
    assert!(mismatch.is_empty(),
        "{} mismatch cmp_ctx_bitnot_rhs:\n{}", mismatch.len(), mismatch.join("\n"));
}

/// Mixed width comparison: 8-bit LHS vs 32-bit RHS (unsigned)
#[test]
fn cmp_ctx_mixed_width_fuzz() {
    let mut mismatch = Vec::new();
    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xBB_04);
        let a = rng.u8(0..255);
        let b = rng.u64(0..255);

        // y8 = (a < b), y1 = (a < b) — both should be same
        let expected = ((a as u64) < b) as u64;

        let src = format!(
            "module m;\n\
             \x20 wire [7:0] y8;\n\
             \x20 wire [0:0] y1;\n\
             \x20 assign y8 = (8'd{a} < 8'd{b});\n\
             \x20 assign y1 = (8'd{a} < 8'd{b});\n\
             \x20 initial begin #10; $finish; end\n\
             endmodule\n",
            a = a, b = b,
        );

        let (actual8, actual1) = {
            let src_c = src.clone();
            let r1 = std::thread::Builder::new().stack_size(256*1024*1024).spawn(move || {
                crate::simulate_signals(&src_c, 100).ok().and_then(|sigs| {
                    sigs.iter().find(|(n, _)| n == "y8").map(|(_, v)| v.to_u64())
                })
            }).unwrap().join().unwrap();
            let r2 = std::thread::Builder::new().stack_size(256*1024*1024).spawn(move || {
                crate::simulate_signals(&src, 100).ok().and_then(|sigs| {
                    sigs.iter().find(|(n, _)| n == "y1").map(|(_, v)| v.to_u64())
                })
            }).unwrap().join().unwrap();
            (r1, r2)
        };

        if actual8 != Some(expected) || actual1 != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} b={} expected={} y8={:?} y1={:?}",
                seed, a, b, expected, actual8, actual1
            ));
        }
    }
    assert!(mismatch.is_empty(),
        "{} mismatch cmp_ctx_mixed_width:\n{}", mismatch.len(), mismatch.join("\n"));
}
