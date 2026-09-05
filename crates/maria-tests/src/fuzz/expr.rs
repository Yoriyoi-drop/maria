//! Grammar inti fuzzer — AST ekspresi terbatas (structure-aware).
//!
//! Satu file = satu tanggung jawab: definisi `Expr` + render ke SystemVerilog
//! (`to_sv`) + evaluasi referensi mandiri (`eval`). Generator dan oracle
//! bergantung pada ini; tidak ada logika fuzz lain di sini.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Xor,
    Xnor,
    Shl,
    Shr,
    Sshl,
    Sshr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    LogicAnd,
    LogicOr,
    CaseEq,
    CaseNeq,
    Power,
    Concat,
    Inside,
}

impl BinOp {
    pub fn sym(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::And => "&",
            BinOp::Or => "|",
            BinOp::Xor => "^",
            BinOp::Xnor => "^~",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::Sshl => "<<<",
            BinOp::Sshr => ">>>",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::LogicAnd => "&&",
            BinOp::LogicOr => "||",
            BinOp::CaseEq => "===",
            BinOp::CaseNeq => "!==",
            BinOp::Power => "**",
            BinOp::Concat => "{,}",
            BinOp::Inside => "inside",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            BinOp::Add => "Add",
            BinOp::Sub => "Sub",
            BinOp::Mul => "Mul",
            BinOp::Div => "Div",
            BinOp::Mod => "Mod",
            BinOp::And => "And",
            BinOp::Or => "Or",
            BinOp::Xor => "Xor",
            BinOp::Xnor => "Xnor",
            BinOp::Shl => "Shl",
            BinOp::Shr => "Shr",
            BinOp::Sshl => "Sshl",
            BinOp::Sshr => "Sshr",
            BinOp::Eq => "Eq",
            BinOp::Ne => "Ne",
            BinOp::Lt => "Lt",
            BinOp::Le => "Le",
            BinOp::Gt => "Gt",
            BinOp::Ge => "Ge",
            BinOp::LogicAnd => "LogicAnd",
            BinOp::LogicOr => "LogicOr",
            BinOp::CaseEq => "CaseEq",
            BinOp::CaseNeq => "CaseNeq",
            BinOp::Power => "Power",
            BinOp::Concat => "Concat",
            BinOp::Inside => "Inside",
        }
    }

    pub fn all() -> &'static [BinOp] {
        &[
            BinOp::Add,
            BinOp::Sub,
            BinOp::Mul,
            BinOp::Div,
            BinOp::Mod,
            BinOp::And,
            BinOp::Or,
            BinOp::Xor,
            BinOp::Xnor,
            BinOp::Shl,
            BinOp::Shr,
            BinOp::Sshl,
            BinOp::Sshr,
            BinOp::Eq,
            BinOp::Ne,
            BinOp::Lt,
            BinOp::Le,
            BinOp::Gt,
            BinOp::Ge,
            BinOp::LogicAnd,
            BinOp::LogicOr,
            BinOp::CaseEq,
            BinOp::CaseNeq,
            BinOp::Power,
            BinOp::Concat,
            BinOp::Inside,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    LogicNot,
    Neg,
    /// Reduction AND `&e` — self-determined 1-bit.
    RedAnd,
    /// Reduction OR `|e`.
    RedOr,
    /// Reduction XOR `^e`.
    RedXor,
    /// Reduction NAND `~&e`.
    RedNand,
    /// Reduction NOR `~|e`.
    RedNor,
    /// Reduction XNOR `^~e`.
    RedXnor,
}

impl UnOp {
    pub fn sym(self) -> &'static str {
        match self {
            UnOp::Not => "~",
            UnOp::LogicNot => "!",
            UnOp::Neg => "-",
            UnOp::RedAnd => "&",
            UnOp::RedOr => "|",
            UnOp::RedXor => "^",
            UnOp::RedNand => "~&",
            UnOp::RedNor => "~|",
            UnOp::RedXnor => "^~",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            UnOp::Not => "Not",
            UnOp::LogicNot => "LogicNot",
            UnOp::Neg => "Neg",
            UnOp::RedAnd => "RedAnd",
            UnOp::RedOr => "RedOr",
            UnOp::RedXor => "RedXor",
            UnOp::RedNand => "RedNand",
            UnOp::RedNor => "RedNor",
            UnOp::RedXnor => "RedXnor",
        }
    }

    pub fn all() -> &'static [UnOp] {
        &[
            UnOp::Not,
            UnOp::LogicNot,
            UnOp::Neg,
            UnOp::RedAnd,
            UnOp::RedOr,
            UnOp::RedXor,
            UnOp::RedNand,
            UnOp::RedNor,
            UnOp::RedXnor,
        ]
    }

    /// Reduksi = self-determined 1-bit (LRM §11.4.9), berbeda semantik dari
    /// operator unary bitwise/logical.
    pub fn is_reduction(self) -> bool {
        matches!(
            self,
            UnOp::RedAnd
                | UnOp::RedOr
                | UnOp::RedXor
                | UnOp::RedNand
                | UnOp::RedNor
                | UnOp::RedXnor
        )
    }
}

