//! Fuzz differential ARITMETIKA & PERBANDINGAN LEBAR (>64-bit).
//!
//! Blind spot yang dilatih: jalur u128 di evaluator (`to_u128_wide`,
//! `from_u128_wide`, Power square-and-multiply, perbandingan bitwise
//! `cmp_unsigned_bits`, Div/Mod signed 128) yang tidak tersentuh generator
//! lama — seluruh stimulus lama muat u64 sehingga bit ≥64 selalu nol.
//!
//! Desain emas eksak & sederhana (selaras signed_fuzz):
//! - SEMUA operan selebar `w`; satu kasus = satu dunia signedness
//!   (`'sh` vs `'h`) → tidak ada mixed-signedness.
//! - Cmp hanya boleh jadi ROOT atau kondisi ternary (hasil 1-bit tidak
//!   masuk aritmetika → lebar hasil pasti w, tanpa propagasi rumit).
//! - Emas menghitung dalam i128/u128 lalu memotong ke pola w-bit.
//! - Kasus tak terdefinisi (div/mod nol, INT_MIN/-1) ditandai undefined →
//!   skip compare numerik.

/// Mask pola pada lebar `w` (≤128).
fn mask128(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

/// Lebar >64-bit + boundary-nya. 63/64/65 melintasi jalur u64↔u128;
/// 96/127/128 melatih multi-word LogicVec penuh.
const WIDE_WIDTHS: [u32; 7] = [48, 63, 64, 65, 96, 127, 128];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WArith {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Xor,
    Sll,
    Srl,
    Sra,
}

impl WArith {
    fn sym(self) -> &'static str {
        match self {
            WArith::Add => "+",
            WArith::Sub => "-",
            WArith::Mul => "*",
            WArith::Div => "/",
            WArith::Mod => "%",
            WArith::And => "&",
            WArith::Or => "|",
            WArith::Xor => "^",
            WArith::Sll => "<<<",
            WArith::Srl => ">>",
            WArith::Sra => ">>>",
        }
    }
    fn all() -> &'static [WArith] {
        &[
            WArith::Add,
            WArith::Sub,
            WArith::Mul,
            WArith::Div,
            WArith::Mod,
            WArith::And,
            WArith::Or,
            WArith::Xor,
            WArith::Sll,
            WArith::Srl,
            WArith::Sra,
        ]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WCmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Neq,
}

impl WCmp {
    fn sym(self) -> &'static str {
        match self {
            WCmp::Lt => "<",
            WCmp::Le => "<=",
            WCmp::Gt => ">",
            WCmp::Ge => ">=",
            WCmp::Eq => "==",
            WCmp::Neq => "!=",
        }
    }
}

/// Mini-AST lebar. Semua operan selebar w.
#[derive(Clone, Debug)]
enum WExpr {
    VarA,
    VarB,
    /// Pola w-bit mentah.
    Lit(u128),
    Neg(Box<WExpr>),
    BitNot(Box<WExpr>),
    Arith(WArith, Box<WExpr>, Box<WExpr>),
    Cmp(WCmp, Box<WExpr>, Box<WExpr>),
    /// Kondisi PASTI diketahui (Cmp antar nilai known).
    Cond(Box<WExpr>, Box<WExpr>, Box<WExpr>),
}

fn to_pattern(v: i128, w: u32) -> u128 {
    ((v as u128) & mask128(w)) as u128
}

/// Interpretasi pola w-bit: signed → two's-complement; unsigned → zero-ext.
fn interpret(p: u128, w: u32, is_signed: bool) -> i128 {
    let m = mask128(w);
    let p = p & m;
    if is_signed && w > 0 && w < 128 && ((p >> (w - 1)) & 1) == 1 {
        (p as i128).wrapping_sub(1i128 << w)
    } else {
        p as i128
    }
}

struct WEval {
    /// Pola hasil (selalu selebar w, kecuali Cmp = 1 bit).
    val: u128,
    undefined: bool,
    /// Signedness node ini (LRM §11.8.2 Tabel 11-21): Var/Lit ikut dunia
    /// kasus; perbandingan & logical → SELALU unsigned; shift mengikuti
    /// operan kiri; operator lain → signed bila KEDUA operan signed.
    /// "Taint" unsigned menular ke atas — tanpa ini Div/Mod/Cmp pada
    /// sub-ekspresi yang mengandung Cmp salah tanda.
    signed: bool,
}

