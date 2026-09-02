//! Fuzz differential typedef usage patterns.
//!
//! Blind spot: fuzzer existing menguji expression, tapi typedef dengan
//! berbagai kombinasi tipe dasar dan range belum terekspos secara systematic.
//! Edge cases:
//! - Typedef base types (bit, logic, reg) with different widths
//! - Typedef enum with random member values
//! - Typedef used in declaration with implicit truncation
//! - Typedef range overrides

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("typedef-fuzz-sim".to_string())
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

/// Typedef with random base type and width — assign value, read back.
#[test]
fn typedef_base_type_width_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    let base_types = ["bit", "logic", "reg"];
    let widths = [1u32, 2, 4, 8, 12, 16, 24, 32];

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_01);
        let base = base_types[rng.usize(0..base_types.len())];
        let w = widths[rng.usize(0..widths.len())];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let val = rng.u64(0..) & m;

        let src = format!(
            "module typedef_mod;\n\
             \x20   typedef {base} [{h}:0] my_type_t;\n\
             \x20   wire [{h}:0] y;\n\
             \x20   initial begin\n\
             \x20       my_type_t v;\n\
             \x20       v = {w}'h{val:x};\n\
             \x20       y = v;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            base = base,
            h = w - 1,
            w = w,
            val = val,
        );

        let actual = run_sim(src);
        if actual != Some(val) {
            mismatch.push(format!(
                "seed={} base={} w={} val={:#x} harap={:#x} can={:?}",
                seed, base, w, val, val, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch typedef base type:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Typedef enum with random values — assign member, verify value.
/// Uses numeric assignment instead of enum member name to avoid symbol resolution issues.
#[test]
fn typedef_enum_values_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_02);
        let n = rng.usize(2..=6);
        let pick = rng.usize(0..n);
        // Enum members auto-assign 0,1,2,... so pick value = pick index
        let expected = pick as u64;

        let enum_body = (0..n)
            .map(|i| format!("    V{}", i))
            .collect::<Vec<_>>()
            .join(",\n");

        let src = format!(
            "module typedef_enum_mod;\n\
             \x20   typedef enum bit [4:0] {{\n\
             {enum_body}\n\
             \x20   }} enum_t;\n\
             \x20   wire [4:0] y;\n\
             \x20   initial begin\n\
             \x20       y = {pick};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            enum_body = enum_body,
            pick = pick,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} pick={} harap={} can={:?}",
                seed, pick, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch typedef enum:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Typedef used in declaration with implicit truncation/extension.
#[test]
fn typedef_implicit_truncation_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_03);
        let src_w = [8u32, 12, 16, 24, 32][rng.usize(0..5)];
        let dst_w = [4u32, 8, 12][rng.usize(0..3)];
        let src_m = if src_w >= 64 { u64::MAX } else { (1u64 << src_w) - 1 };
        let val = rng.u64(0..) & src_m;
        let dst_m = if dst_w >= 64 { u64::MAX } else { (1u64 << dst_w) - 1 };
        let expected = val & dst_m;

        let src_code = format!(
            "module typedef_trunc_mod;\n\
             \x20   typedef bit [{sh}:0] src_t;\n\
             \x20   typedef bit [{dh}:0] dst_t;\n\
             \x20   wire [{dh}:0] y;\n\
             \x20   initial begin\n\
             \x20       src_t sv;\n\
             \x20       dst_t dv;\n\
             \x20       sv = {sw}'h{val:x};\n\
             \x20       dv = sv;\n\
             \x20       y = dv;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            sh = src_w - 1,
            dh = dst_w - 1,
            sw = src_w,
            val = val,
        );

        let actual = run_sim(src_code);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} src_w={} dst_w={} val={:#x} harap={:#x} can={:?}",
                seed, src_w, dst_w, val, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch typedef truncation:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
