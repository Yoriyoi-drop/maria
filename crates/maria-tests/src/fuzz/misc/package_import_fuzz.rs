//! Fuzz differential package import system.
//!
//! Blind spot: fuzzer existing menguji expression evaluation, tapi package
//! import dengan typedef/param random belum terekspos secara systematic.
//! Edge cases:
//! - Package dengan multiple typedefs (enum, struct, base type)
//! - Package dengan parameter/localparam
//! - `import pkg::*` vs `import pkg::item`
//! - Parameter expressions in package

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("pkg-import-fuzz-sim".to_string())
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

/// Package with typedef and import using numeric values.
#[test]
fn pkg_import_typedef_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_01);
        let w = [4u32, 8, 12, 16][rng.usize(0..4)];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let val = rng.u64(0..) & m;

        let src = format!(
            "package my_pkg;\n\
             \x20   typedef bit [{h}:0] my_type_t;\n\
             endpackage\n\
             \n\
             module pkg_typedef_mod;\n\
             \x20   import my_pkg::*;\n\
             \x20   wire [{h}:0] y;\n\
             \x20   initial begin\n\
             \x20       my_type_t v;\n\
             \x20       v = {w}'h{val:x};\n\
             \x20       y = v;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            h = w - 1,
            w = w,
            val = val,
        );

        let actual = run_sim(src);
        if actual != Some(val) {
            mismatch.push(format!(
                "seed={} w={} val={:#x} harap={:#x} dapat={:?}",
                seed, w, val, val, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch pkg import typedef:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Package with random parameter and expression using it.
#[test]
fn pkg_import_param_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xBB_02);
        let w = [4u32, 8, 12, 16][rng.usize(0..4)];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let base_val = rng.u64(0..) & m;
        let offset = rng.u32(0..w);
        let expected = base_val.wrapping_add(offset as u64) & m;

        let src = format!(
            "package my_pkg;\n\
             \x20   parameter [31:0] BASE = {base};\n\
             \x20   localparam [31:0] OFFSET = {offset};\n\
             endpackage\n\
             \n\
             module pkg_param_mod;\n\
             \x20   import my_pkg::*;\n\
             \x20   wire [{w1}:0] y;\n\
             \x20   assign y = BASE + OFFSET;\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            base = base_val,
            offset = offset,
            w1 = w - 1,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} base={} offset={} harap={:#x} can={:?}",
                seed, base_val, offset, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch pkg import param:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Named import `import pkg::item` vs wildcard `import pkg::*`.
#[test]
fn pkg_named_vs_wildcard_import_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_03);
        let val = rng.u64(0..255);
        let use_wildcard = rng.bool();

        let import_line = if use_wildcard {
            "import my_pkg::*;".to_string()
        } else {
            "import my_pkg::MY_VAL;".to_string()
        };

        let src = format!(
            "package my_pkg;\n\
             \x20   parameter [7:0] MY_VAL = {val};\n\
             endpackage\n\
             \n\
             module pkg_import_mod;\n\
             \x20   {import}\n\
             \x20   wire [7:0] y;\n\
             \x20   assign y = MY_VAL;\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            val = val,
            import = import_line,
        );

        let actual = run_sim(src);
        if actual != Some(val) {
            mismatch.push(format!(
                "seed={} wildcard={} val={} harap={} can={:?}",
                seed, use_wildcard, val, val, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch named vs wildcard:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
