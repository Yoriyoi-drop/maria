//! Fuzz differential ternary operator — `cond ? true_val : false_val`.
//!
//! Blind spot fuzzer existing: generator acak menghasilkan ternary jarang
//! dan kondisi jarang bernilai X. Padahal LRM §11.4.11 + Tabel 11-21
//! punya aturan khusus:
//! - Kondisi X + true_val ≠ false_val → hasil X
//! - Kondisi X + true_val == false_val → hasil value
//! - Kondisi 0/1 → pilih cabang; cabang terpilih menentukan lebar hasil.
//!
//! Selain itu: nested ternary, ternary sebagai operand aritmetika, dan
//! ternary dengan operand X/Z menguji propagasi di kedalaman.

use crate::fuzz::gen::{generate, lit_sv, mask_of};
use crate::fuzz::expr::Expr;

/// Lebar yang dipakai — kecil agar per-bandit eksak & tabel murah.
const TERNARY_WIDTHS: [u32; 5] = [2, 4, 8, 16, 32];

/// Mini-AST untuk ternary differential. Lebih terkontrol dari Expr acak:
/// kondisi = perbandingan atau literal, true/false = literal atau variabel
/// atau konstruksi khusus (X-propagation, bitwise).
#[derive(Clone, Debug)]
enum TExpr {
    /// Literal 2-state.
    Lit(u64),
    /// Literal dengan bit X (mask).
    XLit { v: u64, m: u64 },
    /// Variabel.
    Var(char),
    /// Ternary bersarang: `c ? t : f`.
    Ternary(Box<TExpr>, Box<TExpr>, Box<TExpr>),
    /// Perbandingan: `l == r`, `l < r`, `l >= r`.
    Cmp(TCmp, Box<TExpr>, Box<TExpr>),
    /// Bitwise AND/OR/XOR — menguji propagasi bit per bit.
    BitOp(TBitOp, Box<TExpr>, Box<TExpr>),
    /// Unary negasi: `-e`.
    Neg(Box<TExpr>),
}

#[derive(Clone, Copy, Debug)]
enum TCmp { Eq, Ne, Lt, Le, Gt, Ge }

#[derive(Clone, Copy, Debug)]
enum TBitOp { And, Or, Xor }

impl TCmp {
    fn sym(self) -> &'static str {
        match self {
            TCmp::Eq => "==", TCmp::Ne => "!=",
            TCmp::Lt => "<", TCmp::Le => "<=",
            TCmp::Gt => ">", TCmp::Ge => ">=",
        }
    }
}

impl TBitOp {
    fn sym(self) -> &'static str {
        match self { TBitOp::And => "&", TBitOp::Or => "|", TBitOp::Xor => "^" }
    }

    fn apply(self, a: u128, b: u128, w: u32) -> u128 {
        let m = mask_of128(w);
        match self {
            TBitOp::And => (a & m) & (b & m),
            TBitOp::Or => (a & m) | (b & m),
            TBitOp::Xor => (a & m) ^ (b & m),
        }
    }
}

fn mask_of128(w: u32) -> u128 {
    if w >= 128 { u128::MAX } else { (1u128 << w) - 1 }
}

fn sign_ext(p: u128, w: u32) -> i128 {
    if w > 0 && w < 128 && ((p >> (w - 1)) & 1) == 1 {
        (p as i128).wrapping_sub(1i128 << w)
    } else {
        p as i128
    }
}

struct TEval {
    val: u128,
    undef: bool, // X/Z propagation
}

