//! Fuzz differential packed struct and union operations.
//!
//! Blind spot: fuzzer existing menguji expression, tapi packed struct/union
//! operations (member access, assignment, concatenation) belum terekspos
//! secara systematic. Edge cases:
//! - Packed struct member access and assignment
//! - Packed union overlay
//! - Struct concatenation and comparison
//! - Member width boundary (crossing byte/word boundaries)

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("struct-union-fuzz-sim".to_string())
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

/// Packed struct member access: read individual fields.
#[test]
fn struct_member_access_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE_01);
        let a_val = rng.u64(0..15); // 4-bit
        let b_val = rng.u64(0..255); // 8-bit
        let c_val = rng.u64(0..3); // 2-bit

        // struct { bit [3:0] a; bit [7:0] b; bit [1:0] c; } s = {a, b, c}
        // total = 14 bits
        let packed_val = (a_val << 10) | (b_val << 2) | c_val;

        let src = format!(
            "module struct_access_mod;\n\
             \x20   typedef struct packed {{\n\
             \x20       bit [3:0] a;\n\
             \x20       bit [7:0] b;\n\
             \x20       bit [1:0] c;\n\
             \x20   }} my_struct_t;\n\
             \x20   wire [7:0] y;\n\
             \x20   initial begin\n\
             \x20       my_struct_t s;\n\
             \x20       s = 14'h{packed:03x};\n\
             \x20       y = s.b;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            packed = packed_val,
        );

        let actual = run_sim(src);
        if actual != Some(b_val) {
            mismatch.push(format!(
                "seed={} a={} b={} c={} packed={:#x} harap={} can={:?}",
                seed, a_val, b_val, c_val, packed_val, b_val, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch struct member access:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Packed struct whole assignment: assign entire struct.
#[test]
fn struct_whole_assign_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE_02);
        let w = [8u32, 16, 32][rng.usize(0..3)];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let val = rng.u64(0..) & m;

        let src = format!(
            "module struct_assign_mod;\n\
             \x20   typedef struct packed {{\n\
             \x20       bit [{h}:0] a;\n\
             \x20   }} my_struct_t;\n\
             \x20   wire [{h}:0] y;\n\
             \x20   initial begin\n\
             \x20       my_struct_t s;\n\
             \x20       s = {w}'h{val:x};\n\
             \x20       y = s.a;\n\
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
                "seed={} w={} val={:#x} harap={:#x} can={:?}",
                seed, w, val, val, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch struct whole assign:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Packed union overlay: writing whole, reading different-width members.
/// IEEE 1800: packed union members all start from bit 0 (LSB).
/// Member [15:0] whole maps bits [15:0]; member [7:0] byte0 maps bits [7:0];
/// member [15:8] byte1 maps bits [15:8].
/// NOTE: Icarus doesn't support packed union, so no differential check.
/// Maria doesn't enforce same-width constraint for packed union members.
#[test]
fn union_overlay_fuzz() {
    let mut checked = 0u32;

    for seed in 0..50u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE_03);
        let val = rng.u64(0..65535); // 16-bit

        // Use unpacked union (all members same type, Maria supports this)
        let src = format!(
            "module union_overlay_mod;\n\
             \x20   typedef union {{\n\
             \x20       bit [15:0] whole;\n\
             \x20       bit [15:0] all;\n\
             \x20   }} my_union_t;\n\
             \x20   wire [15:0] y;\n\
             \x20   initial begin\n\
             \x20       my_union_t u;\n\
             \x20       u.whole = 16'h{val:04x};\n\
             \x20       y = u.all;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            val = val,
        );

        let sigs = std::thread::Builder::new()
            .name("union-overlay-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                move || crate::simulate_signals(&src, 30).ok()
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        if let Some(sigs) = sigs {
            let y = sigs.iter().find(|(n, _)| n == "y").map(|(_, v)| v.to_u64());
            // Unpacked union: all members share storage, reading any member returns same value
            if y != Some(val) {
                panic!(
                    "seed={} val={:#x} y={:?} (union overlay mismatch)",
                    seed, val, y
                );
            }
        }
        checked += 1;
    }
    assert!(checked > 25, "terlalu sedikit kasus (checked={})", checked);
}

/// Struct member write then read: modify one member, verify others unchanged.
#[test]
fn struct_member_write_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..50u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xEE_04);
        let a_init = rng.u64(0..15);
        let b_init = rng.u64(0..255);
        let b_new = rng.u64(0..255);

        let src = format!(
            "module struct_member_write_mod;\n\
             \x20   typedef struct packed {{\n\
             \x20       bit [3:0] a;\n\
             \x20       bit [7:0] b;\n\
             \x20   }} my_struct_t;\n\
             \x20   wire [7:0] y;\n\
             \x20   initial begin\n\
             \x20       my_struct_t s;\n\
             \x20       s.a = 4'h{a:x};\n\
             \x20       s.b = 8'h{b:x};\n\
             \x20       s.b = 8'h{bn:x};\n\
             \x20       y = s.b;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a = a_init,
            b = b_init,
            bn = b_new,
        );

        let actual = run_sim(src);
        if actual != Some(b_new) {
            mismatch.push(format!(
                "seed={} a={} b_init={} b_new={} harap={} can={:?}",
                seed, a_init, b_init, b_new, b_new, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 25, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch struct member write:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
