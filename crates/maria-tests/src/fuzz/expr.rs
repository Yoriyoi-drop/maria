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
    EqWild,
    NeqWild,
    Power,
    Concat,
    Inside,
    Min,
    Max,
    Implies,
    Equiv,
    Dist,
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
            BinOp::EqWild => "==?",
            BinOp::NeqWild => "!=?",
            BinOp::Power => "**",
            BinOp::Concat => "{,}",
            BinOp::Inside => "inside",
            BinOp::Min => "min",
            BinOp::Max => "max",
            BinOp::Implies => "->",
            BinOp::Equiv => "<->",
            BinOp::Dist => "dist",
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
            BinOp::EqWild => "EqWild",
            BinOp::NeqWild => "NeqWild",
            BinOp::Power => "Power",
            BinOp::Concat => "Concat",
            BinOp::Inside => "Inside",
            BinOp::Min => "Min",
            BinOp::Max => "Max",
            BinOp::Implies => "Implies",
            BinOp::Equiv => "Equiv",
            BinOp::Dist => "Dist",
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
            BinOp::EqWild,
            BinOp::NeqWild,
            BinOp::Power,
            BinOp::Concat,
            BinOp::Inside,
            BinOp::Min,
            BinOp::Max,
            BinOp::Implies,
            BinOp::Equiv,
            BinOp::Dist,
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

fn lit_sv(v: u64, w: u32) -> String {
    if w == 0 {
        return "0".to_string();
    }
    let m = mask_of(w);
    let val = v & m;
    let mut bits = String::with_capacity(w as usize);
    for i in (0..w).rev() {
        let bit = if i >= 64 {
            0
        } else {
            (val >> i) & 1
        };
        bits.push(if bit == 1 { '1' } else { '0' });
    }
    format!("{}'b{}", w, bits)
}

