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
}

impl UnOp {
    pub fn sym(self) -> &'static str {
        match self {
            UnOp::Not => "~",
            UnOp::LogicNot => "!",
            UnOp::Neg => "-",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            UnOp::Not => "Not",
            UnOp::LogicNot => "LogicNot",
            UnOp::Neg => "Neg",
        }
    }

    pub fn all() -> &'static [UnOp] {
        &[UnOp::Not, UnOp::LogicNot, UnOp::Neg]
    }
}

/// Ekspresi kombinasional terbatas: literal, variabel (a/b), unary, binary.
/// Selalu valid secara sintaksis saat di-render (`to_sv`) — ini yang membuat
/// fuzzer "tidak buta": input bukan byte acak, melainkan SV well-formed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Lit(u64),
    Var(char),
    Un(UnOp, Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
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
            Expr::Var(c) => {
                let x = if *c == 'a' { a } else { b };
                ((x & mask_of128(W)), W.max(w), false)
            }
            Expr::Un(op, e) => {
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
                }
            }
            Expr::Bin(op, l, r) => {
                // Anak concat: self-determined murni (ctx=0). Anak lainnya —
                // termasuk operan comparison/logical — context-determined
                // (warisi ctx; selaras propagate_context_width di elaborator
                // dan divalidasi differential).
                let self_det = *op == BinOp::Concat;
                let c = if self_det { 0 } else { w };
                let (x, lw, hx) = l.eval_w128(c, W, a, b);
                let (y, rw, hy) = r.eval_w128(c, W, a, b);
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
                        let cw = lw.max(rw);
                        let xe = x & mask_of128(cw);
                        let ye = y & mask_of128(cw);
                        if either_x {
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
                        // Mutual extension ke max lebar operan, TANPA konteks
                        // luar (divalidasi vs Icarus seed=332598).
                        let cw = lw.max(rw);
                        let xe = x & mask_of128(cw);
                        let ye = y & mask_of128(cw);
                        if either_x {
                            // X in comparison operand = X result
                            return (0, w.max(1), true);
                        }
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
            Expr::Lit(_) | Expr::Var(_) => w,
            Expr::Un(_, e) => e.max_width(w),
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
            Expr::Var(c) => c.to_string(),
            Expr::Un(op, e) => format!("{}({})", op.sym(), e.to_sv(w)),
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
            Expr::Un(op, e) => {
                out.push(format!("un:{}", op.name()));
                e.features(out);
            }
            Expr::Bin(op, l, r) => {
                out.push(format!("bin:{}", op.name()));
                l.features(out);
                r.features(out);
            }
        }
    }

    /// Ganti satu node acak (mutasi structure-aware untuk feedback loop).
    pub fn mutate(&mut self, w: u32, rng: &mut fastrand::Rng) {
        // Pilih satu node secara acak lalu ubah op/leaf-nya.
        // Pendekatan sederhana: rekursi dengan probabilitas ganti di root.
        if rng.bool() {
            *self = gen_node(w, rng, 0);
            return;
        }
        match self {
            Expr::Un(_, e) => e.mutate(w, rng),
            Expr::Bin(_, l, r) => {
                if rng.bool() {
                    l.mutate(w, rng);
                } else {
                    r.mutate(w, rng);
                }
            }
            Expr::Lit(v) => *v = rng.u64(0..) & mask_of(w),
            Expr::Var(c) => *c = if rng.bool() { 'a' } else { 'b' },
        }
    }
}

/// Bangun node ekspresi acak dengan kedalaman maksimum `depth`.
pub fn gen_node(w: u32, rng: &mut fastrand::Rng, depth: u32) -> Expr {
    if depth >= 4 {
        return leaf(w, rng);
    }
    let roll = rng.usize(0..10);
    match roll {
        0..=3 => leaf(w, rng),
        4..=5 => {
            let op = UnOp::all()[rng.usize(0..UnOp::all().len())];
            Expr::Un(op, Box::new(gen_node(w, rng, depth + 1)))
        }
        _ => {
            let op = BinOp::all()[rng.usize(0..BinOp::all().len())];
            Expr::Bin(
                op,
                Box::new(gen_node(w, rng, depth + 1)),
                Box::new(gen_node(w, rng, depth + 1)),
            )
        }
    }
}

fn leaf(w: u32, rng: &mut fastrand::Rng) -> Expr {
    if rng.bool() {
        Expr::Var(if rng.bool() { 'a' } else { 'b' })
    } else {
        Expr::Lit(rng.u64(0..) & mask_of(w))
    }
}
