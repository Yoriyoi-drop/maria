//! Fuzz differential for loop iteration with various bounds and update patterns.
//!
//! Blind spot: fuzzer existing menguji expression, tapi for loop dengan
//! batas acak, step acak, dan body kompleks belum terekspos secara systematic.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("for-loop-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn({
            move || {
                crate::simulate_signals(&src, 100).ok().and_then(|sigs| {
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

/// For loop sum: sum of 0..N-1 = N*(N-1)/2
#[test]
fn for_loop_sum_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_01);
        let n = rng.u32(1..=16);
        let expected = (n * (n - 1) / 2) as u64;

        let src = format!(
            "module for_sum_mod;\n\
             \x20   reg [15:0] y;\n\
             \x20   integer i;\n\
             \x20   initial begin\n\
             \x20       y = 0;\n\
             \x20       for (i = 0; i < {n}; i = i + 1)\n\
             \x20           y = y + i;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            n = n,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} n={} harap={} can={:?}",
                seed, n, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch for loop sum:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// For loop with step 2: sum of even numbers 0,2,4,...,2*(N-1) = N*(N-1)
#[test]
fn for_loop_step2_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_02);
        let n = rng.u32(1..=8);
        let expected = (n * (n - 1)) as u64;

        let src = format!(
            "module for_step2_mod;\n\
             \x20   reg [15:0] y;\n\
             \x20   integer i;\n\
             \x20   initial begin\n\
             \x20       y = 0;\n\
             \x20       for (i = 0; i < {n}; i = i + 2)\n\
             \x20           y = y + i;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            n = n * 2,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} n={} harap={} can={:?}",
                seed, n, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch for step2:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// For loop nested: sum of i*j for i in 0..N, j in 0..M
#[test]
fn for_loop_nested_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_03);
        let n = rng.u32(1..=5);
        let m = rng.u32(1..=5);
        let expected = (n * (n - 1) / 2) * m * (m - 1) / 2;

        let src = format!(
            "module for_nested_mod;\n\
             \x20   reg [15:0] y;\n\
             \x20   integer i, j;\n\
             \x20   initial begin\n\
             \x20       y = 0;\n\
             \x20       for (i = 0; i < {n}; i = i + 1)\n\
             \x20           for (j = 0; j < {m}; j = j + 1)\n\
             \x20               y = y + 1;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            n = n,
            m = m,
        );

        let actual = run_sim(src);
        let expected_count: u64 = (n * m) as u64;
        if actual != Some(expected_count) {
            mismatch.push(format!(
                "seed={} n={} m={} harap={} can={:?}",
                seed, n, m, expected_count, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch for nested:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// For loop with break: sum until break condition
#[test]
fn for_loop_break_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_04);
        let threshold = rng.u32(2..=10);
        let max_iter = 20u32;
        let mut expected = 0u64;
        for i in 0..max_iter {
            if i >= threshold {
                break;
            }
            expected += i as u64;
        }

        let src = format!(
            "module for_break_mod;\n\
             \x20   reg [15:0] y;\n\
             \x20   integer i;\n\
             \x20   initial begin\n\
             \x20       y = 0;\n\
             \x20       for (i = 0; i < 20; i = i + 1) begin\n\
             \x20           if (i == {threshold}) break;\n\
             \x20           y = y + i;\n\
             \x20       end\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            threshold = threshold,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} threshold={} harap={} can={:?}",
                seed, threshold, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch for break:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// For loop with continue: skip odd numbers
#[test]
fn for_loop_continue_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFF_05);
        let n = rng.u32(2..=20);
        let expected: u64 = (0..n).filter(|i| i % 2 == 0).map(|i| i as u64).sum();

        let src = format!(
            "module for_continue_mod;\n\
             \x20   reg [15:0] y;\n\
             \x20   integer i;\n\
             \x20   initial begin\n\
             \x20       y = 0;\n\
             \x20       for (i = 0; i < {n}; i = i + 1) begin\n\
             \x20           if (i % 2 != 0) continue;\n\
             \x20           y = y + i;\n\
             \x20       end\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            n = n,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} n={} harap={} can={:?}",
                seed, n, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch for continue:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
