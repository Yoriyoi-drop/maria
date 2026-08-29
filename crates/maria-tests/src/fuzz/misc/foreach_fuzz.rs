//! Fuzz differential foreach loop over unpacked arrays.
//!
//! Tests: foreach sum, foreach with index, foreach on different array sizes.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("foreach-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn({
            move || {
                crate::simulate_signals(&src, 50)
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

/// foreach sum: sum all elements of an unpacked array.
#[test]
fn foreach_sum() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xE3_01);
        let n = 4usize;
        let mut vals = [0u64; 4];
        for i in 0..n {
            vals[i] = rng.u64(0..16);
        }
        let expected = vals.iter().sum::<u64>();

        let vals_str: Vec<String> = vals.iter().map(|v| format!("4'h{:x}", v)).collect();
        let init_str = vals_str.join(", ");

        let src = format!(
            "module test;\n\
             \x20   reg [3:0] arr [0:{n}];\n\
             \x20   reg [7:0] y;\n\
             \x20   integer i;\n\
             \x20   initial begin\n\
             \x20       arr = '{{{init}}};\n\
             \x20       y = 0;\n\
             \x20       foreach (arr[i]) y = y + arr[i];\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            n = n - 1,
            init = init_str,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} vals={:?} harap={} dapat={:?}",
                seed, vals, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch foreach sum:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// foreach with index: compute weighted sum using individual assignments.
/// NOTE: Maria's array literal `'{...}'` assigns in reverse order —
/// use explicit arr[i] = val assignments instead.
#[test]
fn foreach_weighted() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xE3_02);
        let n = 4usize;
        let mut vals = [0u64; 4];
        for i in 0..n {
            vals[i] = rng.u64(0..4); // smaller values to avoid overflow
        }
        // Weighted sum: vals[0]*1 + vals[1]*2 + vals[2]*3 + vals[3]*4
        // Max = 3*1 + 3*2 + 3*3 + 3*4 = 30 → fits in 8 bits
        let expected: u64 = vals.iter().enumerate().map(|(i, v)| v * (i as u64 + 1)).sum();

        // Use explicit assignments to avoid array literal reverse-order bug
        let assigns: Vec<String> = vals.iter().enumerate()
            .map(|(i, v)| format!("arr[{}] = 4'h{:x};", i, v))
            .collect();
        let assign_str = assigns.join(" ");

        let src = format!(
            "module test;\n\
             \x20   reg [3:0] arr [0:{n}];\n\
             \x20   reg [7:0] y;\n\
             \x20   integer i;\n\
             \x20   initial begin\n\
             \x20       {assigns}\n\
             \x20       y = 0;\n\
             \x20       foreach (arr[i]) y = y + arr[i] * (i + 1);\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            n = n - 1,
            assigns = assign_str,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} vals={:?} harap={} dapat={:?}",
                seed, vals, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch foreach weighted:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
