//! Differential panggilan fungsi — ekspresi fuzz dieksekusi lewat fungsi.
//!
//! Satu file = satu tanggung jawab: invarian pemanggilan. Blind spot fuzzer
//! existing: seluruh generator menempel ekspresi langsung di assign/proses,
//! sehingga jalur call-function (coercion lebar argumen, deklarasi return,
//! binding formal→aktual di elaborator DAN evaluator engine) tidak pernah
//! terlatih dengan input acak.
//!
//! Pola: `function automatic [w-1:0] f(input [w-1:0] x, input [wb-1:0] z);`
//! berisi EKSPRESI FUZZ yang sama (variabel a/b direname x/z) lalu dipanggil:
//! 1. kontinu:      `assign y = f(a, b);`
//! 2. prosedural:   `always @(*) y = f(a, b);`
//!
//! Emas: `Expr::eval(w, a, b)` yang sama dengan guided_fuzz — kalau fungsi
//! adalah pintu transparan, keduanya wajib identik.

use crate::fuzz::gen::{generate, lit_sv, mask_of};

/// Rename identifier utuh pada source (a→x, b→z). Batas ident diperiksa
/// agar sufiks literal (`8'b101`) dan nama lain tak tersentuh.
fn rename_vars(sv: &str, pairs: &[(&str, &str)]) -> String {
    let bytes = sv.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let mut out = String::with_capacity(sv.len());
    let mut i = 0;
    'outer: while i < sv.len() {
        for (from, to) in pairs {
            let fb = from.as_bytes();
            if i + fb.len() <= sv.len()
                && &sv[i..i + fb.len()] == *from
                && (i == 0 || !is_ident(bytes[i - 1]))
                && (i + fb.len() == sv.len() || !is_ident(bytes[i + fb.len()]))
            {
                out.push_str(to);
                i += fb.len();
                continue 'outer;
            }
        }
        out.push(sv[i..].chars().next().expect("non-empty") as char);
        i += 1;
    }
    out
}

/// Blind spot model emas: `BitSel/PartSel` pada `b` divalidasi emas
/// terhadap lebar konteks `w`, BUKAN lebar deklarasi `wb` — select di luar
/// `wb` (X asli menurut §11.5.1) tak tertandai has_x sehingga compare-nya
/// tak bermakna. Kasus seperti itu di-skip (konsisten dgn kontrak oracle).
fn golden_blind_spot(expr: &crate::fuzz::expr::Expr, wb: u32) -> bool {
    use crate::fuzz::expr::Expr;
    match expr {
        Expr::BitSel('b', i) => *i >= wb,
        Expr::PartSel('b', hi, _) => *hi >= wb,
        Expr::Un(_, e) => golden_blind_spot(e, wb),
        Expr::Ternary(c, t, f) => {
            golden_blind_spot(c, wb) || golden_blind_spot(t, wb) || golden_blind_spot(f, wb)
        }
        Expr::Repl(_, e) => golden_blind_spot(e, wb),
        Expr::Bin(_, l, r) => golden_blind_spot(l, wb) || golden_blind_spot(r, wb),
        _ => false,
    }
}

/// Source modul dengan fungsi berisi ekspresi fuzz; `procedural` memilih
/// konteks pemanggilan (continuous assign vs always @(*)).
fn fn_source(expr_sv: &str, w: u32, wb: u32, aval: &str, bval: &str, procedural: bool) -> String {
    let body_expr = rename_vars(expr_sv, &[("a", "x"), ("b", "z")]);
    let drive = if procedural {
        "    always @(*) y = f(a, b);\n"
    } else {
        "    assign y = f(a, b);\n"
    };
    format!(
        "module function_fuzz_mod;\n\
         \x20   reg [{hi}:0] a;\n\
         \x20   reg [{bhi}:0] b;\n\
         \x20   wire [{hi}:0] y;\n\
         \x20   function automatic [{hi}:0] f(input [{hi}:0] x, input [{bhi}:0] z);\n\
         \x20       f = {body};\n\
         \x20   endfunction\n\
         {drive}\
         \x20   initial begin\n\
         \x20       a = {aval};\n\
         \x20       b = {bval};\n\
         \x20       #10;\n\
         \x20       $finish;\n\
         \x20   end\n\
         endmodule\n",
        hi = w - 1,
        bhi = wb - 1,
        body = body_expr,
        drive = drive,
        aval = aval,
        bval = bval,
    )
}

#[test]
fn function_call_is_transparent_vs_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..120u64 {
        let input = generate(seed.wrapping_mul(7_356_331).wrapping_add(23));
        if input.w > 64 || input.w < 2 {
            continue; // emas hanya eksak ≤64 bit; w=1 terlalu sempit utk variasi
        }
        // X pada emas → skip compare numerik (kontrak oracle).
        if input.expr.eval_has_x(input.w, input.a, input.b) {
            continue;
        }
        // Blind spot emas utk select `b` di luar wb (lihat doc fn).
        if golden_blind_spot(&input.expr, input.wb) {
            continue;
        }
        let expected = input.expr.eval(input.w, input.a, input.b) & mask_of(input.w);
        let aval = lit_sv(input.a, input.w);
        let bval = lit_sv(input.b, input.wb);
        let expr_sv = input.expr.to_sv(input.w);

        for procedural in [false, true] {
            let src = fn_source(&expr_sv, input.w, input.wb, &aval, &bval, procedural);
            let actual = std::thread::Builder::new()
                .name("func-fuzz-sim".to_string())
                .stack_size(256 * 1024 * 1024)
                .spawn({
                    let src = src.clone();
                    move || {
                        crate::simulate_signals(&src, 30).ok().and_then(|sigs| {
                            sigs.iter()
                                .find(|(n, _)| *n == "y")
                                .map(|(_, v)| v.to_u64())
                        })
                    }
                })
                .expect("spawn func-fuzz-sim")
                .join()
                .expect("sim panic");
            if actual != Some(expected) {
                mismatch.push(format!(
                    "seed={} procedural={} harap={:#x} dapat={:?}\n{}",
                    seed, procedural, expected, actual, src
                ));
            }
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} ketidakcocokan fungsi:\n{}",
        mismatch.len(),
        mismatch.join("\n---\n")
    );
}