impl WExpr {
    fn eval(&self, w: u32, a: u128, b: u128, world_signed: bool) -> WEval {
        match self {
            WExpr::VarA => WEval {
                val: a & mask128(w),
                undefined: false,
                signed: world_signed,
            },
            WExpr::VarB => WEval {
                val: b & mask128(w),
                undefined: false,
                signed: world_signed,
            },
            WExpr::Lit(v) => WEval {
                val: v & mask128(w),
                undefined: false,
                signed: world_signed,
            },
            WExpr::Neg(e) => {
                let r = e.eval(w, a, b, world_signed);
                // wrapping_neg: -i128::MIN overflow di debug (pola w=128
                // dengan MSB set).
                WEval {
                    val: to_pattern(interpret(r.val, w, r.signed).wrapping_neg(), w),
                    undefined: r.undefined,
                    signed: r.signed,
                }
            }
            WExpr::BitNot(e) => {
                let r = e.eval(w, a, b, world_signed);
                WEval {
                    val: !r.val & mask128(w),
                    undefined: r.undefined,
                    signed: r.signed,
                }
            }
            WExpr::Cmp(op, l, r) => {
                let lv = l.eval(w, a, b, world_signed);
                let rv = r.eval(w, a, b, world_signed);
                if lv.undefined || rv.undefined {
                    return WEval { val: 0, undefined: true, signed: false };
                }
                // Perbandingan bertanda hanya bila KEDUA operan signed;
                // operand di-interpretasi pada lebar w masing-masing.
                // UNSIGNED pada w=128 HARUS dibandingkan sebagai u128 —
                // `p as i128` menafsir ulang MSB sehingga Ge/Lt salah
                // (wide_fuzz seed=48).
                let ok = if lv.signed && rv.signed {
                    let (x, y) = (interpret(lv.val, w, true), interpret(rv.val, w, true));
                    match op {
                        WCmp::Lt => x < y,
                        WCmp::Le => x <= y,
                        WCmp::Gt => x > y,
                        WCmp::Ge => x >= y,
                        WCmp::Eq => x == y,
                        WCmp::Neq => x != y,
                    }
                } else {
                    let (x, y) = (lv.val & mask128(w), rv.val & mask128(w));
                    match op {
                        WCmp::Lt => x < y,
                        WCmp::Le => x <= y,
                        WCmp::Gt => x > y,
                        WCmp::Ge => x >= y,
                        WCmp::Eq => x == y,
                        WCmp::Neq => x != y,
                    }
                };
                WEval {
                    val: if ok { 1 } else { 0 },
                    undefined: false,
                    signed: false, // hasil cmp SELALU unsigned
                }
            }
            WExpr::Cond(c, t, f) => {
                let cv = c.eval(w, a, b, world_signed);
                if cv.undefined {
                    return WEval { val: 0, undefined: true, signed: false };
                }
                if cv.val & 1 == 1 {
                    t.eval(w, a, b, world_signed)
                } else {
                    f.eval(w, a, b, world_signed)
                }
            }
            WExpr::Arith(op, l, r) => {
                let lv = l.eval(w, a, b, world_signed);
                let rv = r.eval(w, a, b, world_signed);
                let dbg = std::env::var("MARIA_DBG_WGOLD2").is_ok();
                if dbg {
                    eprintln!("[WG2] {:?} l={:x}({}) r={:x}({})", op, lv.val, lv.signed, rv.val, rv.signed);
                }
                if lv.undefined || rv.undefined {
                    return WEval { val: 0, undefined: true, signed: false };
                }
                match op {
                    WArith::Sll | WArith::Srl | WArith::Sra => {
                        // Shift amount self-determined UNSIGNED dari pola;
                        // saturasi ke w (hasil shift ≥ w sudah jenuh).
                        let amt = rv.val.min(w as u128) as u32;
                        let res = match op {
                            WArith::Sll => {
                                if amt >= w {
                                    0
                                } else {
                                    (lv.val << amt) & mask128(w)
                                }
                            }
                            WArith::Srl => {
                                if amt >= w {
                                    0
                                } else {
                                    (lv.val & mask128(w)) >> amt
                                }
                            }
                            _ => {
                                // SRA: arithmetic bila lhs signed, else logical.
                                if lv.signed {
                                    let sv = interpret(lv.val, w, true);
                                    let shifted = if amt >= w {
                                        if sv < 0 {
                                            -1i128
                                        } else {
                                            0
                                        }
                                    } else {
                                        sv >> amt
                                    };
                                    to_pattern(shifted, w)
                                } else if amt >= w {
                                    0
                                } else {
                                    lv.val >> amt
                                }
                            }
                        };
                        // Hasil shift mengikuti signedness OPERAN KIRI.
                        WEval { val: res, undefined: false, signed: lv.signed }
                    }
                    _ => {
                        let op_signed = lv.signed && rv.signed;
                        let (xs, ys) = (
                            interpret(lv.val, w, op_signed),
                            interpret(rv.val, w, op_signed),
                        );
                        if matches!(op, WArith::Div | WArith::Mod) && ys == 0 {
                            return WEval { val: 0, undefined: true, signed: op_signed };
                        }
                        if matches!(op, WArith::Div | WArith::Mod)
                            && op_signed
                            && xs == i128::MIN
                            && ys == -1
                        {
                            return WEval { val: 0, undefined: true, signed: op_signed };
                        }
                        // Bitwise: level pola, sign-agnostic. Add/Sub/Mul:
                        // kongruen mod 2^w — pola sama. Div/Mod: HARUS pada
                        // interpretasi sesuai signedness operasi.
                        let v: u128 = match op {
                            WArith::And => (lv.val & mask128(w)) & (rv.val & mask128(w)),
                            WArith::Or => (lv.val & mask128(w)) | (rv.val & mask128(w)),
                            WArith::Xor => (lv.val & mask128(w)) ^ (rv.val & mask128(w)),
                            _ => {
                                if op_signed {
                                    match op {
                                        WArith::Add => to_pattern(xs.wrapping_add(ys), w),
                                        WArith::Sub => to_pattern(xs.wrapping_sub(ys), w),
                                        WArith::Mul => to_pattern(xs.wrapping_mul(ys), w),
                                        WArith::Div => to_pattern(xs.wrapping_div(ys), w),
                                        WArith::Mod => to_pattern(xs.wrapping_rem(ys), w),
                                        _ => unreachable!(),
                                    }
                                } else {
                                    match op {
                                        WArith::Add => lv.val.wrapping_add(rv.val),
                                        WArith::Sub => lv.val.wrapping_sub(rv.val),
                                        WArith::Mul => lv.val.wrapping_mul(rv.val),
                                        WArith::Div => lv.val / rv.val,
                                        WArith::Mod => lv.val % rv.val,
                                        _ => unreachable!(),
                                    }
                                }
                            }
                        };
                        WEval {
                            val: v & mask128(w),
                            undefined: false,
                            signed: op_signed,
                        }
                    }
                }
            }
        }
    }

