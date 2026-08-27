//! Fuzz differential MIXED SIGNED/UNSIGNED — dua operan beda signedness
//! dalam satu ekspresi.
//!
//! Blind spot: signed_fuzz memaksa SEMUA operan signed (reg signed),
//! wide_fuzz memilih satu dunia per seed. Tapi LRM §11.8.2 Tabel 11-21
//! punya aturan kompleks saat operand beda signedness:
//! - Arithmetic: signed IFF KEDUA operan signed → unsigned bila salah satu
//! - Comparison: KEDUA harus signed utk signed compare → unsigned bila
//! - Shift: mengikuti signedness operan KIRI
//!
//! Test ini menghasilkan campuran:
//! - `reg signed [w-1:0] a` + `reg [wb-1:0] b` (signed + unsigned)
//! - Literal `'sd` + variabel unsigned
//! - Assignment `wire signed` dari hasil campuran

use crate::fuzz::gen::{generate, lit_sv, mask_of};

const MIX_WIDTHS: [u32; 5] = [4, 8, 16, 24, 32];

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

/// Mini-AST dengan penanda signedness setiap node.
#[derive(Clone, Debug)]
enum MExpr {
    VarA(bool), // bool = is_signed
    VarB(bool),
    Lit(u128, bool),
    Neg(Box<MExpr>),
    Arith(MOp, Box<MExpr>, Box<MExpr>),
    Cmp(MCmp, Box<MExpr>, Box<MExpr>),
    /// `>>>` — shift aritmetika (sign-fill jika lhs signed)
    Sshr(Box<MExpr>, Box<MExpr>),
    /// `>>` — shift logis
    Shr(Box<MExpr>, Box<MExpr>),
    /// `<<<` — shift kiri
    Shl(Box<MExpr>, Box<MExpr>),
}

#[derive(Clone, Copy, Debug)]
enum MOp { Add, Sub, Mul, Div, Mod }

#[derive(Clone, Copy, Debug)]
enum MCmp { Lt, Le, Gt, Ge, Eq, Ne }

impl MOp {
    fn sym(self) -> &'static str {
        match self { MOp::Add => "+", MOp::Sub => "-", MOp::Mul => "*", MOp::Div => "/", MOp::Mod => "%" }
    }
}
impl MCmp {
    fn sym(self) -> &'static str {
        match self {
            MCmp::Lt => "<", MCmp::Le => "<=", MCmp::Gt => ">", MCmp::Ge => ">=",
            MCmp::Eq => "==", MCmp::Ne => "!=",
        }
    }
}

impl MExpr {
    /// Signedness menurut LRM §11.8.2.
    fn is_signed(&self) -> bool {
        match self {
            MExpr::VarA(s) | MExpr::VarB(s) => *s,
            MExpr::Lit(_, s) => *s,
            MExpr::Neg(e) => e.is_signed(),
            MExpr::Cmp(..) => false, // cmp selalu unsigned result
            MExpr::Sshr(l, _) | MExpr::Shr(l, _) | MExpr::Shl(l, _) => l.is_signed(),
            MExpr::Arith(_, l, r) => l.is_signed() && r.is_signed(),
        }
    }

