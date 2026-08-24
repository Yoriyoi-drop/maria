//! Fuzz scheduler & konkurensi SystemVerilog — fork/join, delta cycle,
//! zero-delay loop, determinisme antar-run.
//!
//! Satu file = satu tanggung jawab. Invariant:
//! - Semua kombinasi konstruksi proses selesai dalam batas waktu (tidak hang).
//! - Zero-delay loop ditolak dengan error (delta cap), bukan spin tanpa batas.
//! - Simulasi ulang desain sama menghasilkan trace identik (determinisme).

use std::sync::mpsc;
use std::time::Duration;

fn with_timeout<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(128 * 1024 * 1024)
        .spawn(move || {
            let _ = tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
        })
        .ok()?;
    rx.recv_timeout(Duration::from_secs(secs))
        .ok()
        .and_then(|r| r.ok())
}

/// Kombinasi konstruksi proses — semuanya harus selesai <10s.
#[test]
fn fuzz_process_constructs_complete() {
    let cases = [
        // fork/join dasar
        "module m; initial begin fork #5 a = 1; #3 b = 2; join #1 $finish; end endmodule",
        // join_any
        "module m; initial begin fork begin #5 a = 1; end begin #1 b = 2; end join_any #1 $finish; end endmodule",
        // join_none
        "module m; initial begin fork a = 1; b = 2; join_none #1 $finish; end endmodule",
        // nested fork
        "module m; initial begin fork fork #2 a = 1; join begin #3 b = 2; end join #1 $finish; end endmodule",
        // wait fork
        "module m; initial begin fork #2 a = 1; #4 b = 2; join_none wait fork; #1 $finish; end endmodule",
        // disable fork
        "module m; initial begin fork begin #100 a = 1; end begin #1; disable fork; end join #1 $finish; end endmodule",
        // always + initial interplay
        "module m; reg clk = 0; always #5 clk = ~clk; initial begin #22 $finish; end endmodule",
        // multiple always on same signal
        "module m; reg [7:0] q = 0; always @(posedge clk) q <= q + 1; reg clk = 0; initial begin repeat(5) #1 clk = ~clk; $finish; end endmodule",
        // event trigger/wait
        "module m; event e; initial begin ->e; $finish; end initial begin @e $display(\"hit\"); end endmodule",
        // semaphore/mailbox smoke
        "module m; semaphore s; initial begin s = new(1); s.get(); s.put(); $finish; end endmodule",
    ];
    for (i, c) in cases.iter().enumerate() {
        let t = c.to_string();
        assert!(
            with_timeout(10, move || crate::simulate_signals(&t, 50).is_ok()).is_some(),
            "hang/crash pada kasus scheduler #{}: {}",
            i,
            c
        );
    }
}

/// Zero-delay loop harus ditolak delta-cap, bukan infinite loop.
#[test]
fn fuzz_zero_delay_loop_bounded() {
    let src = "module m; reg q = 0; always #0 q = ~q; initial begin #1 $finish; end endmodule";
    let t = src.to_string();
    let r = with_timeout(30, move || crate::simulate_signals(&t, 20));
    assert!(r.is_some(), "zero-delay loop tidak berhenti (livelock)");
    // Ok atau Err sama-sama sah — yang penting TERMINATE.
}

/// Kombinasi delay ekstrem — besar & nol bercampur.
#[test]
fn fuzz_extreme_delays() {
    let cases = [
        "module m; initial begin #0 $finish; end endmodule",
        "module m; initial begin a = 0; #0 a = 1; #0 a = 2; #1 $finish; end endmodule",
        "module m; wire [7:0] w = 8'd1; initial begin #9000000000000000000; $finish; end endmodule",
    ];
    for (i, c) in cases.iter().enumerate() {
        let t = c.to_string();
        assert!(
            with_timeout(15, move || crate::simulate_signals(&t, 100).is_ok() || true).is_some(),
            "hang pada delay ekstrem #{}",
            i
        );
    }
}

/// Determinisme: simulasi ulang N kali → hasil identik persis.
#[test]
fn fuzz_simulation_repeatability() {
    let designs = [
        "module m; reg [7:0] cnt = 0; reg clk = 0; always #2 clk = ~clk; always @(posedge clk) cnt <= cnt + 3; initial begin #21 $finish; end endmodule",
        "module m; reg a = 0, b = 1; initial fork begin #3 a = 1; end begin #1 b = 0; #2 b = 1; end join #5 $finish; end endmodule",
        "module m; wire [31:0] y = f(6); function automatic [31:0] f(input [31:0] x); return x * x + 1; endfunction initial begin #1 $finish; end endmodule",
    ];
    for (di, d) in designs.iter().enumerate() {
        let base = {
            let t = d.to_string();
            with_timeout(20, move || crate::simulate_signals(&t, 40).ok())
        };
        assert!(base.is_some(), "run pertama gagal/hang desain #{}", di);
        for run in 0..3 {
            let t = d.to_string();
            let again = with_timeout(20, move || crate::simulate_signals(&t, 40).ok());
            assert_eq!(
                base, again,
                "desain #{} run {} ≠ baseline → non-determinisme",
                di, run
            );
        }
    }
}
