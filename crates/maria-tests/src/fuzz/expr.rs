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
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Gt,
    LogicAnd,
    LogicOr,
}

impl BinOp {
    pub fn sym(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::And => "&",
            BinOp::Or => "|",
            BinOp::Xor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::LogicAnd => "&&",
            BinOp::LogicOr => "||",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            BinOp::Add => "Add",
            BinOp::Sub => "Sub",
            BinOp::Mul => "Mul",
            BinOp::And => "And",
            BinOp::Or => "Or",
            BinOp::Xor => "Xor",
            BinOp::Shl => "Shl",
            BinOp::Shr => "Shr",
            BinOp::Eq => "Eq",
            BinOp::Ne => "Ne",
            BinOp::Lt => "Lt",
            BinOp::Gt => "Gt",
            BinOp::LogicAnd => "LogicAnd",
            BinOp::LogicOr => "LogicOr",
        }
    }

    pub fn all() -> &'static [BinOp] {
        &[
            BinOp::Add,
            BinOp::Sub,
            BinOp::Mul,
            BinOp::And,
            BinOp::Or,
            BinOp::Xor,
            BinOp::Shl,
            BinOp::Shr,
            BinOp::Eq,
            BinOp::Ne,
            BinOp::Lt,
            BinOp::Gt,
            BinOp::LogicAnd,
            BinOp::LogicOr,
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
#[derive(Debug, Clone)]
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
    let m = mask_of(w);
    let val = v & m;
    let mut bits = String::with_capacity(w as usize);
    for i in (0..w).rev() {
        bits.push(if (val >> i) & 1 == 1 { '1' } else { '0' });
    }
    format!("{}'b{}", w, bits)
}

impl Expr {
    /// Evaluasi referensi width-aware (model emas). Mengembalikan
    /// `(nilai, lebar_sv)`. Aturan lebar SV (self-determined):
    /// perbandingan/relasional/logical → 1 bit; shift → lebar operand kiri;
    /// lainnya → max(lebar operan). Inilah yang membedakan fuzzer ini dari
    /// fuzzing buta: model emas memahami semantik lebar intermediate SV.
    fn eval_w(&self, w: u32, a: u64, b: u64) -> (u64, u32) {
        match self {
            Expr::Lit(v) => (v & mask_of(w), w),
            Expr::Var(c) => {
                let x = if *c == 'a' { a } else { b };
                (x & mask_of(w), w)
            }
            Expr::Un(op, e) => {
                let (x, ew) = e.eval_w(w, a, b);
                let m = mask_of(ew);
                match op {
                    UnOp::Not => ((!x) & m, ew),
                    UnOp::LogicNot => ((if x == 0 { 1 } else { 0 }), 1),
                    UnOp::Neg => (((!x).wrapping_add(1)) & m, ew),
                }
            }
            Expr::Bin(op, l, r) => {
                let (x, lw) = l.eval_w(w, a, b);
                let (y, rw) = r.eval_w(w, a, b);
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::And | BinOp::Or | BinOp::Xor => {
                        let ow = lw.max(rw);
                        let om = mask_of(ow);
                        let v = match op {
                            BinOp::Add => x.wrapping_add(y),
                            BinOp::Sub => x.wrapping_sub(y),
                            BinOp::Mul => x.wrapping_mul(y),
                            BinOp::And => x & y,
                            BinOp::Or => x | y,
                            BinOp::Xor => x ^ y,
                            _ => unreachable!(),
                        } & om;
                        (v, ow)
                    }
                    BinOp::Shl | BinOp::Shr => {
                        let ow = lw; // lebar hasil = operand kiri
                        let om = mask_of(ow);
                        let amt = y; // nilai rhs penuh (bukan dimask ke 63)
                        let v = if amt >= ow as u64 {
                            0u64
                        } else {
                            match op {
                                BinOp::Shl => (x << amt as u32) & om,
                                BinOp::Shr => (x >> amt as u32) & om,
                                _ => unreachable!(),
                            }
                        };
                        (v, ow)
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::LogicAnd | BinOp::LogicOr => {
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
                            BinOp::Gt => {
                                if x > y {
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
                            _ => unreachable!(),
                        };
                        (v, 1)
                    }
                }
            }
        }
    }

    /// Evaluasi akhir: hasil ekspresi di-assign ke `y` (lebar `w`) dengan
    /// zero-extension (unsigned). Nilai dikembalikan ter-mask ke `w`.
    pub fn eval(&self, w: u32, a: u64, b: u64) -> u64 {
        let (v, _rw) = self.eval_w(w, a, b);
        v & mask_of(w)
    }

    /// Render ke SystemVerilog. Setiap sub-ekspresi non-leaf dibungkus
    /// kurung agar selalu unambiguous.
    pub fn to_sv(&self, w: u32) -> String {
        match self {
            Expr::Lit(v) => lit_sv(*v, w),
            Expr::Var(c) => c.to_string(),
            Expr::Un(op, e) => format!("{}({})", op.sym(), e.to_sv(w)),
            Expr::Bin(op, l, r) => format!("({} {} {})", l.to_sv(w), op.sym(), r.to_sv(w)),
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
