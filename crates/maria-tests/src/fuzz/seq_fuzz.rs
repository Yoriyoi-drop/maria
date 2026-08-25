//! Differential sekuensial — register clocked dengan NBA + feedback.
//!
//! Satu file = satu tanggung jawab: invarian temporal. Blind spot fuzzer
//! kombinasional: scheduler event (`posedge`), update nonblocking (`<=`),
//! dan feedback register (q menjadi operand ekspresi siklus berikutnya)
//! belum terlatih.
//!
//! Pola: `q <= f(a, q)` tiap `posedge clk`; stimulus `a` acak per siklus;
//! nilai q tiap siklus di-capture ke reg m_i dan dibandingkan dengan model
//! emas iteratif `Expr::eval(w, a_i, q_prev)` (operand b = q). Seed dengan
//! X di jalur emas mana pun di-skip (model 2-state); lebar >64 bit juga
//! di-skip (kontrak eval hanya menjamin 64 bit rendah, sedangkan shift
//! kanan pada lebar >64 mempengaruhi bit rendah dari bit tinggi yang tak
//! termodelkan).

use super::gen::{generate, lit_sv};

/// Jumlah siklus clock per testcase.
const CYCLES: usize = 6;

/// Ganti variabel `b` → `q` pada hasil `to_sv` (identifier utuh saja; base
/// literal `'b` tidak tersentuh karena didahului `'`).
fn rename_b_to_q(sv: &str) -> String {
    let bytes = sv.as_bytes();
    let mut out = String::with_capacity(sv.len());
    let mut i = 0;
    while i < sv.len() {
        if bytes[i] == b'b'
            && (i == 0 || !is_ident_byte(bytes[i - 1]))
            && (i + 1 == sv.len() || !is_ident_byte(bytes[i + 1]))
        {
            out.push('q');
            i += 1;
        } else {
            out.push_str(&sv[i..i + 1]);
            i += 1;
        }
    }
    out
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn seq_source(expr_sv: &str, w: u32, avecs: &[u64]) -> String {
    let mut decls = String::new();
    for i in 0..CYCLES {
        decls.push_str(&format!("    reg [{}:0] m{};\n", w - 1, i));
    }
    let mut body = String::from("        clk = 0;\n");
    body.push_str(&format!("        a = {};\n", lit_sv(avecs[0], w)));
    body.push_str("        q = 0;\n        #1;\n");
    for i in 0..CYCLES {
        body.push_str("        clk = 1;\n        #1;\n");
        body.push_str(&format!("        m{} = q;\n", i));
        body.push_str("        clk = 0;\n        #1;\n");
        if i + 1 < CYCLES {
            body.push_str(&format!(
                "        a = {};\n        #1;\n",
                lit_sv(avecs[i + 1], w)
            ));
        }
    }
    body.push_str("        #1 $finish;\n");
    format!(
        "module seq_fuzz_mod;\n\
         \x20   reg clk;\n\
         \x20   reg [{hi}:0] a;\n\
         \x20   reg [{hi}:0] q;\n\
         \x20   wire [{hi}:0] nxt;\n\
         \x20   assign nxt = {expr};\n\
         \x20   always @(posedge clk) q <= nxt;\n\
         {decls}\
         \x20   initial begin\n\
         {body}\
         \x20   end\n\
         endmodule\n",
        hi = w - 1,
        expr = expr_sv,
        decls = decls,
        body = body,
    )
}

#[test]
fn sequential_nba_feedback_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    'seeds: for seed in 0..110u64 {
        let input = generate(seed.wrapping_mul(61_509).wrapping_add(7));
        // Kontrak eval 64-bit rendah — lihat catatan header.
        if input.w > 64 {
            continue;
        }
        // Model feedback mengasumsikan q selebar w; b sempit (wb < w)
        // butuh model truncation q terpisah — skip.
        if input.wb != input.w {
            continue;
        }
        // Stimulus a per siklus — deterministik dari seed turunan.
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x5EED_5EED);
        let mask = super::gen::mask_of(input.w);
        let avecs: Vec<u64> = (0..CYCLES)
            .map(|i| {
                if i == 0 {
                    input.a & mask
                } else {
                    rng.u64(0..) & mask
                }
            })
            .collect();
        // Jalankan model emas lebih dulu; seed dengan X di langkah mana
        // pun di-skip (maria menghasilkan X, model 2-state tak comparable).
        let mut golden_q = 0u64;
        let mut golden: Vec<u64> = Vec::with_capacity(CYCLES);
        for &a in &avecs {
            if input.expr.eval_has_x(input.w, a, golden_q) {
                continue 'seeds;
            }
            if input.expr.max_width(input.w as u64) > 128 {
                continue 'seeds;
            }
            golden_q = input.expr.eval(input.w, a, golden_q) & mask;
            golden.push(golden_q);
        }
        // Operand b pada ekspresi = q (feedback) — rename b→q saat render.
        let expr_sv = rename_b_to_q(&input.expr.to_sv(input.w));
        let src = seq_source(&expr_sv, input.w, &avecs);
        let sigs = match std::thread::Builder::new()
            .name("seq-fuzz-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
                move || crate::simulate_signals(&src, ((CYCLES + 4) * 5) as u64)
            })
        {
            Ok(h) => match h.join() {
                Ok(Ok(s)) => s,
                _ => {
                    mismatch.push(format!("seed={} sim gagal/panic", input.seed));
                    continue;
                }
            },
            Err(_) => {
                mismatch.push(format!("seed={} spawn gagal", input.seed));
                continue;
            }
        };
        for i in 0..CYCLES {
            let name = format!("m{}", i);
            let actual = sigs
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.to_u64());
            if actual != Some(golden[i]) {
                mismatch.push(format!(
                    "seed={} cycle={} a={:#x} exp={:#x} act={:?}\n{}",
                    input.seed, i, avecs[i], golden[i], actual, src
                ));
                break;
            }
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch sekuensial:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