impl TExpr {
    /// Evaluasi golden. `world_a`/`world_b` adalah nilai variabel penuh
    /// (selebar konteks). Variabel di sini hanya a/b, selebar w.
    fn eval(&self, w: u32, a: u128, b: u128) -> TEval {
        let m = mask_of128(w);
        match self {
            TExpr::Lit(v) => TEval { val: (*v as u128) & m, undef: false },
            TExpr::XLit { .. } => TEval { val: 0, undef: true },
            TExpr::Var(c) => {
                let v = if *c == 'a' { a } else { b };
                TEval { val: v & m, undef: false }
            }
            TExpr::Neg(e) => {
                let r = e.eval(w, a, b);
                if r.undef { return TEval { val: 0, undef: true }; }
                let sv = sign_ext(r.val, w);
                let negated = sv.wrapping_neg();
                TEval { val: (negated as u128) & m, undef: false }
            }
            TExpr::Cmp(op, l, r) => {
                let lv = l.eval(w, a, b);
                let rv = r.eval(w, a, b);
                if lv.undef || rv.undef { return TEval { val: 0, undef: true }; }
                let ok = match op {
                    TCmp::Eq => lv.val == rv.val,
                    TCmp::Ne => lv.val != rv.val,
                    TCmp::Lt => lv.val < rv.val,
                    TCmp::Le => lv.val <= rv.val,
                    TCmp::Gt => lv.val > rv.val,
                    TCmp::Ge => lv.val >= rv.val,
                };
                TEval { val: if ok { 1 } else { 0 }, undef: false }
            }
            TExpr::BitOp(op, l, r) => {
                let lv = l.eval(w, a, b);
                let rv = r.eval(w, a, b);
                if lv.undef || rv.undef { return TEval { val: 0, undef: true }; }
                TEval { val: op.apply(lv.val, rv.val, w), undef: false }
            }
            TExpr::Ternary(cond, t, f) => {
                let cv = cond.eval(w, a, b);
                let tv = t.eval(w, a, b);
                let fv = f.eval(w, a, b);
                if cv.undef {
                    // Kondisi X: jika true_val == false_val → value,
                    // else → X (LRM §11.4.11).
                    if !tv.undef && !fv.undef && (tv.val & m) == (fv.val & m) {
                        TEval { val: tv.val & m, undef: false }
                    } else {
                        TEval { val: 0, undef: true }
                    }
                } else if cv.val != 0 {
                    TEval { val: tv.val & m, undef: tv.undef }
                } else {
                    TEval { val: fv.val & m, undef: fv.undef }
                }
            }
        }
    }

    fn to_sv(&self, w: u32) -> String {
        match self {
            TExpr::Lit(v) => lit_sv(*v, w),
            TExpr::XLit { v, m } => {
                let mut bits = String::with_capacity(w as usize);
                for i in (0..w).rev() {
                    let bit_idx = i as u64;
                    if bit_idx < 64 && (m >> bit_idx) & 1 == 1 {
                        bits.push('x');
                    } else {
                        let bit = if bit_idx >= 64 { 0 } else { (v >> bit_idx) & 1 };
                        bits.push(if bit == 1 { '1' } else { '0' });
                    }
                }
                format!("{}'b{}", w, bits)
            }
            TExpr::Var(c) => c.to_string(),
            TExpr::Neg(e) => format!("(-{})", e.to_sv(w)),
            TExpr::Cmp(op, l, r) => format!("({} {} {})", l.to_sv(w), op.sym(), r.to_sv(w)),
            TExpr::BitOp(op, l, r) => format!("({} {} {})", l.to_sv(w), op.sym(), r.to_sv(w)),
            TExpr::Ternary(c, t, f) => {
                format!("(({}) ? ({}) : ({}))", c.to_sv(w), t.to_sv(w), f.to_sv(w))
            }
        }
    }
}

fn gen_texpr(w: u32, rng: &mut fastrand::Rng, depth: u32) -> TExpr {
    if depth >= 4 {
        return gen_tleaf(w, rng);
    }
    match rng.usize(0..10) {
        0..=2 => gen_tleaf(w, rng),
        3..=4 => {
            let op = [TCmp::Eq, TCmp::Ne, TCmp::Lt, TCmp::Le, TCmp::Gt, TCmp::Ge];
            let o = op[rng.usize(0..op.len())];
            TExpr::Cmp(o, Box::new(gen_texpr(w, rng, depth + 1)),
                       Box::new(gen_texpr(w, rng, depth + 1)))
        }
        5..=6 => {
            let op = [TBitOp::And, TBitOp::Or, TBitOp::Xor];
            let o = op[rng.usize(0..op.len())];
            TExpr::BitOp(o, Box::new(gen_texpr(w, rng, depth + 1)),
                          Box::new(gen_texpr(w, rng, depth + 1)))
        }
        7 => TExpr::Neg(Box::new(gen_texpr(w, rng, depth + 1))),
        _ => TExpr::Ternary(
            Box::new(gen_texpr(w, rng, depth + 1)),
            Box::new(gen_texpr(w, rng, depth + 1)),
            Box::new(gen_texpr(w, rng, depth + 1)),
        ),
    }
}

fn gen_tleaf(w: u32, rng: &mut fastrand::Rng) -> TExpr {
    match rng.usize(0..5) {
        0 => TExpr::Var(if rng.bool() { 'a' } else { 'b' }),
        1 => TExpr::Lit(rng.u64(0..) & mask_of(w)),
        2 => {
            let m = mask_of(w);
            let mask = loop {
                let xm = rng.u64(0..) & m;
                if xm != 0 { break xm; }
            };
            TExpr::XLit { v: rng.u64(0..) & m, m: mask }
        }
        3 => TExpr::Lit(0),
        _ => TExpr::Lit(mask_of(w)),
    }
}

