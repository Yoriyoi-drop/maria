//! Fuzz multi-module complex hierarchies — deeper than existing multi_module_fuzz.
//!
//! Blind spot: existing fuzzer menguji module chain sederhana (2 module).
//! Ini menguji 3+ level hierarchy dengan parameter, generate, dan port width
//! interaction antar module.

use std::sync::mpsc;
use std::time::Duration;

fn with_timeout<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let _ = tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
        })
        .ok()?;
    rx.recv_timeout(Duration::from_secs(30))
        .ok()
        .and_then(|r| r.ok())
}

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("multi-mod-complex-fuzz".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            crate::simulate_signals(&src, 50)
                .ok()
                .and_then(|sigs| {
                    sigs.iter()
                        .find(|(n, _)| n == "y")
                        .map(|(_, v)| v.to_u64())
                })
        })
        .expect("spawn")
        .join()
        .expect("sim panic")
}

/// 3-level hierarchy: top → mid → leaf, each with parameters
#[test]
fn fuzz_3level_hierarchy_parameterized() {
    let mut mismatch = Vec::new();
    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE01);
        let w = [4u32, 8, 16][rng.usize(0..3)];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let v0 = rng.u64(0..) & m;
        let v1 = rng.u64(0..) & m;

        let src = format!(
            "module leaf #(parameter [7:0] V = 0)(output [{w}-1:0] y);\n\
             assign y = V[{w}-1:0];\n\
             endmodule\n\
             \n\
             module mid #(parameter [7:0] V0 = 0, parameter [7:0] V1 = 0)(output [{w}-1:0] y);\n\
             wire [{w}-1:0] l0, l1;\n\
             leaf #(.V(V0)) u0(.y(l0));\n\
             leaf #(.V(V1)) u1(.y(l1));\n\
             assign y = l0 + l1;\n\
             endmodule\n\
             \n\
             module top;\n\
             wire [{w}-1:0] y;\n\
             mid #(.V0({v0}), .V1({v1})) u(.y(y));\n\
             initial begin\n\
             \x20   #10;\n\
             \x20   $finish;\n\
             end\n\
             endmodule",
            w = w,
            v0 = v0 & 0xFF,
            v1 = v1 & 0xFF,
        );

        let expected = ((v0 & 0xFF) + (v1 & 0xFF)) & m;
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} v0={} v1={} expected={:#x} actual={:?}",
                seed, v0, v1, expected, actual
            ));
        }
    }
    assert!(mismatch.is_empty(), "{} mismatches:\n{}", mismatch.len(), mismatch.join("\n"));
}

/// Module with generate for instantiation — array of instances
#[test]
fn fuzz_generate_array_instances() {
    let mut mismatch = Vec::new();
    for seed in 0..20u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE02);
        let n = rng.usize(2..6);
        let mut expected = 0u64;

        let mut instances = String::new();
        let mut assigns = String::new();
        for i in 0..n {
            let v = rng.u64(0..255);
            expected = expected.wrapping_add(v);
            instances.push_str(&format!(
                "    wire [7:0] w{};\n",
                i
            ));
            assigns.push_str(&format!(
                "    leaf #(.V({v})) u{i}(.y(w{i}));\n",
                v = v,
                i = i,
            ));
        }
        let mut sum_parts = String::new();
        for i in 0..n {
            if i > 0 { sum_parts.push_str(" + "); }
            sum_parts.push_str(&format!("w{}", i));
        }

        let src = format!(
            "module leaf #(parameter [7:0] V = 0)(output [7:0] y);\n\
             assign y = V;\n\
             endmodule\n\
             \n\
             module top;\n\
             {instances}\n\
             wire [7:0] y;\n\
             {assigns}\n\
             assign y = {sum};\n\
             initial begin\n\
             \x20   #10;\n\
             \x20   $finish;\n\
             end\n\
             endmodule",
            instances = instances,
            assigns = assigns,
            sum = sum_parts,
        );

        let expected_masked = expected & 0xFF;
        let actual = run_sim(src);
        if actual != Some(expected_masked) {
            mismatch.push(format!(
                "seed={} n={} expected={:#x} actual={:?}",
                seed, n, expected_masked, actual
            ));
        }
    }
    assert!(mismatch.is_empty(), "{} mismatches", mismatch.len());
}

/// Module with wire-AND (wand) across instances
#[test]
fn fuzz_wand_multi_driver_across_modules() {
    let mut mismatch = Vec::new();
    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE03);
        let a_val = rng.u64(0..255);
        let b_val = rng.u64(0..255);

        let src = format!(
            "module drv_a(input [7:0] v, output [7:0] y);\n\
             assign y = v;\n\
             endmodule\n\
             \n\
             module drv_b(input [7:0] v, output [7:0] y);\n\
             assign y = v;\n\
             endmodule\n\
             \n\
             module top;\n\
             wire [7:0] wand wire_y;\n\
             drv_a u_a(.v(8'h{a:02x}), .y(wire_y));\n\
             drv_b u_b(.v(8'h{b:02x}), .y(wire_y));\n\
             wire [7:0] y;\n\
             assign y = wire_y;\n\
             initial #10 $finish;\n\
             endmodule",
            a = a_val,
            b = b_val,
        );

        let expected = a_val & b_val;
        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={:#x} b={:#x} expected={:#x} actual={:?}",
                seed, a_val, b_val, expected, actual
            ));
        }
    }
    assert!(mismatch.is_empty(), "{} mismatches", mismatch.len());
}

/// Module with port width mismatch — should handle gracefully
#[test]
fn fuzz_port_width_mismatch_across_modules() {
    for seed in 0..20u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE04);
        let narrow_w = [2u32, 4, 8][rng.usize(0..3)];
        let wide_w = narrow_w * 2;

        let src = format!(
            "module narrow(output [{narrow_w}-1:0] y);\n\
             assign y = {narrow_w}'hFF;\n\
             endmodule\n\
             \n\
             module top;\n\
             wire [{wide_w}-1:0] wide_y;\n\
             narrow u(.y(wide_y));\n\
             wire [{wide_w}-1:0] y;\n\
             assign y = wide_y;\n\
             initial #10 $finish;\n\
             endmodule",
            narrow_w = narrow_w,
            wide_w = wide_w,
        );

        // Should not panic even with width mismatch
        let result = with_timeout(move || {
            let _ = crate::compile_str(&src);
            true
        });
        assert!(result.is_some(), "port width mismatch crashed seed={}", seed);
    }
}

/// Recursive module reference — should detect and error, not loop
#[test]
fn fuzz_recursive_module_reference() {
    let src = "module a; wire y; b u(.y(y)); endmodule\n\
               module b; wire y; a u(.y(y)); endmodule";

    let result = with_timeout(move || {
        let _ = crate::compile_str(src);
        true
    });
    assert!(result.is_some(), "recursive module hung");
}
