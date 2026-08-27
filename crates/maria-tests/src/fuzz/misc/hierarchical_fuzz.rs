//! Fuzz differential hierarchical references — `$root`, cross-module paths,
//! `parent.child.sig` dalam assign/initial.
//!
//! Blind spot: fuzzer existing menguji single-module, tapi hierarchical
//! references (pattern paling rentan bug di simulator) belum terekspos.
//! Edge cases:
//! - Direct hierarchical: `parent.sig`
//! - Two-level: `top.sub.sig`
//! - $root reference: `$root.top.sig`
//! - Read after write di module berbeda
//! - Hierarchical write di initial block

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("hier-fuzz-sim".to_string())
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

#[test]
fn hier_direct_ref_matches() {
    // Direct hierarchical reference: top.uut.sig
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let w = [4u32, 8, 16, 32][seed as usize % 4];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x11_11);
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let val = rng.u64(..) & m;

        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module sub_mod;\n\
             \x20   reg [{hi}:0] x;\n\
             endmodule\n\
             module top;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   sub_mod uut ();\n\
             \x20   initial begin\n\
             \x20       uut.x = {val};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             \x20   assign y = uut.x;\n\
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
        "{} mismatch direct hier ref:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn hier_two_level_ref_matches() {
    // Two-level: top.u1.u2.sig
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let w = [4u32, 8, 16][seed as usize % 3];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x22_22);
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let val = rng.u64(..) & m;

        let expected = val;

        let val_lit = format!("{}'h{:x}", w, val);
        let src = format!(
            "module leaf;\n\
             \x20   reg [{hi}:0] v;\n\
             endmodule\n\
             module mid;\n\
             \x20   leaf u ();\n\
             endmodule\n\
             module top;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   mid u1 ();\n\
             \x20   initial begin\n\
             \x20       u1.u.v = {val};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             \x20   assign y = u1.u.v;\n\
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
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch two-level hier ref:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn hier_read_after_write_matches() {
    // Write in submodule initial, read in top assign.
    // Tests event ordering across module boundaries.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let w = [4u32, 8, 16][seed as usize % 3];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x33_33);
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let val1 = rng.u64(..) & m;
        let val2 = rng.u64(..) & m;

        // Initial block writes val1 then val2; assign y = uut.x reads final val2
        let expected = val2;

        let v1_lit = format!("{}'h{:x}", w, val1);
        let v2_lit = format!("{}'h{:x}", w, val2);
        let src = format!(
            "module sub;\n\
             \x20   reg [{hi}:0] x;\n\
             \x20   initial begin\n\
             \x20       x = {v1};\n\
             \x20       x = {v2};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n\
             module top;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   sub uut ();\n\
             \x20   assign y = uut.x;\n\
             endmodule\n",
            hi = w - 1,
            v1 = v1_lit,
            v2 = v2_lit,
        );

        let actual = run_sim(src);

        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} v1={:#x} v2={:#x} harap={:#x} dapat={:?}",
                seed, w, val1, val2, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch hier read-after-write:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
