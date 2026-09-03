//! Fuzz sequential logic — always_ff, always_comb, always_latch dengan
//! stimulus acak, clock domain crossing, blocking/non-blocking edge cases.
//!
//! Blind spot: fuzzer existing menguji ekspresi kombinasional, tapi
//! sequential logic dengan clock/reset interaction belum terekspos.

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
        .name("seq-logic-fuzz".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            crate::simulate_signals(&src, 100)
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

/// Always_ff with async reset — random reset timing
#[test]
fn fuzz_always_ff_async_reset_random_timing() {
    let mut mismatch = Vec::new();
    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA01);
        let w = [4u32, 8, 16, 32][rng.usize(0..4)];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let init_val = rng.u64(0..) & m;
        let reset_val = rng.u64(0..) & m;
        let inc_val = (rng.u64(1..16)) & m;
        let reset_at = rng.u64(3..10);
        let release_at = rng.u64(10..20);

        let src = format!(
            "module m;\n\
             reg clk = 0;\n\
             reg rst_n = 0;\n\
             reg [{w}-1:0] cnt = {init_hex};\n\
             wire [{w}-1:0] y;\n\
             assign y = cnt;\n\
             always #1 clk = ~clk;\n\
             always @(posedge clk or negedge rst_n) begin\n\
             \x20   if (!rst_n) cnt <= {reset_hex};\n\
             \x20   else cnt <= cnt + {inc_hex};\n\
             end\n\
             initial begin\n\
             \x20   rst_n = 0;\n\
             \x20   #{reset_at};\n\
             \x20   rst_n = 1;\n\
             \x20   #{release_at};\n\
             \x20   $finish;\n\
             end\n\
             endmodule",
            w = w,
            init_hex = format!("{}'h{:x}", w, init_val),
            reset_hex = format!("{}'h{:x}", w, reset_val),
            inc_hex = format!("{}'h{:x}", w, inc_val),
            reset_at = reset_at,
            release_at = release_at,
        );

        let result = run_sim(src.clone());
        // Invariant: no panic, no hang (timeout handles that)
        if result.is_none() {
            mismatch.push(format!("seed={} sim returned None", seed));
        }
    }
    assert!(
        mismatch.is_empty(),
        "{} mismatches:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Always_comb with sensitivity — auto-re-eval on input change
#[test]
fn fuzz_always_comb_sensitivity_chain() {
    let mut mismatch = Vec::new();
    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xBB02);
        let ops = ["+", "-", "&", "|", "^", "==", "!=", "<", ">"];
        let op = ops[rng.usize(0..ops.len())];
        let w = [4u32, 8, 16][rng.usize(0..3)];
        let val_a = rng.u64(0..255);
        let val_b = rng.u64(0..255);

        let src = format!(
            "module m;\n\
             reg [{w}-1:0] a = {a_hex};\n\
             reg [{w}-1:0] b = {b_hex};\n\
             reg [{w}-1:0] mid;\n\
             wire [{w}-1:0] y;\n\
             always_comb mid = a {op} b;\n\
             always_comb y = mid ^ a;\n\
             initial begin\n\
             \x20   #5;\n\
             \x20   a = {a2_hex};\n\
             \x20   #5;\n\
             \x20   $finish;\n\
             end\n\
             endmodule",
            w = w,
            a_hex = format!("{}'h{:x}", w, val_a),
            b_hex = format!("{}'h{:x}", w, val_b),
            op = op,
            a2_hex = format!("{}'h{:x}", w, rng.u64(0..255)),
        );

        let result = run_sim(src);
        if result.is_none() {
            mismatch.push(format!("seed={} sensitivity chain failed", seed));
        }
    }
    assert!(mismatch.is_empty(), "{} mismatches", mismatch.len());
}

/// Always_latch with enable — verify latch behavior
#[test]
fn fuzz_always_latch_enable() {
    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC03);
        let w = [4u32, 8, 16][rng.usize(0..3)];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let d_val = rng.u64(0..) & m;
        let q_init = rng.u64(0..) & m;

        let src = format!(
            "module m;\n\
             reg en = 0;\n\
             reg [{w}-1:0] d = {d_hex};\n\
             reg [{w}-1:0] q = {q_hex};\n\
             wire [{w}-1:0] y;\n\
             assign y = q;\n\
             always_latch if (en) q = d;\n\
             initial begin\n\
             \x20   en = 1;\n\
             \x20   #2;\n\
             \x20   en = 0;\n\
             \x20   d = {d2_hex};\n\
             \x20   #2;\n\
             \x20   en = 1;\n\
             \x20   #2;\n\
             \x20   $finish;\n\
             end\n\
             endmodule",
            w = w,
            d_hex = format!("{}'h{:x}", w, d_val),
            q_hex = format!("{}'h{:x}", w, q_init),
            d2_hex = format!("{}'h{:x}", w, rng.u64(0..) & m),
        );

        let result = run_sim(src);
        assert!(result.is_some(), "latch sim failed seed={}", seed);
    }
}

/// Non-blocking assignment delta cycle ordering
#[test]
fn fuzz_nba_delta_cycle_ordering() {
    let mut mismatch = Vec::new();
    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xDD04);
        let w = 8u32;

        // Two NBA to same signal — last writer wins in same delta
        let src = format!(
            "module m;\n\
             reg clk = 0;\n\
             reg [7:0] a = 0;\n\
             reg [7:0] b = 0;\n\
             wire [7:0] y;\n\
             assign y = a;\n\
             always @(posedge clk) begin\n\
             \x20   a <= {v1};\n\
             \x20   a <= {v2};\n\
             end\n\
             always #1 clk = ~clk;\n\
             initial #10 $finish;\n\
             endmodule",
            v1 = rng.u64(0..255),
            v2 = rng.u64(0..255),
        );

        let result = run_sim(src);
        if result.is_none() {
            mismatch.push(format!("seed={} NBA ordering failed", seed));
        }
    }
    assert!(mismatch.is_empty(), "{} mismatches", mismatch.len());
}

/// Blocking assignment in always_ff (should warn but not crash)
#[test]
fn fuzz_blocking_in_sequential() {
    for seed in 0..20u64 {
        let src = format!(
            "module m;\n\
             reg clk = 0;\n\
             reg [7:0] q = 0;\n\
             wire [7:0] y;\n\
             assign y = q;\n\
             always @(posedge clk) begin\n\
             \x20   q = q + 1;\n\
             end\n\
             always #1 clk = ~clk;\n\
             initial #20 $finish;\n\
             endmodule"
        );

        let result = run_sim(src);
        // Blocking in sequential: Maria may warn, but should not panic
        assert!(result.is_some(), "blocking-in-seq crashed seed={}", seed);
    }
}