    fn to_sv(&self, w: u32, is_signed: bool) -> String {
        let sfx = if is_signed { "s" } else { "" };
        match self {
            WExpr::VarA => "a".to_string(),
            WExpr::VarB => "b".to_string(),
            WExpr::Lit(v) => format!("{}'{}h{:x}", w, sfx, v & mask128(w)),
            WExpr::Neg(e) => format!("(-{})", e.to_sv(w, is_signed)),
            WExpr::BitNot(e) => format!("(~{})", e.to_sv(w, is_signed)),
            WExpr::Arith(op, l, r) => format!(
                "({} {} {})",
                l.to_sv(w, is_signed),
                op.sym(),
                r.to_sv(w, is_signed)
            ),
            WExpr::Cmp(op, l, r) => format!(
                "({} {} {})",
                l.to_sv(w, is_signed),
                op.sym(),
                r.to_sv(w, is_signed)
            ),
            WExpr::Cond(c, t, f) => format!(
                "({} ? {} : {})",
                c.to_sv(w, is_signed),
                t.to_sv(w, is_signed),
                f.to_sv(w, is_signed)
            ),
        }
    }
}

fn gen_wexpr(w: u32, rng: &mut fastrand::Rng, depth: u32) -> WExpr {
    if depth == 0 {
        match rng.usize(0..3) {
            0 => WExpr::VarA,
            1 => WExpr::VarB,
            _ => WExpr::Lit(rng.u128(..) & mask128(w)),
        }
    } else {
        match rng.usize(0..10) {
            0..=2 => gen_wexpr(w, rng, 0), // leaf lebih sering
            3 => WExpr::Neg(Box::new(gen_wexpr(w, rng, depth - 1))),
            4 => WExpr::BitNot(Box::new(gen_wexpr(w, rng, depth - 1))),
            5..=8 => {
                let op = WArith::all()[rng.usize(0..WArith::all().len())];
                WExpr::Arith(
                    op,
                    Box::new(gen_wexpr(w, rng, depth - 1)),
                    Box::new(gen_wexpr(w, rng, depth - 1)),
                )
            }
            _ => {
                let cmp = match rng.usize(0..6) {
                    0 => WCmp::Lt,
                    1 => WCmp::Le,
                    2 => WCmp::Gt,
                    3 => WCmp::Ge,
                    4 => WCmp::Eq,
                    _ => WCmp::Neq,
                };
                WExpr::Cmp(
                    cmp,
                    Box::new(gen_wexpr(w, rng, depth - 1)),
                    Box::new(gen_wexpr(w, rng, depth - 1)),
                )
            }
        }
    }
}

