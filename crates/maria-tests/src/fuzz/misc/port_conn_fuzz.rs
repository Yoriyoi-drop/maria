//! Fuzz differential cross-module port connections.
//!
//! Tests: named port connection, positional port connection, implicit port.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("port-conn-fuzz-sim".to_string())
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

/// Named port connection: .port_name(signal).
#[test]
fn named_port_connection() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xB1_01);
        let w = [4u32, 8, 16, 32][seed as usize % 4];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let val = rng.u64(..) & m;
        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module child (input [{hi}:0] a, output [{hi}:0] y);\n\
             \x20   assign y = a;\n\
             endmodule\n\
             \n\
             module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   child inst (.a({val}), .y(y));\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
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
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch named port:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Positional port connection: inst(signal).
#[test]
fn positional_port_connection() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xB1_02);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let val = rng.u64(..) & m;
        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module child (input [{hi}:0] a, output [{hi}:0] y);\n\
             \x20   assign y = a;\n\
             endmodule\n\
             \n\
             module test;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   child inst ({val}, y);\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
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
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch positional port:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Multiple ports with different widths.
#[test]
fn multi_port_widths() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xB1_03);
        let w1 = [4u32, 8][seed as usize % 2];
        let w2 = [8u32, 16][seed as usize % 2];
        let m1 = if w1 >= 64 { u64::MAX } else { (1u64 << w1) - 1 };
        let m2 = if w2 >= 64 { u64::MAX } else { (1u64 << w2) - 1 };

        let a = rng.u64(..) & m1;
        let b = rng.u64(..) & m2;
        // child computes a + b, result width = max(w1, w2)
        let expected = (a.wrapping_add(b)) & m2;

        let a_lit = format!("{}'h{:x}", w1, a);
        let b_lit = format!("{}'h{:x}", w2, b);

        let src = format!(
            "module child (input [{w1a}:0] a, input [{w2a}:0] b, output [{w2a}:0] y);\n\
             \x20   assign y = a + b;\n\
             endmodule\n\
             \n\
             module test;\n\
             \x20   wire [{w2a}:0] y;\n\
             \x20   child inst (.a({a_val}), .b({b_val}), .y(y));\n\
             \x20   initial begin #1; $finish; end\n\
             endmodule\n",
            w1a = w1 - 1,
            w2a = w2 - 1,
            a_val = a_lit,
            b_val = b_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w1={} w2={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w1, w2, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch multi port widths:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