    fn eval(&self, w: u32, a: u64, b: u64) -> (u128, bool) {
        // Returns (value_masked, has_x)
        let m = mask_of128(w);
        match self {
            MExpr::VarA(s) => {
                let v = a as u128 & m;
                if *s { (v, false) } else { (v, false) }
            }
            MExpr::VarB(s) => {
                let v = b as u128 & m;
                if *s { (v, false) } else { (v, false) }
            }
            MExpr::Lit(v, _) => (v & m, false),
            MExpr::Neg(e) => {
                let (val, hx) = e.eval(w, a, b);
                if hx { return (0, true); }
                let sv = if e.is_signed() { sign_ext(val, w) } else { val as i128 };
                let neg = sv.wrapping_neg();
                ((neg as u128) & m, false)
            }
            MExpr::Arith(op, l, r) => {
                let (lv, hx) = l.eval(w, a, b);
                let (rv, hy) = r.eval(w, a, b);
                if hx || hy { return (0, true); }
                let signed = l.is_signed() && r.is_signed();
                let (x, y) = if signed {
                    (sign_ext(lv, w), sign_ext(rv, w))
                } else {
                    (lv as i128, rv as i128)
                };
                if matches!(op, MOp::Div | MOp::Mod) && y == 0 {
                    return (0, true);
                }
                if matches!(op, MOp::Div | MOp::Mod) && signed
                    && x == -(1i128 << (w - 1)) && y == -1
                {
                    return (0, true); // overflow
                }
                let v = match op {
                    MOp::Add => x.wrapping_add(y),
                    MOp::Sub => x.wrapping_sub(y),
                    MOp::Mul => x.wrapping_mul(y),
                    MOp::Div => x.wrapping_div(y),
                    MOp::Mod => x.wrapping_rem(y),
                };
                ((v as u128) & m, false)
            }
            MExpr::Cmp(op, l, r) => {
                let (lv, hx) = l.eval(w, a, b);
                let (rv, hy) = r.eval(w, a, b);
                if hx || hy { return (0, true); }
                // Comparison: signed IFF KEDUA operan signed (LRM §11.8.2)
                let signed = l.is_signed() && r.is_signed();
                let (x, y) = if signed {
                    (sign_ext(lv, w), sign_ext(rv, w))
                } else {
                    (lv as i128, rv as i128)
                };
                let ok = match op {
                    MCmp::Lt => x < y,
                    MCmp::Le => x <= y,
                    MCmp::Gt => x > y,
                    MCmp::Ge => x >= y,
                    MCmp::Eq => x == y,
                    MCmp::Ne => x != y,
                };
                (if ok { 1 } else { 0 }, false)
            }
            MExpr::Sshr(l, r) => {
                let (lv, hx) = l.eval(w, a, b);
                let (rv, hy) = r.eval(w, a, b);
                if hx || hy { return (0, true); }
                let amt = (rv.min(w as u128)) as u32;
                if l.is_signed() {
                    let sv = sign_ext(lv, w);
                    let shifted = if amt >= w {
                        if sv < 0 { -1i128 } else { 0 }
                    } else { sv >> amt };
                    ((shifted as u128) & m, false)
                } else {
                    let shifted = if amt >= w { 0u128 } else { lv >> amt };
                    (shifted & m, false)
                }
            }
            MExpr::Shr(l, r) => {
                let (lv, hx) = l.eval(w, a, b);
                let (rv, hy) = r.eval(w, a, b);
                if hx || hy { return (0, true); }
                let amt = (rv.min(w as u128)) as u32;
                let shifted = if amt >= w { 0u128 } else { lv >> amt };
                (shifted & m, false)
            }
            MExpr::Shl(l, r) => {
                let (lv, hx) = l.eval(w, a, b);
                let (rv, hy) = r.eval(w, a, b);
                if hx || hy { return (0, true); }
                let amt = (rv.min(w as u128)) as u32;
                let shifted = if amt >= w { 0u128 } else { (lv << amt) & m };
                (shifted & m, false)
            }
        }
    }

    fn to_sv(&self, w: u32) -> String {
        match self {
            MExpr::VarA(s) => "a".to_string(),
            MExpr::VarB(s) => "b".to_string(),
            MExpr::Lit(v, s) => {
                let sfx = if *s { "s" } else { "" };
                format!("{}'{}h{:x}", w, sfx, v & mask_of128(w))
            }
            MExpr::Neg(e) => format!("(-{})", e.to_sv(w)),
            MExpr::Arith(op, l, r) => format!("({} {} {})", l.to_sv(w), op.sym(), r.to_sv(w)),
            MExpr::Cmp(op, l, r) => format!("({} {} {})", l.to_sv(w), op.sym(), r.to_sv(w)),
            MExpr::Sshr(l, r) => format!("({} >>> {})", l.to_sv(w), r.to_sv(w)),
            MExpr::Shr(l, r) => format!("({} >> {})", l.to_sv(w), r.to_sv(w)),
            MExpr::Shl(l, r) => format!("({} <<< {})", l.to_sv(w), r.to_sv(w)),
        }
    }
}

fn gen_mixed_expr(w: u32, rng: &mut fastrand::Rng, depth: u32) -> MExpr {
    if depth >= 3 {
        return gen_mixed_leaf(w, rng);
    }
    match rng.usize(0..10) {
        0..=2 => gen_mixed_leaf(w, rng),
        3 => {
            let op = [MOp::Add, MOp::Sub, MOp::Mul, MOp::Div, MOp::Mod];
            let o = op[rng.usize(0..op.len())];
            MExpr::Arith(o, Box::new(gen_mixed_expr(w, rng, depth + 1)),
                         Box::new(gen_mixed_expr(w, rng, depth + 1)))
        }
        4..=5 => {
            let op = [MCmp::Lt, MCmp::Le, MCmp::Gt, MCmp::Ge, MCmp::Eq, MCmp::Ne];
            let o = op[rng.usize(0..op.len())];
            MExpr::Cmp(o, Box::new(gen_mixed_expr(w, rng, depth + 1)),
                        Box::new(gen_mixed_expr(w, rng, depth + 1)))
        }
        6 => MExpr::Neg(Box::new(gen_mixed_expr(w, rng, depth + 1))),
        7 => MExpr::Sshr(Box::new(gen_mixed_expr(w, rng, depth + 1)),
                          Box::new(gen_mixed_expr(w, rng, depth + 1))),
        8 => MExpr::Shr(Box::new(gen_mixed_expr(w, rng, depth + 1)),
                         Box::new(gen_mixed_expr(w, rng, depth + 1))),
        _ => MExpr::Shl(Box::new(gen_mixed_expr(w, rng, depth + 1)),
                         Box::new(gen_mixed_expr(w, rng, depth + 1))),
    }
}