fn wide_source(expr_sv: &str, w: u32, a: u128, b: u128, is_signed: bool, cmp_root: bool) -> String {
    let sgn = if is_signed { "signed " } else { "" };
    let (y_decl, expr_final) = if cmp_root {
        ("wire y;".to_string(), expr_sv.to_string())
    } else {
        (format!("wire {}[{}:0] y;", sgn, w - 1), expr_sv.to_string())
    };
    let sfx = if is_signed { "s" } else { "" };
    format!(
        "module wide_fuzz_mod;\n\
         \x20   reg {}[{hi}:0] a;\n\
         \x20   reg {}[{hi}:0] b;\n\
         \x20   {y_decl}\n\
         \x20   assign y = {expr};\n\
         \x20   initial begin\n\
         \x20       a = {w}'{sfx}h{ax:x};\n\
         \x20       b = {w}'{sfx}h{bx:x};\n\
         \x20       #10;\n\
         \x20       $finish;\n\
         \x20   end\n\
         endmodule\n",
        sgn,
        sgn,
        y_decl = y_decl,
        expr = expr_final,
        w = w,
        sfx = sfx,
        hi = w - 1,
        ax = a & mask128(w),
        bx = b & mask128(w),
    )
}

#[test]
fn wide_arithmetic_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    let n_seeds: u64 = std::env::var("MARIA_WIDE_FUZZ_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);

    for seed in 0..n_seeds {
        let w = WIDE_WIDTHS[seed as usize % WIDE_WIDTHS.len()];
        let is_signed = seed % 2 == 1;
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x57_1D_E);

        let pick = |rng: &mut fastrand::Rng| -> u128 {
            // ~30% boundary: 0, 1, all-ones, MSB-set.
            if rng.usize(0..10) < 3 {
                match rng.usize(0..4) {
                    0 => 0,
                    1 => 1,
                    2 => mask128(w),
                    _ => {
                        let msb = if w >= 128 {
                            1u128 << 127
                        } else {
                            1u128 << (w - 1)
                        };
                        msb | (rng.u128(..) & mask128(w) >> 1)
                    }
                }
            } else {
                rng.u128(..) & mask128(w)
            }
        };
        let (a, b) = (pick(&mut rng), pick(&mut rng));
        let root = gen_wexpr(w, &mut rng, 3);

        let r = root.eval(w, a, b, is_signed);
        if std::env::var("MARIA_DBG_WGOLD").is_ok() {
            eprintln!("[DBG-WG] seed={} w={} val={:x} undef={}", seed, w, r.val, r.undefined);
        }
        if r.undefined {
            continue;
        }
        let cmp_root = matches!(root, WExpr::Cmp(..));
        let expected = if cmp_root { r.val } else { r.val & mask128(w) };

        let expr_sv = root.to_sv(w, is_signed);
        let src = wide_source(&expr_sv, w, a, b, is_signed, cmp_root);
        let yw = if cmp_root { 1 } else { w };
        let actual = std::thread::Builder::new()
            .name("wide-fuzz-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
                move || {
                    crate::simulate_signals(&src, 30)
                        .ok()
                        .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "y").map(|(_, v)| v.clone()))
                }
            })
            .expect("spawn wide-fuzz-sim")
            .join()
            .expect("sim panic");
        // Bandingkan per-bit (LogicVec tidak punya to_u128; pola >64-bit
        // tidak bisa lewat u64).
        let expected_bits: Vec<maria_ir::LogicVal> = (0..yw)
            .map(|i| {
                if (expected >> i) & 1 == 1 {
                    maria_ir::LogicVal::One
                } else {
                    maria_ir::LogicVal::Zero
                }
            })
            .collect();
        let ok = match &actual {
            Some(v) => {
                v.width as u32 == yw
                    && v.bits
                        .iter()
                        .zip(expected_bits.iter())
                        .all(|(g, e)| g == e)
            }
            None => false,
        };
        if !ok {
            mismatch.push(format!(
                "seed={} w={} signed={} harap={:x} dapat={:?}\n{}\n---\n{}",
                seed, w, is_signed, expected, actual, src, expr_sv
            ));
        }
        checked += 1;
    }
    assert!(checked > 60, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch wide:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}