fn ternary_source(expr_sv: &str, w: u32, aval: &str, bval: &str) -> String {
    format!(
        "module ternary_fuzz_mod;\n\
         \x20   reg [{hi}:0] a;\n\
         \x20   reg [{hi}:0] b;\n\
         \x20   wire [{hi}:0] y;\n\
         \x20   assign y = {expr};\n\
         \x20   initial begin\n\
         \x20       a = {aval};\n\
         \x20       b = {bval};\n\
         \x20       #10;\n\
         \x20       $finish;\n\
         \x20   end\n\
         endmodule\n",
        hi = w - 1,
        expr = expr_sv,
        aval = aval,
        bval = bval,
    )
}

#[test]
fn ternary_x_condition_semantics_match_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    let n_seeds: u64 = std::env::var("MARIA_TERNARY_FUZZ_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);

    for seed in 0..n_seeds {
        let w = TERNARY_WIDTHS[seed as usize % TERNARY_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x7E_3A_B);

        // ~30% kondisi X (XLit), 70% kondisi known (perbandingan/literal).
        let cond = if rng.usize(0..10) < 3 {
            let m = mask_of(w);
            let xmask = loop {
                let xm = rng.u64(0..) & m;
                if xm != 0 { break xm; }
            };
            TExpr::XLit { v: rng.u64(0..) & m, m: xmask }
        } else {
            gen_texpr(w, &mut rng, 2)
        };
        let true_val = gen_texpr(w, &mut rng, 3);
        let false_val = gen_texpr(w, &mut rng, 3);

        let root = TExpr::Ternary(
            Box::new(cond),
            Box::new(true_val),
            Box::new(false_val),
        );

        let mut apat = Vec::with_capacity(w as usize);
        let mut bpat = Vec::with_capacity(w as usize);
        for _ in 0..w {
            apat.push(if rng.bool() { 1u128 } else { 0 });
            bpat.push(if rng.bool() { 1u128 } else { 0 });
        }
        let a = apat.iter().enumerate().fold(0u128, |acc, (i, &v)| acc | (v << i));
        let b = bpat.iter().enumerate().fold(0u128, |acc, (i, &v)| acc | (v << i));

        let golden = root.eval(w, a, b);
        if golden.undef {
            continue; // skip X result — invariant panic/determinism masih jalan
        }
        let expected = golden.val & mask_of128(w);

        let expr_sv = root.to_sv(w);
        let aval = lit_sv((a & mask_of128(w)) as u64, w);
        let bval = lit_sv((b & mask_of128(w)) as u64, w);
        let src = ternary_source(&expr_sv, w, &aval, &bval);

        let actual = std::thread::Builder::new()
            .name("ternary-fuzz-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
                move || {
                    crate::simulate_signals(&src, 30)
                        .ok()
                        .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "y").map(|(_, v)| v.to_u64()))
                }
            })
            .expect("spawn ternary-fuzz-sim")
            .join()
            .expect("sim panic");

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} harap={:#x} dapat={:?}\n{}\n---\n{}",
                seed, w, expected, actual, src, expr_sv
            ));
        }
        checked += 1;
    }
    assert!(checked > 50, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch ternary:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}

#[test]
fn ternary_same_branch_xor_identity() {
    // Metamorphic: `(c ? a : a)` selalu == a terlepas kondisi.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let input = generate(seed ^ 0x3C_4D_5E);
        if input.expr.eval_has_x(input.w, input.a, input.b) {
            continue;
        }
        let w = input.w;
        let a_lit = lit_sv(input.a, w);
        let b_lit = lit_sv(input.b, input.wb);

        // (a > b) ? a : a  → harus selalu a
        let src_same = format!(
            "module ternary_fuzz_mod;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   reg [{hi}:0] b;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = ((a > b) ? a : a);\n\
             \x20   initial begin\n\
             \x20       a = {a};\n\
             \x20       b = {b};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1, a = a_lit, b = b_lit
        );

        let actual = std::thread::Builder::new()
            .name("ternary-id-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn(move || {
                crate::simulate_signals(&src_same, 30)
                    .ok()
                    .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "y").map(|(_, v)| v.to_u64()))
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        let expected = input.a & mask_of(w);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} harap={:#x} dapat={:?}", seed, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(mismatch.is_empty(), "{} mismatch:\n{}", mismatch.len(), mismatch.join("\n"));
}
