//! Fuzz resource exhaustion — tangga ukuran input ekstrem.
//!
//! Satu file = satu tanggung jawab: mencari panic/hang/stack-overflow pada
//! struktur bersarang dalam, literal raksasa, identifier panjang, sinyal
//! banyak, dan loop generate tak berujung. Pola: 1 → 10 → 100 → 1K → 100K.
//!
//! Invariant: pipeline boleh menolak (Err) atau menyelesaikan, TIDAK BOLEH
//! crash proses / hang tanpa batas.

use std::sync::mpsc;
use std::time::Duration;

fn with_timeout<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    // Stack besar agar overflow rekursi parser terdeteksi sebagai Err/timeout,
    // bukan SIGSEGV proses.
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || {
            let _ = tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
        })
        .ok()?;
    rx.recv_timeout(Duration::from_secs(secs))
        .ok()
        .and_then(|r| r.ok())
}

/// Tangga nested begin/end — parser harus graceful di kedalaman ekstrem.
#[test]
fn fuzz_deep_nested_blocks() {
    for depth in [10i64, 100, 1_000, 10_000, 50_000] {
        let mut src = String::from("module m;\ninitial begin\n");
        for i in 0..depth {
            src.push_str(&format!("begin : b{}\n", i));
        }
        for i in 0..depth {
            src.push_str(&format!("end : b{}\n", depth - 1 - i));
        }
        src.push_str("end\nendmodule");
        let t = src;
        let r = with_timeout(15, move || crate::compile_str(&t).is_ok());
        assert!(r.is_some(), "hang/crash pada nested begin depth {}", depth);
    }
}

/// Tangga kurung expression bersarang.
#[test]
fn fuzz_deep_paren_nesting() {
    for depth in [10, 100, 1_000, 10_000, 100_000] {
        let inner = "8'd1".to_string();
        let mut src = String::from("module m; wire [7:0] w = ");
        src.push_str(&"(".repeat(depth));
        src.push_str(&inner);
        src.push_str(&")".repeat(depth));
        src.push_str("; endmodule");
        let t = src;
        let r = with_timeout(15, move || crate::compile_str(&t).is_ok());
        assert!(r.is_some(), "hang/crash pada paren depth {}", depth);
    }
}

/// Unary bersarang (tanpa kurung) — kasus Pratt parser paling dalam.
#[test]
fn fuzz_deep_unary_chain() {
    for depth in [100, 1_000, 10_000] {
        let mut expr = String::from("8'd1");
        for _ in 0..depth {
            expr.insert(0, '~');
        }
        let src = format!("module m; wire [7:0] w = {}; endmodule", expr);
        let t = src;
        let r = with_timeout(15, move || crate::compile_str(&t).is_ok());
        assert!(r.is_some(), "hang/crash pada unary chain {}", depth);
    }
}

/// Literal boundary: lebar maksimum, digit tidak valid, underscore aneh.
#[test]
fn fuzz_extreme_literals() {
    let mut cases: Vec<String> = vec![
        "module m; wire w = 1'bx; endmodule".to_string(),
        "module m; wire [1023:0] w = 1024'h0; endmodule".to_string(),
        "module m; wire [65535:0] w = 65536'hx; endmodule".to_string(),
        "module m; wire w = 0'd0; endmodule".to_string(),           // width 0
        "module m; wire w = 2'b2; endmodule".to_string(),           // digit invalid
        "module m; wire w = 8'b; endmodule".to_string(),            // tanpa digit
        "module m; wire w = 'd; endmodule".to_string(),             // sized tanpa width
        "module m; wire w = 8'd99999999999999999999; endmodule".to_string(), // overflow nilai
        "module m; wire w = 4'sd15; endmodule".to_string(),         // signed
        "module m; wire w = 8'b1010_; endmodule".to_string(),       // underscore trailing
        "module m; wire w = 16'h____; endmodule".to_string(),       // underscore semua
        "module m; real r = 3.; endmodule".to_string(),
        "module m; real r = .5; endmodule".to_string(),
        "module m; wire w = 8'o779; endmodule".to_string(),         // octal invalid
        "module m; wire w = 65'haaaaaaaaaaaaaaaaaaa; endmodule".to_string(),
    ];
    cases.push(format!("module m; wire w = {}'h0; endmodule", u32::MAX)); // width ekstrem
    for (i, c) in cases.iter().enumerate() {
        let t = c.clone();
        assert!(
            with_timeout(10, move || crate::compile_str(&t).is_ok()).is_some(),
            "hang/crash pada literal case #{}: {}",
            i,
            c
        );
    }
}

/// Identifier panjang & escaped identifier.
#[test]
fn fuzz_long_identifiers() {
    for len in [255usize, 4096, 65536, 1_000_000] {
        let id = format!("s{}", "x".repeat(len));
        let src = format!("module m; wire {} ; assign {} = 1'b0; endmodule", id, id);
        let t = src;
        let r = with_timeout(10, move || crate::compile_str(&t).is_ok());
        assert!(r.is_some(), "hang/crash pada identifier len {}", len);
    }
    let escaped = [
        "\\bus+width ",
        "\\$root.x ",
        "\\ ",
        "\\a\\\n b",
        "\\\\",
    ];
    for (i, e) in escaped.iter().enumerate() {
        let src = format!("module m; wire {}; endmodule", e);
        let t = src;
        assert!(
            with_timeout(10, move || crate::compile_str(&t).is_ok()).is_some(),
            "hang/crash pada escaped identifier #{}: {:?}",
            i,
            e
        );
    }
}

/// Sinyal sangat banyak — state/symbol table/VCD growth.
#[test]
fn fuzz_many_signals_sim() {
    for n in [100usize, 1_000, 10_000] {
        let mut src = String::from("module m;\n");
        for i in 0..n {
            src.push_str(&format!("wire [7:0] w{} = 8'd{};\n", i, i % 256));
        }
        src.push_str("initial #1 $finish;\nendmodule");
        let t = src;
        let r = with_timeout(30, move || crate::simulate_signals(&t, 10).is_ok());
        assert!(r.is_some(), "hang/crash pada {} sinyal", n);
    }
}

/// Generate loop besar & parameter chain — elaborator unrolling.
#[test]
fn fuzz_generate_scale() {
    for n in [10usize, 100, 1_000] {
        let src = format!(
            "module m; wire [31:0] acc [0:{}]; genvar i; generate for (i=0; i<{}; i=i+1) begin : g assign acc[i] = i * 4; end endgenerate endmodule", n, n);
        let t = src;
        let r = with_timeout(20, move || crate::compile_str(&t).is_ok());
        assert!(r.is_some(), "hang/crash pada generate n={}", n);
    }
}
