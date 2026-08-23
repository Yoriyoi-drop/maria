//! Fuzz EOF & truncation — potong source valid di setiap posisi penting.
//!
//! Satu file = satu tanggung jawab: menemukan parser yang panic, hang,
//! atau infinite-loop saat input terputus mendadak (EOF di tengah konstruksi).
//!
//! Invariant yang ditegakkan:
//! - `compile_str` pada SEMUA prefix dari source valid tidak boleh panic
//!   maupun hang. Hasil Ok/Err bebas (truncated = sah ditolak).
//! - Gabungan valid+invalid / invalid+valid tidak boleh hang.

use std::sync::mpsc;
use std::time::Duration;

/// Corpus konstruksi SV yang beragam — tiap fragmen punya titik potong
/// "menarik" (setelah keyword, dalam expression, tengah string, dll).
const CORPUS: &[&str] = &[
    "module m; endmodule",
    "module m #(parameter W = 8) (input clk); reg [W-1:0] r; always @(posedge clk) r <= r + 1'b1; endmodule",
    "module m; function automatic [31:0] f(input [31:0] x); return x * 2; endfunction assign y = f(4); endmodule",
    "module m; generate for (genvar i = 0; i < 3; i++) begin : g wire w = i; end endgenerate endmodule",
    "`define ADD(a, b) ((a) + (b))\nmodule m; wire w = `ADD(1, 2); endmodule",
    "module m; initial begin $display(\"hello %0d\", 42); #10; $finish; end endmodule",
    "module m; typedef enum logic [1:0] {A, B} st_t; st_t s; always_comb case (s) A: ; default: ; endcase endmodule",
    "module m; interface_if u(); endmodule\ninterface interface_if; logic q; endinterface",
    "package p; parameter int K = 5; endpackage\nmodule m; import p::*; wire [K-1:0] w; endmodule",
    "module m; class c; int x; task t(); x++; endtask endclass endmodule",
    "module m; reg [7:0] mem [0:15]; initial $readmemh(\"x.hex\", mem); endmodule",
    "module m; property p_x; @(posedge clk) a |-> b; endproperty assert property (p_x); endmodule",
    "/* block comment */ module m; // line comment\n wire w; endmodule",
    "module m; wire [31:0] w = 32'hDEAD_BEEF ^ {16'h1, 16'h2}; endmodule",
];

/// Jalankan `f` dengan timeout — hang = kegagalan keras (livelock parser).
fn with_timeout<T: Send + 'static>(name: &str, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .name(name.to_string())
        .spawn(move || {
            let _ = tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
        })
        .ok()?;
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(res) => res.ok(),
        Err(_) => None, // timeout atau join gagal
    }
}

#[test]
fn fuzz_truncation_no_panic_no_hang() {
    let mut total = 0u64;
    let mut ok_count = 0u64;
    for (ci, src) in CORPUS.iter().enumerate() {
        let chars: Vec<char> = src.chars().collect();
        // Truncate di setiap posisi untuk source pendek; sampling untuk panjang.
        let step = if chars.len() > 200 { 3 } else { 1 };
        let mut pos = 0;
        while pos <= chars.len() {
            let truncated: String = chars[..pos].iter().collect();
            total += 1;
            let t = truncated.clone();
            match with_timeout("fuzz-trunc", move || crate::compile_str(&t).is_ok()) {
                None => panic!(
                    "hang/panic saat compile truncated (corpus #{}, pos {}):\n{}",
                    ci, pos, truncated
                ),
                Some(true) => ok_count += 1,
                Some(false) => {}
            }
            pos += step;
        }
    }
    eprintln!("[fuzz-truncation] cases={} compile_ok={}", total, ok_count);
    assert!(total > 500, "corpus terlalu kecil untuk bermakna");
}

#[test]
fn fuzz_valid_invalid_valid_no_hang() {
    // Sequence mutation: sisipkan fragment rusak di antara dua bagian valid.
    let broken = [
        "endmodule",
        ")",
        "]",
        "}",
        ";",
        "begin",
        "`endif",
        "\"unterminated",
        "8'b2",
        "'d",
        "\\escaped ",
        "128'hFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
        "module",
        "((((((",
    ];
    let base = "module m; wire [7:0] w = 8'd1 + 8'd2; endmodule";
    let mut total = 0u64;
    for b in broken {
        for split in [base.find("wire").unwrap(), base.len()] {
            let mutated = format!("{}{}{}", &base[..split], b, &base[split..]);
            total += 1;
            let t = mutated.clone();
            assert!(
                with_timeout("fuzz-seq", move || crate::compile_str(&t).is_ok()).is_some(),
                "hang/panic pada:\n{}",
                mutated
            );
        }
    }

    eprintln!("[fuzz-valid-invalid-valid] cases={}", total);
}
