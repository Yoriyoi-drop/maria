//! Fuzz differential multi-module interaction with parameter passing.
//!
//! Tests: multiple modules with parameterized widths, connected via wires.
//! Blind spot: fuzzer existing menguji expression tunggal, tapi interaksi
//! antar module dengan parameter berbeda belum terekspos.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("multi-mod-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn({
            move || {
                crate::simulate_signals(&src, 50).ok().and_then(|sigs| {
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

/// Two modules chained: module A outputs, module B reads and inverts.
#[test]
fn multi_module_chain_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_01);
        let val = rng.u64(0..255);

        let src = format!(
            "module passthrough #(parameter [7:0] V = 0) (output [7:0] y);\n\
             \x20   assign y = V;\n\
             endmodule\n\
             \n\
             module inverter (input [7:0] a, output [7:0] y);\n\
             \x20   assign y = ~a;\n\
             endmodule\n\
             \n\
             module chain_mod;\n\
             \x20   wire [7:0] mid, y;\n\
             \x20   passthrough #(.V(8'h{val:02x})) src (.y(mid));\n\
             \x20   inverter inv (.a(mid), .y(y));\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            val = val,
        );

        let expected = !val & 0xFF;
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} val={:#x} harap={:#x} can={:?}",
                seed, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch multi module chain:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Three modules in pipeline: add → multiply → mask.
#[test]
fn multi_module_pipeline_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_02);
        let a = rng.u64(0..15);
        let b = rng.u64(0..15);
        let c = rng.u64(1..5);

        let src = format!(
            "module adder(input [3:0] a, b, output [7:0] y);\n\
             \x20   assign y = a + b;\n\
             endmodule\n\
             \n\
             module multiplier(input [7:0] a, input [3:0] b, output [11:0] y);\n\
             \x20   assign y = a * b;\n\
             endmodule\n\
             \n\
             module pipeline_mod;\n\
             \x20   wire [7:0] sum;\n\
             \x20   wire [11:0] prod;\n\
             \x20   wire [7:0] y;\n\
             \x20   adder u1 (.a(4'h{a:x}), .b(4'h{b:x}), .y(sum));\n\
             \x20   multiplier u2 (.a(sum), .b(4'h{c:x}), .y(prod));\n\
             \x20   assign y = prod[7:0];\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a = a,
            b = b,
            c = c,
        );

        let expected = ((a + b) * c) & 0xFF;
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} b={} c={} harap={} can={:?}",
                seed, a, b, c, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch pipeline:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Module with wire-AND (wand) — multiple drivers.
#[test]
fn multi_module_wand_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_03);
        let a = rng.u64(0..255);
        let b = rng.u64(0..255);

        let src = format!(
            "module wand_mod;\n\
             \x20   wire [7:0] y;\n\
             \x20   assign y = 8'h{a:02x};\n\
             \x20   assign y = 8'h{b:02x};\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a = a,
            b = b,
        );

        let expected = a & b;
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={:#x} b={:#x} harap={:#x} can={:?}",
                seed, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 15, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch wand:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