impl Expr {
    /// Evaluasi referensi width-aware (model emas). Mengembalikan
    /// `(nilai, lebar_sv)`. Aturan lebar SV (self-determined):
    /// perbandingan/relasional/logical → 1 bit; shift → lebar operand kiri;
    /// lainnya → max(lebar operan). Inilah yang membedakan fuzzer ini dari
    /// fuzzing buta: model emas memahami semantik lebar intermediate SV.
    /// Returns (value, width, has_x) where has_x indicates X-state propagation
    /// (e.g. div/mod by zero). When has_x=true, oracle should skip comparison.
    fn eval_w(&self, w: u32, a: u64, b: u64) -> (u64, u32, bool) {
        match self {
            Expr::Lit(v) => (v & mask_of(w), w, false),
            Expr::Var(c) => {
                let x = if *c == 'a' { a } else { b };
                (x & mask_of(w), w, false)
            }
            Expr::Un(op, e) => {
                let (x, ew, hx) = e.eval_w(w, a, b);
                let m = mask_of(ew);
                if hx {
                    return (0, ew, true); // X propagates through any unary op
                }
                match op {
                    UnOp::Not => ((!x) & m, ew, false),
                    UnOp::LogicNot => ((if x == 0 { 1 } else { 0 }), 1, false),
                    UnOp::Neg => (((!x).wrapping_add(1)) & m, ew, false),
                }
            }
            Expr::Bin(op, l, r) => {
                let (x, lw, hx) = l.eval_w(w, a, b);
                let (y, rw, hy) = r.eval_w(w, a, b);
                let either_x = hx || hy;
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Xnor => {
                        let ow = lw.max(rw);
                        let om = mask_of(ow);
                        let v = match op {
                            BinOp::Add => x.wrapping_add(y),
                            BinOp::Sub => x.wrapping_sub(y),
                            BinOp::Mul => x.wrapping_mul(y),
                            BinOp::And => x & y,
                            BinOp::Or => x | y,
                            BinOp::Xor => x ^ y,
                            BinOp::Xnor => !(x ^ y),
                            _ => unreachable!(),
                        } & om;
                        (v, ow, either_x)
                    }
                    BinOp::Div => {
                        let ow = lw.max(rw);
                        if y == 0 {
                            (0, ow, true) // div by zero = X
                        } else if either_x {
                            (0, ow, true) // X propagates
                        } else {
                            (x / y, ow, false)
                        }
                    }
                    BinOp::Mod => {
                        let ow = lw.max(rw);
                        if y == 0 {
                            (0, ow, true) // mod by zero = X
                        } else if either_x {
                            (0, ow, true) // X propagates
                        } else {
                            (x % y, ow, false)
                        }
                    }
                    BinOp::Shl | BinOp::Shr | BinOp::Sshl | BinOp::Sshr => {
                        let ow = lw; // lebar hasil = operand kiri
                        let om = mask_of(ow);
                        if either_x {
                            (0, ow, true) // X propagates through shift
                        } else {
                            let amt = y; // nilai rhs penuh (bukan dimask ke 63)
                            let v = if amt >= ow as u64 || amt >= 64 {
                                0u64
                            } else {
                                match op {
                                    BinOp::Shl => (x << amt as u32) & om,
                                    BinOp::Shr => (x >> amt as u32) & om,
                                    BinOp::Sshl => (x << amt as u32) & om, // arithmetic shift left = logical
                                    BinOp::Sshr => {
                                        // arithmetic shift right: sign extend
                                        let msb = if ow == 64 { (x >> 63) & 1 } else { (x >> (ow - 1)) & 1 };
                                        if msb == 1 {
                                            // negative: fill with 1s
                                            let mask = if amt >= 64 { !0u64 } else { (!0u64) << amt };
                                            (x >> amt as u32) | (mask & om)
                                        } else {
                                            (x >> amt as u32) & om
                                        }
                                    }
                                    _ => unreachable!(),
                                }
                            };
                            (v, ow, false)
                        }
                    }
                    BinOp::Power => {
                        let ow = lw.max(rw);
                        let om = mask_of(ow);
                        if either_x {
                            (0, ow, true)
                        } else if y >= 64 {
                            (0, ow, false)
                        } else {
                            (x.wrapping_pow(y as u32) & om, ow, false)
                        }
                    }
                    BinOp::Concat => {
                        let ow = lw + rw;
                        let om = mask_of(ow);
                        let v = if rw >= 64 { 0 } else { (x << rw) | y };
                        (v & om, ow, either_x)
                    }
                    BinOp::Inside => {
                        if either_x {
                            return (0, 1, true);
                        }
                        let v = if x == y { 1 } else { 0 };
                        (v, 1, false)
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::LogicAnd | BinOp::LogicOr | BinOp::CaseEq | BinOp::CaseNeq | BinOp::EqWild | BinOp::NeqWild | BinOp::Min | BinOp::Max | BinOp::Implies | BinOp::Equiv | BinOp::Dist => {
                        if either_x {
                            // X in comparison operand = X result
                            return (0, 1, true);
                        }
                        let v = match op {
                            BinOp::Eq => {
                                if x == y {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::Ne => {
                                if x != y {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::Lt => {
                                if x < y {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::Le => {
                                if x <= y {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::Gt => {
                                if x > y {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::Ge => {
                                if x >= y {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::LogicAnd => {
                                if x != 0 && y != 0 {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::LogicOr => {
                                if x != 0 || y != 0 {
                                    1
                                } else {
                                    0
                                }
                            }
                            BinOp::CaseEq => {
                                if x == y { 1 } else { 0 }
                            }
                            BinOp::CaseNeq => {
                                if x != y { 1 } else { 0 }
                            }
                            BinOp::EqWild => {
                                if x == y { 1 } else { 0 }
                            }
                            BinOp::NeqWild => {
                                if x != y { 1 } else { 0 }
                            }
                            BinOp::Min => {
                                if x < y { x } else { y }
                            }
                            BinOp::Max => {
                                if x > y { x } else { y }
                            }
                            BinOp::Implies => {
                                // a -> b  equiv to  (!a) | b
                                if x == 0 || y != 0 { 1 } else { 0 }
                            }
                            BinOp::Equiv => {
                                // a <-> b  equiv to (a == b)
                                if x == y { 1 } else { 0 }
                            }
                            BinOp::Dist => {
                                // dist is a weighted distribution, simplified as equality for eval
                                if x == y { 1 } else { 0 }
                            }
                            _ => unreachable!(),
                        };
                        (v, 1, false)
                    }
                }
            }
        }
    }

    /// Evaluasi akhir: hasil ekspresi di-assign ke `y` (lebar `w`) dengan
    /// zero-extension (unsigned). Nilai dikembalikan ter-mask ke `w`.
    pub fn eval(&self, w: u32, a: u64, b: u64) -> u64 {
        let (v, _, _) = self.eval_w(w, a, b);
        v & mask_of(w)
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
                    BinOp::Inside => format!("{} inside {{{}}}", l.to_sv(w), r.to_sv(w)),
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
    if depth >= 3 {
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