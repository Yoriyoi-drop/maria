//! Fuzz differential always_comb with random combinational logic.
//!
//! Tests: random combinational expressions, priority cases, if-else chains.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("always-comb-fuzz-sim".to_string())
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

/// Always comb with random mux: y = sel ? a : b
#[test]
fn always_comb_mux_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_01);
        let a = rng.u64(0..255);
        let b = rng.u64(0..255);
        let sel = rng.u64(0..1);

        let src = format!(
            "module mux_mod;\n\
             \x20   reg [7:0] a, b;\n\
             \x20   reg sel;\n\
             \x20   wire [7:0] y;\n\
             \x20   always @(*) begin\n\
             \x20       if (sel)\n\
             \x20           y = a;\n\
             \x20       else\n\
             \x20           y = b;\n\
             \x20   end\n\
             \x20   initial begin\n\
             \x20       a = 8'h{a:02x};\n\
             \x20       b = 8'h{b:02x};\n\
             \x20       sel = 8'd{sel};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a = a,
            b = b,
            sel = sel,
        );

        let expected = if sel != 0 { a } else { b };
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} b={} sel={} harap={} can={:?}",
                seed, a, b, sel, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch always comb mux:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Always comb with priority case: casez with wildcard patterns.
/// Expected values per sel:
///   0 (2'b00) → 2 (matches 2'b00)
///   1 (2'b01) → 1 (matches 2'b01)
///   2 (2'b10) → 0 (matches 2'b1?)
///   3 (2'b11) → 0 (matches 2'b1?)
#[test]
fn always_comb_priority_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_02);
        let sel = rng.u32(0..4);
        let expected: u64 = match sel {
            0 => 2,
            1 => 1,
            2 => 0,
            3 => 0,
            _ => unreachable!(),
        };

        let src = format!(
            "module priority_mod;\n\
             \x20   reg [1:0] sel;\n\
             \x20   reg [7:0] y;\n\
             \x20   always @(*) begin\n\
             \x20       casez (sel)\n\
             \x20           2'b1?: y = 0;\n\
             \x20           2'b01: y = 1;\n\
             \x20           2'b00: y = 2;\n\
             \x20           default: y = 3;\n\
             \x20       endcase\n\
             \x20   end\n\
             \x20   initial begin\n\
             \x20       sel = 2'd{sel};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            sel = sel,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} sel={} harap={} can={:?}",
                seed, sel, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch priority case:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Always comb with multiple outputs driven by same sensitivity
#[test]
fn always_comb_multi_out_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_03);
        let a = rng.u64(0..15);
        let b = rng.u64(0..15);

        let src = format!(
            "module multi_out_mod;\n\
             \x20   reg [3:0] a, b;\n\
             \x20   wire [3:0] y_sum, y_diff;\n\
             \x20   always @(*) begin\n\
             \x20       y_sum = a + b;\n\
             \x20       y_diff = a - b;\n\
             \x20   end\n\
             \x20   wire [7:0] y;\n\
             \x20   assign y = y_sum + y_diff;\n\
             \x20   initial begin\n\
             \x20       a = 4'h{a:x};\n\
             \x20       b = 4'h{b:x};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a = a,
            b = b,
        );

        // a+b and a-b are 4-bit unsigned, then summed as 8-bit unsigned
        let sum4 = (a + b) & 0xF;
        let diff4 = (a.wrapping_sub(b)) & 0xF;
        let expected = (sum4 + diff4) & 0xFF;
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} b={} harap={} can={:?}",
                seed, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch multi out:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
