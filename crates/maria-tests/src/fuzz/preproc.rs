//! Fuzz preprocessor — dark corner `define`/`include`/conditional.
//!
//! Satu file = satu tanggung jawab: makro rekursif, include siklik,
//! kondisional tak seimbang, EOF di tengah direktif. Invariant: tidak panic,
//! tidak hang, tidak infinite expansion (harus ada depth guard).

use std::sync::mpsc;
use std::time::Duration;

fn with_timeout<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let _ = tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
        })
        .ok()?;
    rx.recv_timeout(Duration::from_secs(10))
        .ok()
        .and_then(|r| r.ok())
}

#[test]
fn fuzz_preproc_macro_recursion_guard() {
    // Makro meng-expans dirinya sendiri — boleh Err, TIDAK BOLEH hang/panic.
    let cases = [
        "`define X X\nmodule m; wire w = `X; endmodule",
        "`define A `B\n`define B `A\nmodule m; wire w = `A; endmodule",
        "`define A `A `A\nmodule m; wire w = `A; endmodule",
    ];
    for c in cases {
        let t = c.to_string();
        assert!(
            with_timeout(move || crate::compile_str(&t).is_ok()).is_some(),
            "hang/panic pada: {}",
            c
        );
    }

    // Chain makro panjang (ekspansi bertingkat) — harus selesai dalam waktu
    // wajar untuk ukuran wajar; ekstrem boleh ditolak.
    for n in [10usize, 100, 1_000] {
        let mut src = String::from("`define M0 1\n");
        for i in 1..n {
            src.push_str(&format!("`define M{} (`M{} + 1)\n", i, i - 1));
        }
        src.push_str("module m; wire [31:0] w = `M999; endmodule");
        let t = src;
        assert!(
            with_timeout(move || crate::compile_str(&t).is_ok()).is_some(),
            "hang pada chain {} makro",
            n
        );
    }
}

#[test]
fn fuzz_preproc_unbalanced_conditionals() {
    let cases = [
        "`endif\nmodule m; endmodule",
        "`else\nmodule m; endmodule",
        "module m;\n`ifdef X\nwire w;\n`else",
        "module m; `ifdef A `elsif B `elsif C wire w; endmodule",
        "module m; `ifndef A `undef B endmodule",
        "`ifdef",
        "`elsif",
        "`define",
        "`include",
        "`undef",
        "`define M(\nmodule m; endmodule",
        "`define M(a\nmodule m; endmodule",
        "`define M(a,) (a)\nmodule m; wire w = `M(1,); endmodule",
        "`define M(a,b) (a)+(b)\nmodule m; wire w = `M(1); endmodule", // arg kurang
        "`define M(a) (a)\nmodule m; wire w = `M(1,2,3); endmodule",   // arg lebih
        "`M_TIDAK_ADA\nmodule m; endmodule",
    ];
    for (i, c) in cases.iter().enumerate() {
        let t = c.to_string();
        assert!(
            with_timeout(move || crate::compile_str(&t).is_ok()).is_some(),
            "hang/panic pada case preproc #{}: {}",
            i,
            c
        );
    }
}

#[test]
fn fuzz_preproc_include_cycle_and_eof() {
    // Include siklik via temp dir (di luar project).
    let dir = std::env::temp_dir().join(format!("maria_fuzz_pp_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("a.svh"), "`include \"b.svh\"\n").unwrap();
    std::fs::write(dir.join("b.svh"), "`include \"a.svh\"\n").unwrap();
    std::fs::write(dir.join("self.svh"), "`include \"self.svh\"\n").unwrap();

    let top = format!("`include \"{}/a.svh\"\nmodule m; endmodule", dir.display());
    let t = top.clone();
    assert!(
        with_timeout(move || crate::compile_str(&t).is_ok()).is_some(),
        "hang pada include cycle"
    );

    let top_self = format!("`include \"{}/self.svh\"\nmodule m; endmodule", dir.display());
    let t2 = top_self;
    assert!(
        with_timeout(move || crate::compile_str(&t2).is_ok()).is_some(),
        "hang pada include self-cycle"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // EOF di tengah string & comment & macro body.
    let eof_cases = [
        "module m; initial $display(\"abc",
        "/* unterminated block",
        "module m; /* x */",
        "`define M text-without-newline",
        "`define M(a) (a",
        "module m; string s = \"\\",
    ];
    for (i, c) in eof_cases.iter().enumerate() {
        let t = c.to_string();
        assert!(
            with_timeout(move || crate::compile_str(&t).is_ok()).is_some(),
            "hang/panic pada EOF case #{}: {}",
            i,
            c
        );
    }
}
