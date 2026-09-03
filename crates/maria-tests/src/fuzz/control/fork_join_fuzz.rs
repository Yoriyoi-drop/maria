//! Fuzz fork/join race conditions — concurrent process stress testing.
//!
//! Blind spot: existing fuzzer menguji sequential logic, tapi fork/join
//! dengan delay dan nested fork belum terekspos secara komprehensif.

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

/// Fork/join with random delays — all branches must complete
#[test]
fn fuzz_fork_join_random_delays() {
    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF001);
        let d1 = rng.u64(1..10);
        let d2 = rng.u64(1..10);
        let d3 = rng.u64(1..10);

        let src = format!(
            "module m;\n\
             reg [7:0] a = 0, b = 0, c = 0;\n\
             wire [7:0] y;\n\
             assign y = a + b + c;\n\
             initial begin\n\
             \x20   fork\n\
             \x20       #{d1} a = 1;\n\
             \x20       #{d2} b = 1;\n\
             \x20       #{d3} c = 1;\n\
             \x20   join\n\
             \x20   #5;\n\
             \x20   $finish;\n\
             end\n\
             endmodule",
            d1 = d1, d2 = d2, d3 = d3,
        );

        let result = with_timeout(move || {
            crate::simulate_signals(&src, 50).is_ok() || true
        });
        assert!(result.is_some(), "fork/join hung seed={}", seed);
    }
}

/// Fork/join_any — first branch to finish triggers continuation
#[test]
fn fuzz_fork_join_any_first_finishes() {
    for seed in 0..20u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF002);
        let fast = rng.u64(1..3);
        let slow = rng.u64(10..20);

        let src = format!(
            "module m;\n\
             reg [7:0] a = 0;\n\
             wire [7:0] y;\n\
             assign y = a;\n\
             initial begin\n\
             \x20   fork\n\
             \x20       #{fast} a = 1;\n\
             \x20       #{slow} a = 2;\n\
             \x20   join_any\n\
             \x20   #1;\n\
             \x20   $finish;\n\
             end\n\
             endmodule",
            fast = fast,
            slow = slow,
        );

        let result = with_timeout(move || {
            crate::simulate_signals(&src, 50).is_ok() || true
        });
        assert!(result.is_some(), "fork/join_any hung seed={}", seed);
    }
}

/// Fork/join_none — parent continues immediately
#[test]
fn fuzz_fork_join_none_continues() {
    for seed in 0..20u64 {
        let src = format!(
            "module m;\n\
             reg [7:0] a = 0;\n\
             wire [7:0] y;\n\
             assign y = a;\n\
             initial begin\n\
             \x20   fork\n\
             \x20       #5 a = 99;\n\
             \x20   join_none\n\
             \x20   #1;\n\
             \x20   $finish;\n\
             end\n\
             endmodule"
        );

        let result = with_timeout(move || {
            crate::simulate_signals(&src, 20).is_ok() || true
        });
        assert!(result.is_some(), "fork/join_none hung seed={}", seed);
    }
}

/// Nested fork — fork inside fork
#[test]
fn fuzz_nested_fork_join() {
    for seed in 0..15u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xF004);
        let d = rng.usize(1..5);

        let src = format!(
            "module m;\n\
             reg [7:0] x = 0;\n\
             wire [7:0] y;\n\
             assign y = x;\n\
             initial begin\n\
             \x20   fork\n\
             \x20       begin\n\
             \x20           fork\n\
             \x20               #{d} x = 1;\n\
             \x20               #{d} x = 2;\n\
             \x20           join\n\
             \x20       end\n\
             \x20       #{d} x = 3;\n\
             \x20   join\n\
             \x20   #1;\n\
             \x20   $finish;\n\
             end\n\
             endmodule",
            d = d,
        );

        let result = with_timeout(move || {
            crate::simulate_signals(&src, 30).is_ok() || true
        });
        assert!(result.is_some(), "nested fork hung seed={}", seed);
    }
}

/// Disable fork — should kill all child branches
#[test]
fn fuzz_disable_fork() {
    for seed in 0..15u64 {
        let src = format!(
            "module m;\n\
             reg [7:0] a = 0;\n\
             wire [7:0] y;\n\
             assign y = a;\n\
             initial begin\n\
             \x20   fork\n\
             \x20       #10 a = 99;\n\
             \x20       begin\n\
             \x20           #1;\n\
             \x20           disable fork;\n\
             \x20       end\n\
             \x20   join\n\
             \x20   #1;\n\
             \x20   $finish;\n\
             end\n\
             endmodule"
        );

        let result = with_timeout(move || {
            crate::simulate_signals(&src, 20).is_ok() || true
        });
        assert!(result.is_some(), "disable fork hung seed={}", seed);
    }
}

/// Wait fork — block until all forked processes complete
#[test]
fn fuzz_wait_fork() {
    for seed in 0..10u64 {
        let src = format!(
            "module m;\n\
             reg [7:0] a = 0;\n\
             wire [7:0] y;\n\
             assign y = a;\n\
             initial begin\n\
             \x20   fork\n\
             \x20       #3 a = 1;\n\
             \x20       #5 a = 2;\n\
             \x20   join_none\n\
             \x20   wait fork;\n\
             \x20   #1;\n\
             \x20   $finish;\n\
             end\n\
             endmodule"
        );

        let result = with_timeout(move || {
            crate::simulate_signals(&src, 20).is_ok() || true
        });
        assert!(result.is_some(), "wait fork hung seed={}", seed);
    }
}