fn gen_mixed_leaf(w: u32, rng: &mut fastrand::Rng) -> MExpr {
    let m = mask_of128(w);
    match rng.usize(0..6) {
        0 => MExpr::VarA(true),  // a selalu signed (reg signed di SV)
        1 => MExpr::VarB(false), // b selalu unsigned (reg di SV)
        2 => MExpr::Lit(rng.u128(..) & m, true),   // signed literal
        3 => MExpr::Lit(rng.u128(..) & m, false),   // unsigned literal
        4 => MExpr::Lit(0, true),
        _ => MExpr::Lit(m, rng.bool()),
    }
}

fn mixed_source(expr_sv: &str, w: u32, aval: &str, bval: &str) -> String {
    format!(
        "module mixed_sign_fuzz_mod;\n\
         \x20   reg signed [{hi}:0] a;\n\
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
fn mixed_signed_unsigned_arith_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    let n_seeds: u64 = std::env::var("MARIA_MIXED_FUZZ_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);

    for seed in 0..n_seeds {
        let w = MIX_WIDTHS[seed as usize % MIX_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xFE_DC_BA);

        let root = gen_mixed_expr(w, &mut rng, 2);

        let m = mask_of128(w);
        let a = rng.u128(..) & m;
        let b = rng.u128(..) & m;

        let (expected, has_x) = root.eval(w, a as u64, b as u64);
        if has_x { continue; }

        // Hasil comparison = 1 bit; lainnya = w bit.
        let is_cmp = matches!(&root, MExpr::Cmp(..));
        let result_mask = if is_cmp { 1u128 } else { m };
        let expected = expected & result_mask;

        let expr_sv = root.to_sv(w);
        let aval = lit_sv(a as u64, w);
        let bval = lit_sv(b as u64, w);
        let src = mixed_source(&expr_sv, w, &aval, &bval);

        let actual = std::thread::Builder::new()
            .name("mixed-sign-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
                move || {
                    crate::simulate_signals(&src, 30)
                        .ok()
                        .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "y").map(|(_, v)| v.to_u64()))
                }
            })
            .expect("spawn mixed-sign-sim")
            .join()
            .expect("sim panic");

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} w={} harap={:#x} dapat={:?}\n{}",
                seed, w, expected, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 60, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch mixed sign:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}

#[test]
fn mixed_sign_shift_arithmetic_vs_logical() {
    // `a >>> b` vs `a >> b` pada operand signed — harus beda untuk
    // operand negatif (MSB set). Kedua diuji dalam source terpisah.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = MIX_WIDTHS[seed as usize % MIX_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAB_CD_01);

        let a = rng.u64(0..) & mask_of(w);
        // Paksa MSB set (negatif) ~50% waktu.
        let a = if rng.bool() { a | (1u64 << (w - 1).min(63)) } else { a };
        let shift_amt = rng.u32(0..w);

        let m = mask_of(w);

        // Golden `>>>` (arithmetic shift right, sign-fill)
        let expected_sar = {
            let sv = sign_ext(a as u128, w);
            let amt = shift_amt.min(w);
            let shifted = if amt >= w {
                if sv < 0 { -1i128 } else { 0 }
            } else { sv >> amt };
            (shifted as u128) & mask_of128(w)
        };

        // Golden `>>` (logical shift right, zero-fill)
        let expected_srl = {
            let amt = shift_amt.min(w);
            if amt >= w { 0u128 } else { ((a as u128) >> amt) & mask_of128(w) }
        };

        let a_lit = lit_sv(a, w);
        let sh_lit = lit_sv(shift_amt as u64, w);

        // Test `>>>` (signed)
        let src_sar = format!(
            "module mixed_sign_fuzz_mod;\n\
             \x20   reg signed [{hi}:0] a;\n\
             \x20   reg [{hi}:0] b;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = (a >>> {sh});\n\
             \x20   initial begin\n\
             \x20       a = {a};\n\
             \x20       b = 0;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1, sh = sh_lit, a = a_lit
        );

        let actual_sar = std::thread::Builder::new()
            .name("mixed-sar-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn(move || {
                crate::simulate_signals(&src_sar, 30)
                    .ok()
                    .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "y").map(|(_, v)| v.to_u64()))
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        if actual_sar != Some(expected_sar as u64) {
            mismatch.push(format!(
                "SAR seed={} w={} a={:#x} sh={} harap={:#x} dapat={:?}",
                seed, w, a, shift_amt, expected_sar, actual_sar
            ));
        }

        // Test `>>` (logical, meskipun operan signed)
        let src_srl = format!(
            "module mixed_sign_fuzz_mod;\n\
             \x20   reg signed [{hi}:0] a;\n\
             \x20   reg [{hi}:0] b;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = (a >> {sh});\n\
             \x20   initial begin\n\
             \x20       a = {a};\n\
             \x20       b = 0;\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1, sh = sh_lit, a = a_lit
        );

        let actual_srl = std::thread::Builder::new()
            .name("mixed-srl-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn(move || {
                crate::simulate_signals(&src_srl, 30)
                    .ok()
                    .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "y").map(|(_, v)| v.to_u64()))
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        if actual_srl != Some(expected_srl as u64) {
            mismatch.push(format!(
                "SRL seed={} w={} a={:#x} sh={} harap={:#x} dapat={:?}",
                seed, w, a, shift_amt, expected_srl, actual_srl
            ));
        }

        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch mixed shift:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}