/// Ekspresi kombinasional terbatas: literal, variabel (a/b), unary, binary.
/// Selalu valid secara sintaksis saat di-render (`to_sv`) — ini yang membuat
/// fuzzer "tidak buta": input bukan byte acak, melainkan SV well-formed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// Literal 2-state sized selebar konteks.
    Lit(u64),
    /// Literal 4-state: bit `m` dirender sebagai `x` (stimulus X/Z).
    /// Golden menandai has_x → oracle skip compare numerik; invariant
    /// panic/determinism tetap tereksekusi.
    XLit {
        v: u64,
        m: u64,
    },
    Var(char),
    Un(UnOp, Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    /// Ternary `cond ? t : f` — hasil max(lebar t, f, konteks).
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    /// Replication `{count{e}}`.
    Repl(u32, Box<Expr>),
    /// Bit-select `var[idx]` — self-determined 1-bit.
    BitSel(char, u32),
    /// Part-select `var[hi:lo]` — self-determined selebar hi-lo+1.
    PartSel(char, u32, u32),
}

fn mask_of(w: u32) -> u64 {
    if w >= 64 {
        u64::MAX
    } else {
        (1u64 << w) - 1
    }
}

/// Mask lebar hingga 128 bit untuk aritmetika internal u128.
fn mask_of128(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

fn lit_sv(v: u64, w: u32) -> String {
    if w == 0 {
        return "0".to_string();
    }
    let m = mask_of(w);
    let val = v & m;
    let mut bits = String::with_capacity(w as usize);
    for i in (0..w).rev() {
        let bit = if i >= 64 { 0 } else { (val >> i) & 1 };
        bits.push(if bit == 1 { '1' } else { '0' });
    }
    format!("{}'b{}", w, bits)
}

impl Expr {
    /// Evaluasi referensi width-aware (model emas). Mengembalikan
    /// `(nilai, lebar_sv)`. Aturan lebar SV (self-determined):
    /// perbandingan/relasional/logical → 1 bit; shift → lebar operand kiri
    /// (dinaikkan ke konteks — LRM §11.8.1: operand kiri shift bersifat
    /// *context-determined*, divalidasi differential vs Icarus);
    /// lainnya → max(lebar operan). Aritmetika internal u128 agar
    /// concat/intermediate hingga 128 bit tetap eksak.
    /// Returns (value, width, has_x) where has_x indicates X-state propagation
    /// (e.g. div/mod by zero). When has_x=true, oracle should skip comparison.
    fn eval_w(&self, w: u32, a: u64, b: u64) -> (u64, u32, bool) {
        let (v, ow, hx) = self.eval_w128(w, w, a as u128, b as u128);
        (v as u64, ow, hx)
    }

    /// Inti evaluasi u128 dengan model sizing LRM §11.8.1 (divalidasi
    /// differential vs Icarus):
    /// - Anak dievaluasi SELF-determined (ctx=0).
    /// - Operasi context-determined (~, unary ±, aritmetika, bitwise,
    ///   shift/power-lhs) bekerja pada `max(lebar anak, ctx)`.
    /// - Perbandingan/logical: operan saling di-extend ke max keduanya
    ///   (TANPA konteks luar), hasil 1 bit.
    /// - Concat & rhs shift/power: self-determined murni.
    /// - Literal/variabel selebar `W` (lebar target akhir — generator
    ///   merender literal sized ke lebar itu).
    fn eval_w128(&self, w: u32, W: u32, a: u128, b: u128) -> (u128, u32, bool) {
        match self {
            Expr::Lit(v) => ((*v as u128) & mask_of128(W), W.max(w), false),
            Expr::XLit { v, m } => {
                // 4-state: hasil pasti mengandung X → has_x (skip numerik).
                let mask = mask_of128(W.max(w));
                ((*v as u128) & mask, W.max(w), true)
            }
            Expr::Var(c) => {
                let x = if *c == 'a' { a } else { b };
                ((x & mask_of128(W)), W.max(w), false)
            }
            Expr::Un(op, e) => {
                if op.is_reduction() {
                    // Reduksi self-determined: operasi pada lebar operand
                    // SENDIRI (tanpa extension konteks), hasil 1-bit.
                    // Operan dievaluasi ulang dgn ctx=0 — anak seperti
                    // perbandingan menghasilkan lebar aslinya (1-bit),
                    // bukan lebar konteks (`&(b[1] < b)` = 1, bukan
                    // reduksi atas vektor selebar konteks).
                    let (x, ew, hx) = e.eval_w128(0, W, a, b);
                    if hx {
                        return (0, 1, true);
                    }
                    let bits = x & mask_of128(ew.max(1));
                    let ones = bits.count_ones();
                    let all_one = ew > 0 && ones == ew;
                    let any_one = ones > 0;
                    let parity = ones & 1 == 1;
                    let v = match op {
                        UnOp::RedAnd => all_one,
                        UnOp::RedOr => any_one,
                        UnOp::RedXor => parity,
                        UnOp::RedNand => !all_one,
                        UnOp::RedNor => !any_one,
                        UnOp::RedXnor => !parity,
                        _ => unreachable!(),
                    };
                    return (v as u128, w.max(1), false);
                }
                let (x, ew, hx) = e.eval_w128(w, W, a, b);
                // ~ / unary ± context-determined: operan di-extend ke
                // max(lebar anak, konteks) SEBELUM operasi.
                let mw = ew.max(w);
                let m = mask_of128(mw);
                let xe = x & m;
                match op {
                    UnOp::Not => {
                        if hx {
                            (0, mw, true)
                        } else {
                            ((!xe) & m, mw, false)
                        }
                    }
                    UnOp::LogicNot => {
                        // X/Z treated as false; nilai 1-bit, lebar konteks.
                        let truthy = if hx { false } else { xe != 0 };
                        (if truthy { 0 } else { 1 }, w.max(1), false)
                    }
                    UnOp::Neg => {
                        if hx {
                            (0, mw, true)
                        } else {
                            (((!xe).wrapping_add(1)) & m, mw, false)
                        }
                    }
                    // Reduksi sudah ditangani early-return sebelum arm ini.
                    _ => unreachable!("reduction ops handled above"),
                }
            }
            Expr::Bin(op, l, r) => {
                // Anak concat: self-determined murni (ctx=0). Anak lainnya —
                // termasuk operan comparison/logical — context-determined
                // (warisi ctx; selaras propagate_context_width di elaborator
                // dan divalidasi differential). PENGECUALIAN: RHS shift
                // self-determined (IEEE 1800 §11.8.1) — `a << ~(1-bit)`
                // menggeser dgn nilai 1-bit, bukan nilai yang di-extend ke
                // lebar konteks.
                let self_det_l = *op == BinOp::Concat;
                let self_det_r = *op == BinOp::Concat
                    || matches!(*op, BinOp::Shl | BinOp::Shr | BinOp::Sshl | BinOp::Sshr);
                let cl = if self_det_l { 0 } else { w };
                let cr = if self_det_r { 0 } else { w };
                let (x, lw, hx) = l.eval_w128(cl, W, a, b);
                let (y, rw, hy) = r.eval_w128(cr, W, a, b);
                let either_x = hx || hy;
                match op {
                    BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::And
                    | BinOp::Or
                    | BinOp::Xor
                    | BinOp::Xnor => {
                        let ow = lw.max(rw).max(w);
                        let om = mask_of128(ow);
                        let xe = x & om;
                        let ye = y & om;
                        let v = match op {
                            BinOp::Add => xe.wrapping_add(ye),
                            BinOp::Sub => xe.wrapping_sub(ye),
                            BinOp::Mul => xe.wrapping_mul(ye),
                            BinOp::And => xe & ye,
                            BinOp::Or => xe | ye,
                            BinOp::Xor => xe ^ ye,
                            BinOp::Xnor => !(xe ^ ye),
                            _ => unreachable!(),
                        } & om;
                        (v, ow, either_x)
                    }
                    BinOp::Div => {
                        let ow = lw.max(rw).max(w);
                        let ye = y & mask_of128(ow);
                        let xe = x & mask_of128(ow);
                        if ye == 0 {
                            (0, ow, true) // div by zero = X
                        } else if either_x {
                            (0, ow, true) // X propagates
                        } else {
                            (xe / ye, ow, false)
                        }
                    }
                    BinOp::Mod => {
                        let ow = lw.max(rw).max(w);
                        let ye = y & mask_of128(ow);
                        let xe = x & mask_of128(ow);
                        if ye == 0 {
                            (0, ow, true) // mod by zero = X
                        } else if either_x {
                            (0, ow, true) // X propagates
                        } else {
                            (xe % ye, ow, false)
                        }
                    }
                    BinOp::Shl | BinOp::Shr | BinOp::Sshl | BinOp::Sshr => {
                        // LHS shift context-determined → ow termasuk ctx.
                        let ow = lw.max(rw).max(w);
                        let om = mask_of128(ow);
                        let xe = x & om;
                        if either_x {
                            (0, ow, true)
                        } else {
                            let amt = y;
                            let v = if amt >= ow as u128 || amt >= 128 {
                                0u128
                            } else {
                                match op {
                                    BinOp::Shl | BinOp::Sshl => ((xe << amt) & om),
                                    BinOp::Shr | BinOp::Sshr => ((xe >> amt) & om),
                                    _ => unreachable!(),
                                }
                            };
                            (v, ow, false)
                        }
                    }
                    BinOp::Power => {
                        let ow = lw.max(rw).max(w);
                        let om = mask_of128(ow);
                        if either_x {
                            (0, ow, true)
                        } else {
                            // Eksponensiasi modular biner mod 2^ow (semantik
                            // SV: hasil di-size ke max operand, aritmetika
                            // modular). Dulu `y >= 64 → 0` — salah total vs
                            // Icarus (seed 86149882 / 110046652).
                            let mut base = x & om;
                            let mut acc: u128 = 1;
                            let mut e = y;
                            while e > 0 {
                                if e & 1 == 1 {
                                    acc = acc.wrapping_mul(base) & om;
                                }
                                e >>= 1;
                                if e > 0 {
                                    base = base.wrapping_mul(base) & om;
                                }
                            }
                            (acc, ow, false)
                        }
                    }
                    BinOp::Concat => {
                        let ow = lw.saturating_add(rw).min(128);
                        let om = mask_of128(ow);
                        let sh = rw.min(127);
                        let v = ((x << sh) | y) & om;
                        (v, ow, either_x)
                    }
                    BinOp::Inside => {
                        // LHS vs item saling di-extend SEBELUM evaluasi anak
                        // (op context-determined bersarang seperti unary
                        // minus mengubah nilai dgn lebar akhir — lihat catatan
                        // arm comparison di bawah).
                        let (_, lw0, _) = l.eval_w128(cl, W, a, b);
                        let (_, rw0, _) = r.eval_w128(cr, W, a, b);
                        let (x, lw, hx) = l.eval_w128(cl.max(rw0), W, a, b);
                        let (y, rw, hy) = r.eval_w128(cr.max(lw0), W, a, b);
                        let cw = lw.max(rw);
                        let xe = x & mask_of128(cw);
                        let ye = y & mask_of128(cw);
                        if hx || hy {
                            return (0, w.max(1), true);
                        }
                        let v = if xe == ye { 1 } else { 0 };
                        (v, w.max(1), false)
                    }
                    BinOp::Eq
                    | BinOp::Ne
                    | BinOp::Lt
                    | BinOp::Le
                    | BinOp::Gt
                    | BinOp::Ge
                    | BinOp::LogicAnd
                    | BinOp::LogicOr
                    | BinOp::CaseEq
                    | BinOp::CaseNeq => {
                        // Operan saling di-extend ke max keduanya (+konteks)
                        // SEBELUM evaluasi anak, bukan hanya mask nilai
                        // sesudahnya. Alasan: op context-determined bersarang
                        // (unary minus/~) mengubah NILAI berdasarkan lebar
                        // akhir — `-((b===b)) < lit` = all-ones < lit = false,
                        // bukan 1 < lit = true (LRM §11.8.1; divalidasi vs
                        // Icarus, multivec seed=4712822). Dua putaran
                        // refinement agar rantai bersarang konvergen.
                        let (_, lw0, _) = l.eval_w128(cl, W, a, b);
                        let (_, rw0, _) = r.eval_w128(cr, W, a, b);
                        let (x1, lw1, _) = l.eval_w128(cl.max(rw0), W, a, b);
                        let (y1, rw1, _) = r.eval_w128(cr.max(lw0), W, a, b);
                        let (x, lw, hx) = l.eval_w128(cl.max(rw1), W, a, b);
                        let (y, rw, hy) = r.eval_w128(cr.max(lw1), W, a, b);
                        if hx || hy {
                            // X in comparison operand = X result
                            return (0, w.max(1), true);
                        }
                        let cw = lw1.max(rw1).max(lw).max(rw);
                        let xe = x & mask_of128(cw);
                        let ye = y & mask_of128(cw);
                        let v = match op {
                            BinOp::Eq | BinOp::CaseEq => {
                                if xe == ye {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::Ne | BinOp::CaseNeq => {
                                if xe != ye {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::Lt => {
                                if xe < ye {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::Le => {
                                if xe <= ye {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::Gt => {
                                if xe > ye {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::Ge => {
                                if xe >= ye {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::LogicAnd => {
                                if xe != 0 && ye != 0 {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::LogicOr => {
                                if xe != 0 || ye != 0 {
                                    1
                                } else {
                                    0
                                }
                            }
                            _ => unreachable!(),
                        };
                        // Hasil comparison bernilai 1 bit tapi lebar efektif =
                        // konteks (operand sudah di-extend sebelum compare).
                        (v, w.max(1), false)
                    }
                }
            }
            Expr::Ternary(cond, t, f) => {
                // LRM §11.4.11 + Tabel 11-21: kondisi `expr1` SELF-DETERMINED
                // (tidak mewarisi konteks) — `(~(a === b)) ? x : y`
                // mengevaluasi ~ pada lebar hasil perbandingan (1 bit), bukan
                // lebar konteks. Hasil max(lebar t, f, konteks).
                let (cv, _, chx) = cond.eval_w128(0, W, a, b);
                let (tv, tw, thx) = t.eval_w128(w, W, a, b);
                let (fv, fw, fhx) = f.eval_w128(w, W, a, b);
                let ow = tw.max(fw).max(w);
                let om = mask_of128(ow);
                let truthy = cv != 0;
                let chosen = if truthy { tv } else { fv };
                // Kondisi X + nilai beda → hasil X; anak X → ikut X.
                let hx = thx || fhx || (chx && (tv & om) != (fv & om));
                (chosen & om, ow, hx)
            }
            Expr::Repl(count, e) => {
                // {N{e}}: self-determined, lebar N*lebar(e).
                let (x, ew, hx) = e.eval_w128(0, W, a, b);
                if *count == 0 {
                    return (0, w.max(1), false);
                }
                let ew = ew.max(1);
                let ow = (ew.saturating_mul(*count as u32)).min(128);
                let om = mask_of128(ow);
                let mut v: u128 = 0;
                for i in 0..*count {
                    let sh = (i as u32 * ew).min(127);
                    v |= (x & mask_of128(ew)) << sh;
                }
                (v & om, ow, hx)
            }
            Expr::BitSel(c, idx) => {
                // var[idx]: 1-bit self-determined; index di luar lebar → X
                // (LRM §11.5.1). idx ≥ 128 di luar representasi u128 → 0
                // (guard panic shift-overflow Rust).
                let x = if *c == 'a' { a } else { b };
                if *idx >= W {
                    return (0, 1, true);
                }
                let v = if *idx >= 128 { 0 } else { (x >> *idx) & 1 };
                (v, 1, false)
            }
            Expr::PartSel(c, hi, lo) => {
                // var[hi:lo] self-determined selebar hi-lo+1; sebagian/banyak
                // bit di luar lebar → mengandung X. lo ≥ 128 → hasil 0
                // (guard panic shift-overflow Rust).
                let x = if *c == 'a' { a } else { b };
                let width = hi.saturating_sub(*lo).saturating_add(1);
                if *hi >= W {
                    return (0, width, true);
                }
                let v = if *lo >= 128 {
                    0
                } else {
                    (x >> *lo) & mask_of128(width)
                };
                (v, width, false)
            }
        }
    }

    /// Evaluasi akhir: hasil ekspresi di-assign ke `y` (lebar `w`) dengan
    /// zero-extension (unsigned). Nilai dikembalikan ter-mask ke `w`.
    /// Untuk w > 64, nilai yang terbaca adalah 64 bit rendah (kontrak
    /// `LogicVec::to_u64` di sisi simulasi).
    pub fn eval(&self, w: u32, a: u64, b: u64) -> u64 {
        let (v, _, _) = self.eval_w128(w, w, a as u128, b as u128);
        ((v & mask_of128(w)) & mask_of128(64)) as u64
    }

    /// Apakah evaluasi menyentuh state X (div/mod-by-zero, dsb.)? Kontrak
    /// terdokumentasi `eval_w`: "When has_x=true, oracle should skip
    /// comparison" — oracle memakai ini untuk melewatkan differential.
    pub fn eval_has_x(&self, w: u32, a: u64, b: u64) -> bool {
        let (_, _, hx) = self.eval_w128(w, w, a as u128, b as u128);
        hx
    }

    /// Lebar intermediate MAKSIMUM (worst-case) dari ekspresi pada konteks
    /// lebar `w`. Oracle memakai ini untuk skip comparison ketika ada
    /// intermediate > 128 bit yang tak bisa dipastikan model emas u128.
    pub fn max_width(&self, w: u64) -> u64 {
        match self {
            Expr::Lit(_) | Expr::Var(_) | Expr::XLit { .. } => w,
            Expr::Un(_, e) => e.max_width(w),
            Expr::Ternary(c, t, f) => c.max_width(w).max(t.max_width(w)).max(f.max_width(w)),
            Expr::Repl(count, e) => (*count as u64).saturating_mul(e.max_width(w)).min(u64::MAX),
            Expr::BitSel(..) => 1,
            Expr::PartSel(_, hi, lo) => hi.saturating_sub(*lo).saturating_add(1) as u64,
            Expr::Bin(op, l, r) => {
                let lw = l.max_width(w);
                let rw = r.max_width(w);
                match op {
                    BinOp::Concat => lw.saturating_add(rw),
                    BinOp::Eq
                    | BinOp::Ne
                    | BinOp::Lt
                    | BinOp::Le
                    | BinOp::Gt
                    | BinOp::Ge
                    | BinOp::LogicAnd
                    | BinOp::LogicOr
                    | BinOp::CaseEq
                    | BinOp::CaseNeq
                    | BinOp::Inside => 1,
                    _ => lw.max(rw),
                }
            }
        }
    }

    /// Render ke SystemVerilog. Setiap sub-ekspresi non-leaf dibungkus
    /// kurung agar selalu unambiguous.
    pub fn to_sv(&self, w: u32) -> String {
        match self {
            Expr::Lit(v) => lit_sv(*v, w),
            Expr::XLit { v, m } => {
                // Literal 4-state: bit ber-mask dirender 'x' (stimulus X).
                let mut bits = String::with_capacity(w as usize);
                for i in (0..w).rev() {
                    let bit_idx = i as u64;
                    if bit_idx < 64 && (m >> bit_idx) & 1 == 1 {
                        bits.push('x');
                    } else {
                        let bit = if bit_idx >= 64 {
                            0
                        } else {
                            (*v >> bit_idx) & 1
                        };
                        bits.push(if bit == 1 { '1' } else { '0' });
                    }
                }
                format!("{}'b{}", w, bits)
            }
            Expr::Var(c) => c.to_string(),
            Expr::Un(op, e) => format!("{}({})", op.sym(), e.to_sv(w)),
            Expr::Ternary(c, t, f) => {
                format!("({} ? ({}) : ({}))", c.to_sv(w), t.to_sv(w), f.to_sv(w))
            }
            Expr::Repl(count, e) => format!("{{{}{{{}}}}}", count, e.to_sv(w)),
            Expr::BitSel(c, idx) => format!("{}[{}]", c, idx),
            Expr::PartSel(c, hi, lo) => format!("{}[{}:{}]", c, hi, lo),
            Expr::Bin(op, l, r) => {
                match *op {
                    BinOp::Concat => format!("{{{}, {}}}", l.to_sv(w), r.to_sv(w)),
                    // Inside wajib dibungkus kurung: presedensi `inside` sama
                    // dengan relational dan left-assoc (IEEE 1800 Tabel 11-2),
                    // tanpa kurung `A >= B inside {C}` ter-parse ulang sebagai
                    // `(A >= B) inside {C}` ≠ maksud AST.
                    BinOp::Inside => format!("({} inside {{{}}})", l.to_sv(w), r.to_sv(w)),
                    _ => format!("({} {} {})", l.to_sv(w), op.sym(), r.to_sv(w)),
                }
            }
        }
    }

    /// Kumpulkan tag fitur (nama op) untuk coverage guide.
    pub fn features(&self, out: &mut Vec<String>) {
        match self {
            Expr::Lit(_) | Expr::Var(_) => {}
            Expr::XLit { .. } => out.push("xlit".to_string()),
            Expr::Un(op, e) => {
                out.push(format!("un:{}", op.name()));
                e.features(out);
            }
            Expr::Ternary(c, t, f) => {
                out.push("ternary".to_string());
                c.features(out);
                t.features(out);
                f.features(out);
            }
            Expr::Repl(..) => out.push("repl".to_string()),
            Expr::BitSel(..) => out.push("bitsel".to_string()),
            Expr::PartSel(..) => out.push("partsel".to_string()),
            Expr::Bin(op, l, r) => {
                out.push(format!("bin:{}", op.name()));
                l.features(out);
                r.features(out);
            }
        }
    }

    /// Grammar-aware AST mutation (Superion/DIE inspired).
    ///
    /// Inspired by Superion (Paper #1) and DIE (Paper #6):
    /// - Aspect-preserving mutation: don't break structure
    /// - Subtree-level operations: replace, delete, duplicate, swap
    /// - Tree-based mutation works via replacing subtrees using ASTs
    ///   of parsed test inputs.
    ///
    /// The key insight from DIE: "mutation too aggressive destroys
    /// important structure before reaching deep code". So we preserve
    /// module structure, signal width, type compatibility while mutating.
    pub fn mutate(&mut self, w: u32, rng: &mut fastrand::Rng) {
        // Choose mutation strategy (Superion tree-based + aspect-preserving)
        let strategy = rng.usize(0..7);
        match strategy {
            0 => {
                // Superion: subtree replacement — replace entire node with
                // a fresh random subtree (grammar-aware generation).
                *self = gen_node(w, rng, 0);
            }
            1 => {
                // DIE: aspect-preserving leaf mutation — only change leaf
                // values, preserving tree structure.
                self.mutate_leaf_preserving(w, rng);
            }
            2 => {
                // Superion: subtree deletion — replace node with literal 0
                // (aspect-preserving: keeps type compatibility).
                *self = Expr::Lit(0);
            }
            3 => {
                // Superion: subtree duplication — wrap in unary NOT
                // (preserves width via aspect preservation).
                let original = self.clone();
                *self = Expr::Un(UnOp::Not, Box::new(original));
            }
            4 => {
                // Superion: subtree swap — swap operator in binary node
                // (aspect-preserving: compatible operators only).
                self.swap_operator(rng);
            }
            5 => {
                // Superion: dictionary mutation — inject boundary values
                // into leaves (aspect-preserving: width-compatible).
                self.inject_boundary_value(w, rng);
            }
            _ => {
                // Recurse into children (structure-preserving descent).
                self.mutate_child(w, rng);
            }
        }
    }

    /// Aspect-preserving leaf mutation (DIE Paper #6).
    /// Only mutate leaf values, preserving tree structure.
    fn mutate_leaf_preserving(&mut self, w: u32, rng: &mut fastrand::Rng) {
        match self {
            Expr::Lit(v) => {
                // Boundary values from gen.rs BOUNDARY_VALUES
                let boundaries = [
                    0u64,
                    1,
                    u64::MAX,
                    0x5555_5555_5555_5555,
                    0xAAAA_AAAA_AAAA_AAAA,
                ];
                *v = if rng.bool() {
                    boundaries[rng.usize(0..boundaries.len())] & mask_of(w)
                } else {
                    rng.u64(0..) & mask_of(w)
                };
            }
            Expr::Var(c) => *c = if rng.bool() { 'a' } else { 'b' },
            Expr::XLit { v, m } => {
                let mask = mask_of(w);
                *v = rng.u64(0..) & mask;
                *m = rng.u64(0..) & mask;
            }
            Expr::BitSel(c, idx) => {
                if rng.bool() {
                    *c = if rng.bool() { 'a' } else { 'b' };
                } else {
                    *idx %= w.max(1);
                }
            }
            Expr::PartSel(c, hi, lo) => {
                if rng.bool() {
                    *c = if rng.bool() { 'a' } else { 'b' };
                }
                *hi %= w.max(1);
                *lo %= w.max(1);
                if lo > hi {
                    std::mem::swap(lo, hi);
                }
            }
            Expr::Repl(count, _) => {
                let max_count = (128 / w.max(1)).clamp(1, 8);
                *count = rng.usize(1..=max_count as usize) as u32;
            }
            _ => {
                // Non-leaf: recurse to find a leaf
                self.mutate_child(w, rng);
            }
        }
    }

    /// Swap operator to a compatible one (aspect-preserving).
    fn swap_operator(&mut self, rng: &mut fastrand::Rng) {
        match self {
            Expr::Bin(op, _, _) => {
                // Only swap within compatible operator family
                let new_op = match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                        let choices = [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::Mod];
                        choices[rng.usize(0..choices.len())]
                    }
                    BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Xnor => {
                        let choices = [BinOp::And, BinOp::Or, BinOp::Xor, BinOp::Xnor];
                        choices[rng.usize(0..choices.len())]
                    }
                    BinOp::Shl | BinOp::Shr | BinOp::Sshl | BinOp::Sshr => {
                        let choices = [BinOp::Shl, BinOp::Shr, BinOp::Sshl, BinOp::Sshr];
                        choices[rng.usize(0..choices.len())]
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        let choices = [
                            BinOp::Eq,
                            BinOp::Ne,
                            BinOp::Lt,
                            BinOp::Le,
                            BinOp::Gt,
                            BinOp::Ge,
                        ];
                        choices[rng.usize(0..choices.len())]
                    }
                    BinOp::LogicAnd | BinOp::LogicOr => {
                        if *op == BinOp::LogicAnd {
                            BinOp::LogicOr
                        } else {
                            BinOp::LogicAnd
                        }
                    }
                    _ => return, // Don't swap non-compatible ops
                };
                *op = new_op;
            }
            Expr::Un(op, _) => {
                let new_op = match op {
                    UnOp::Not | UnOp::LogicNot => {
                        if *op == UnOp::Not {
                            UnOp::LogicNot
                        } else {
                            UnOp::Not
                        }
                    }
                    UnOp::RedAnd | UnOp::RedOr | UnOp::RedXor => {
                        let choices = [UnOp::RedAnd, UnOp::RedOr, UnOp::RedXor];
                        choices[rng.usize(0..choices.len())]
                    }
                    _ => return,
                };
                *op = new_op;
            }
            _ => {}
        }
    }

    /// Inject boundary values into expression (Superion dictionary mutation).
    fn inject_boundary_value(&mut self, w: u32, rng: &mut fastrand::Rng) {
        match self {
            Expr::Lit(v) => {
                let boundaries = [0u64, 1, 2, mask_of(w), mask_of(w) >> 1];
                *v = boundaries[rng.usize(0..boundaries.len())];
            }
            Expr::Bin(_, l, r) => {
                // Inject into random child
                if rng.bool() {
                    l.inject_boundary_value(w, rng);
                } else {
                    r.inject_boundary_value(w, rng);
                }
            }
            Expr::Un(_, e) => e.inject_boundary_value(w, rng),
            Expr::Ternary(c, t, f) => match rng.usize(0..3) {
                0 => c.inject_boundary_value(w, rng),
                1 => t.inject_boundary_value(w, rng),
                _ => f.inject_boundary_value(w, rng),
            },
            _ => {}
        }
    }

    /// Mutate a random child node (structure-preserving descent).
    fn mutate_child(&mut self, w: u32, rng: &mut fastrand::Rng) {
        match self {
            Expr::Un(_, e) => e.mutate(w, rng),
            Expr::Ternary(c, t, f) => match rng.usize(0..3) {
                0 => c.mutate(w, rng),
                1 => t.mutate(w, rng),
                _ => f.mutate(w, rng),
            },
            Expr::Bin(_, l, r) => {
                if rng.bool() {
                    l.mutate(w, rng);
                } else {
                    r.mutate(w, rng);
                }
            }
            Expr::Repl(_, e) => e.mutate(w, rng),
            _ => {
                // Leaf: apply leaf mutation
                self.mutate_leaf_preserving(w, rng);
            }
        }
    }
}

/// Bangun node ekspresi acak dengan kedalaman maksimum `depth`.
pub fn gen_node(w: u32, rng: &mut fastrand::Rng, depth: u32) -> Expr {
    if depth >= 5 {
        return leaf(w, rng);
    }
    let roll = rng.usize(0..12);
    match roll {
        // Leaf (termasuk X-literal ~10%): stimulus 4-state melatih jalur
        // X-propagation engine.
        0..=3 => {
            if rng.usize(0..10) == 0 {
                gen_xlit(w, rng)
            } else {
                leaf(w, rng)
            }
        }
        4..=5 => {
            let op = UnOp::all()[rng.usize(0..UnOp::all().len())];
            Expr::Un(op, Box::new(gen_node(w, rng, depth + 1)))
        }
        6..=8 => {
            let op = BinOp::all()[rng.usize(0..BinOp::all().len())];
            Expr::Bin(
                op,
                Box::new(gen_node(w, rng, depth + 1)),
                Box::new(gen_node(w, rng, depth + 1)),
            )
        }
        9 => Expr::Ternary(
            Box::new(gen_node(w, rng, depth + 1)),
            Box::new(gen_node(w, rng, depth + 1)),
            Box::new(gen_node(w, rng, depth + 1)),
        ),
        10 => {
            let max_count = (128 / w.max(1)).clamp(1, 8);
            let count = rng.usize(1..=max_count as usize) as u32;
            Expr::Repl(count, Box::new(gen_node(w, rng, depth + 1)))
        }
        _ => gen_sel(w, rng),
    }
}

/// Bit/part-select pada variabel — index selalu dalam range lebar `w`
/// (out-of-range muncul alami via mutasi lebar; golden menandai X).
fn gen_sel(w: u32, rng: &mut fastrand::Rng) -> Expr {
    let c = if rng.bool() { 'a' } else { 'b' };
    if w == 0 {
        return Expr::Var(c);
    }
    if rng.bool() {
        Expr::BitSel(c, rng.u32(0..w))
    } else {
        let hi = rng.u32(0..w);
        let lo = rng.u32(0..hi + 1);
        Expr::PartSel(c, hi, lo)
    }
}

/// Literal 4-state: subset bit ditandai `x`.
pub fn gen_xlit(w: u32, rng: &mut fastrand::Rng) -> Expr {
    let mask = mask_of(w);
    // Pastikan minimal satu bit x (else sama saja dengan Lit).
    let xmask = loop {
        let m = rng.u64(0..) & mask;
        if m != 0 || mask == 0 {
            break m;
        }
    };
    Expr::XLit {
        v: rng.u64(0..) & mask,
        m: xmask,
    }
}

fn leaf(w: u32, rng: &mut fastrand::Rng) -> Expr {
    if rng.bool() {
        Expr::Var(if rng.bool() { 'a' } else { 'b' })
    } else {
        Expr::Lit(rng.u64(0..) & mask_of(w))
    }
}
