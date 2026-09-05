//! Unified SV AST — satu representasi untuk semua fitur SystemVerilog.
//!
//! Satu file = satu tanggung jawab: definisi tipe, generasi acak, mutasi,
//! rendering ke source, dan evaluasi emas. Menggantikan 30+ mini-AST
//! terpisah (SExpr, TExpr, dll) dengan satu AST terpadu.
//!
//! Alur: `generate_svast(seed)` → `to_source()` → compile + sim →
//!        `eval_golden()` (combinational) / determinism check (sequential)

use fastrand::Rng;

// ── Ekspresi (kompatibel dengan Expr di expr.rs) ────────────────────────────

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
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Mod => "%",
            Self::And => "&",
            Self::Or => "|",
            Self::Xor => "^",
            Self::Xnor => "^~",
            Self::Shl => "<<",
            Self::Shr => ">>",
            Self::Sshl => "<<<",
            Self::Sshr => ">>>",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::LogicAnd => "&&",
            Self::LogicOr => "||",
            Self::CaseEq => "===",
            Self::CaseNeq => "!==",
            Self::Power => "**",
            Self::Concat => "{,}",
            Self::Inside => "inside",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    LogicNot,
    Neg,
    RedAnd,
    RedOr,
    RedXor,
    RedNand,
    RedNor,
    RedXnor,
}

impl UnOp {
    pub fn sym(self) -> &'static str {
        match self {
            Self::Not => "~",
            Self::LogicNot => "!",
            Self::Neg => "-",
            Self::RedAnd => "&",
            Self::RedOr => "|",
            Self::RedXor => "^",
            Self::RedNand => "~&",
            Self::RedNor => "~|",
            Self::RedXnor => "^~",
        }
    }

    pub fn is_reduction(self) -> bool {
        matches!(
            self,
            Self::RedAnd
                | Self::RedOr
                | Self::RedXor
                | Self::RedNand
                | Self::RedNor
                | Self::RedXnor
        )
    }
}

/// Ekspresi — persis sama semantik dengan Expr di expr.rs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Lit(u64),
    XLit { v: u64, m: u64 },
    Var(char),
    Un(UnOp, Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Repl(u32, Box<Expr>),
    BitSel(char, u32),
    PartSel(char, u32, u32),
}

// ── Pernyataan ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignKind {
    Blocking,
    NonBlocking,
}

#[derive(Debug, Clone)]
pub enum SensEdge {
    Posedge,
    Negedge,
}

#[derive(Debug, Clone)]
pub struct SensEntry {
    pub signal: String,
    pub edge: Option<SensEdge>,
}

/// Pernyataan di dalam always/initial block.
#[derive(Debug, Clone)]
pub enum Stmt {
    VarDecl {
        name: String,
        width: u32,
    },
    Assign {
        lhs: String,
        rhs: Expr,
        kind: AssignKind,
    },
    If {
        cond: Expr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
    },
    Case {
        expr: Expr,
        items: Vec<(Vec<u64>, Vec<Stmt>)>,
        default: Option<Vec<Stmt>>,
    },
    For {
        var: String,
        from: Expr,
        to: Expr,
        body: Vec<Stmt>,
    },
    Display {
        format: String,
        args: Vec<String>,
    },
    Delay(u32),
}

/// Blok kombinasional (`assign y = expr`).
#[derive(Debug, Clone)]
pub struct CombAssign {
    pub expr: Expr,
}

/// Blok sequential (`always_ff @(posedge clk) ...`).
#[derive(Debug, Clone)]
pub struct SeqBlock {
    pub sensitivity: Vec<SensEntry>,
    pub body: Vec<Stmt>,
}

/// Blok konkuren (`fork/join ... join_any ... join_none`).
#[derive(Debug, Clone)]
pub enum ForkMode {
    Join,
    JoinAny,
    JoinNone,
}

#[derive(Debug, Clone)]
pub struct ForkJoin {
    pub mode: ForkMode,
    pub branches: Vec<Vec<Stmt>>,
}

/// Constraint dalam class (ekspresi boolean).
#[derive(Debug, Clone)]
pub struct Constraint {
    pub name: String,
    pub body: Vec<Expr>,
}

/// Class definition (UVM-style).
#[derive(Debug, Clone)]
pub struct SvClass {
    pub name: String,
    pub rand_fields: Vec<(String, u32)>, // (name, width)
    pub constraints: Vec<Constraint>,
}

/// Generate block (`generate for` / `generate if`).
#[derive(Debug, Clone)]
pub enum GenKind {
    For {
        var: String,
        bound: u32,
        body: Vec<Stmt>,
    },
    If {
        cond: Expr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
    },
}

/// Modul — kontainer utama test case.
#[derive(Debug, Clone)]
pub struct SvModule {
    pub params: Vec<(String, u32)>, // (name, default_width)
    pub width: u32,                 // lebar data path
    pub comb: Option<CombAssign>,
    pub seq: Option<SeqBlock>,
    pub fork_join: Option<ForkJoin>,
    pub class: Option<SvClass>,
    pub gen_blocks: Vec<GenKind>,
    pub extra_stmts: Vec<Stmt>,
}

/// Top-level AST gabungan.
#[derive(Debug, Clone)]
pub enum SVAst {
    Module(SvModule),
}

// ── Lebar & mask ────────────────────────────────────────────────────────────

pub const WIDTH_CHOICES: [u32; 18] = [
    1, 2, 3, 4, 7, 8, 15, 16, 17, 31, 32, 33, 63, 64, 65, 72, 96, 128,
];

const BOUNDARY_VALUES: [u64; 8] = [
    0,
    1,
    2,
    u64::MAX,
    0x5555_5555_5555_5555,
    0xAAAA_AAAA_AAAA_AAAA,
    0x8000_0000_0000_0000,
    0xFFFF_FFFF_FFFF_FFFF,
];

pub fn mask_of(w: u32) -> u64 {
    if w >= 64 {
        u64::MAX
    } else {
        (1u64 << w) - 1
    }
}

fn mask_of128(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

/// Apakah ekspresi mengandung X-literal? Dipakai test golden untuk skip
/// ekspresi 4-state (eval_has_x tidak selalu menangkap X di dalam
/// operator logical yang di-treat-as-false oleh golden).
fn has_x_literal(e: &Expr) -> bool {
    match e {
        Expr::XLit { .. } => true,
        Expr::Un(_, x) => has_x_literal(x),
        Expr::Ternary(c, t, f) => has_x_literal(c) || has_x_literal(t) || has_x_literal(f),
        Expr::Repl(_, x) => has_x_literal(x),
        Expr::Bin(_, l, r) => has_x_literal(l) || has_x_literal(r),
        _ => false,
    }
}

pub fn lit_sv(v: u64, w: u32) -> String {
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

fn pick_boundary(rng: &mut Rng, w: u32) -> u64 {
    if rng.usize(0..4) == 0 {
        BOUNDARY_VALUES[rng.usize(0..BOUNDARY_VALUES.len())] & mask_of(w)
    } else {
        rng.u64(0..) & mask_of(w)
    }
}

// ── Render Expr → SV ────────────────────────────────────────────────────────

impl Expr {
    pub fn to_sv(&self, w: u32) -> String {
        match self {
            Expr::Lit(v) => lit_sv(*v, w),
            Expr::XLit { v, m } => {
                let mut bits = String::with_capacity(w as usize);
                for i in (0..w).rev() {
                    let bi = i as u64;
                    if bi < 64 && (m >> bi) & 1 == 1 {
                        bits.push('x');
                    } else {
                        let bit = if bi >= 64 { 0 } else { (*v >> bi) & 1 };
                        bits.push(if bit == 1 { '1' } else { '0' });
                    }
                }
                format!("{}'b{}", w, bits)
            }
            Expr::Var(c) => c.to_string(),
            Expr::Un(op, e) => format!("{}({})", op.sym(), e.to_sv(w)),
            Expr::Ternary(c, t, f) => {
                format!("(({}) ? ({}) : ({}))", c.to_sv(w), t.to_sv(w), f.to_sv(w))
            }
            Expr::Repl(count, e) => format!("{{{}{{{}}}}}", count, e.to_sv(w)),
            Expr::BitSel(c, idx) => format!("{}[{}]", c, idx),
            Expr::PartSel(c, hi, lo) => format!("{}[{}:{}]", c, hi, lo),
            Expr::Bin(op, l, r) => match *op {
                BinOp::Concat => format!("{{{}, {}}}", l.to_sv(w), r.to_sv(w)),
                BinOp::Inside => format!("({} inside {{{}}})", l.to_sv(w), r.to_sv(w)),
                _ => format!("({} {} {})", l.to_sv(w), op.sym(), r.to_sv(w)),
            },
        }
    }
}

// ── Fitur tagging (coverage) ────────────────────────────────────────────────

impl Expr {
    pub fn features(&self, out: &mut Vec<String>) {
        match self {
            Expr::Lit(_) | Expr::Var(_) => {}
            Expr::XLit { .. } => out.push("xlit".into()),
            Expr::Un(op, e) => {
                out.push(format!("un:{:?}", op));
                e.features(out);
            }
            Expr::Ternary(c, t, f) => {
                out.push("ternary".into());
                c.features(out);
                t.features(out);
                f.features(out);
            }
            Expr::Repl(..) => out.push("repl".into()),
            Expr::BitSel(..) => out.push("bitsel".into()),
            Expr::PartSel(..) => out.push("partsel".into()),
            Expr::Bin(op, l, r) => {
                out.push(format!("bin:{:?}", op));
                l.features(out);
                r.features(out);
            }
        }
    }
}

// ── Generasi ekspresi acak ──────────────────────────────────────────────────

pub fn gen_expr(w: u32, rng: &mut Rng, depth: u32) -> Expr {
    if depth >= 5 {
        return gen_leaf(w, rng);
    }
    match rng.usize(0..12) {
        0..=3 => {
            if rng.usize(0..10) == 0 {
                gen_xlit(w, rng)
            } else {
                gen_leaf(w, rng)
            }
        }
        4..=5 => {
            let ops = [
                UnOp::Not,
                UnOp::LogicNot,
                UnOp::Neg,
                UnOp::RedAnd,
                UnOp::RedOr,
                UnOp::RedXor,
                UnOp::RedNand,
                UnOp::RedNor,
                UnOp::RedXnor,
            ];
            Expr::Un(
                ops[rng.usize(0..ops.len())],
                Box::new(gen_expr(w, rng, depth + 1)),
            )
        }
        6..=8 => {
            let ops = [
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
            ];
            let op = ops[rng.usize(0..ops.len())];
            Expr::Bin(
                op,
                Box::new(gen_expr(w, rng, depth + 1)),
                Box::new(gen_expr(w, rng, depth + 1)),
            )
        }
        9 => Expr::Ternary(
            Box::new(gen_expr(w, rng, depth + 1)),
            Box::new(gen_expr(w, rng, depth + 1)),
            Box::new(gen_expr(w, rng, depth + 1)),
        ),
        10 => {
            let max_count = (128 / w.max(1)).clamp(1, 8);
            let count = rng.usize(1..=max_count as usize) as u32;
            Expr::Repl(count, Box::new(gen_expr(w, rng, depth + 1)))
        }
        _ => gen_sel(w, rng),
    }
}

fn gen_leaf(w: u32, rng: &mut Rng) -> Expr {
    if rng.bool() {
        Expr::Var(if rng.bool() { 'a' } else { 'b' })
    } else {
        Expr::Lit(rng.u64(0..) & mask_of(w))
    }
}

fn gen_xlit(w: u32, rng: &mut Rng) -> Expr {
    let m = mask_of(w);
    let xmask = loop {
        let xm = rng.u64(0..) & m;
        if xm != 0 || m == 0 {
            break xm;
        }
    };
    Expr::XLit {
        v: rng.u64(0..) & m,
        m: xmask,
    }
}

fn gen_sel(w: u32, rng: &mut Rng) -> Expr {
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

// ── Mutasi ekspresi (grammar-aware) ────────────────────────────────────────

impl Expr {
    pub fn mutate(&mut self, w: u32, rng: &mut Rng) {
        match rng.usize(0..7) {
            0 => *self = gen_expr(w, rng, 0),         // subtree replace
            1 => self.mutate_leaf_preserving(w, rng), // DIE leaf
            2 => *self = Expr::Lit(0),                // subtree delete
            3 => {
                let orig = self.clone();
                *self = Expr::Un(UnOp::Not, Box::new(orig));
            }
            4 => self.swap_operator(rng),      // operator swap
            5 => self.inject_boundary(w, rng), // boundary injection
            _ => self.mutate_child(w, rng),    // recursive descent
        }
    }

    fn mutate_leaf_preserving(&mut self, w: u32, rng: &mut Rng) {
        match self {
            Expr::Lit(v) => {
                *v = if rng.bool() {
                    BOUNDARY_VALUES[rng.usize(0..BOUNDARY_VALUES.len())] & mask_of(w)
                } else {
                    rng.u64(0..) & mask_of(w)
                };
            }
            Expr::Var(c) => *c = if rng.bool() { 'a' } else { 'b' },
            Expr::XLit { v, m } => {
                let mask = mask_of(w);
                *v = rng.u64(0..) & mask;
                *m = loop {
                    let xm = rng.u64(0..) & mask;
                    if xm != 0 {
                        break xm;
                    }
                };
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
                *count = rng.usize(1..=(128 / w.max(1)).clamp(1, 8) as usize) as u32;
            }
            _ => self.mutate_child(w, rng),
        }
    }

    fn swap_operator(&mut self, rng: &mut Rng) {
        match self {
            Expr::Bin(op, _, _) => {
                *op = match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                        let c = [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::Mod];
                        c[rng.usize(0..c.len())]
                    }
                    BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Xnor => {
                        let c = [BinOp::And, BinOp::Or, BinOp::Xor, BinOp::Xnor];
                        c[rng.usize(0..c.len())]
                    }
                    BinOp::Shl | BinOp::Shr | BinOp::Sshl | BinOp::Sshr => {
                        let c = [BinOp::Shl, BinOp::Shr, BinOp::Sshl, BinOp::Sshr];
                        c[rng.usize(0..c.len())]
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        let c = [
                            BinOp::Eq,
                            BinOp::Ne,
                            BinOp::Lt,
                            BinOp::Le,
                            BinOp::Gt,
                            BinOp::Ge,
                        ];
                        c[rng.usize(0..c.len())]
                    }
                    BinOp::LogicAnd | BinOp::LogicOr => {
                        if *op == BinOp::LogicAnd {
                            BinOp::LogicOr
                        } else {
                            BinOp::LogicAnd
                        }
                    }
                    _ => return,
                };
            }
            Expr::Un(op, _) => {
                *op = match op {
                    UnOp::Not | UnOp::LogicNot => {
                        if *op == UnOp::Not {
                            UnOp::LogicNot
                        } else {
                            UnOp::Not
                        }
                    }
                    UnOp::RedAnd | UnOp::RedOr | UnOp::RedXor => {
                        let c = [UnOp::RedAnd, UnOp::RedOr, UnOp::RedXor];
                        c[rng.usize(0..c.len())]
                    }
                    _ => return,
                };
            }
            _ => {}
        }
    }

    fn inject_boundary(&mut self, w: u32, rng: &mut Rng) {
        match self {
            Expr::Lit(v) => {
                let b = [0u64, 1, 2, mask_of(w), mask_of(w) >> 1];
                *v = b[rng.usize(0..b.len())];
            }
            Expr::Bin(_, l, r) => {
                if rng.bool() {
                    l.inject_boundary(w, rng);
                } else {
                    r.inject_boundary(w, rng);
                }
            }
            Expr::Un(_, e) => e.inject_boundary(w, rng),
            Expr::Ternary(c, t, f) => match rng.usize(0..3) {
                0 => c.inject_boundary(w, rng),
                1 => t.inject_boundary(w, rng),
                _ => f.inject_boundary(w, rng),
            },
            _ => {}
        }
    }

    fn mutate_child(&mut self, w: u32, rng: &mut Rng) {
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
            _ => self.mutate_leaf_preserving(w, rng),
        }
    }
}

// ── Evaluasi emas (model referensi) ─────────────────────────────────────────

impl Expr {
    /// Evaluasi akhir: hasil ekspresi di-assign ke y (lebar w), zero-extend.
    pub fn eval(&self, w: u32, a: u64, b: u64) -> u64 {
        let (v, _, _) = self.eval_w128(w, w, a as u128, b as u128);
        ((v & mask_of128(w)) & mask_of128(64)) as u64
    }

    pub fn eval_has_x(&self, w: u32, a: u64, b: u64) -> bool {
        let (_, _, hx) = self.eval_w128(w, w, a as u128, b as u128);
        hx
    }

    fn eval_w128(&self, w: u32, W: u32, a: u128, b: u128) -> (u128, u32, bool) {
        match self {
            Expr::Lit(v) => ((*v as u128) & mask_of128(W), W.max(w), false),
            Expr::XLit { v, m } => {
                let mask = mask_of128(W.max(w));
                ((*v as u128) & mask, W.max(w), true)
            }
            Expr::Var(c) => {
                let x = if *c == 'a' { a } else { b };
                ((x & mask_of128(W)), W.max(w), false)
            }
            Expr::Un(op, e) => {
                if op.is_reduction() {
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
                    _ => unreachable!(),
                }
            }
            Expr::Bin(op, l, r) => {
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
                            (0, ow, true)
                        } else if either_x {
                            (0, ow, true)
                        } else {
                            (xe / ye, ow, false)
                        }
                    }
                    BinOp::Mod => {
                        let ow = lw.max(rw).max(w);
                        let ye = y & mask_of128(ow);
                        let xe = x & mask_of128(ow);
                        if ye == 0 {
                            (0, ow, true)
                        } else if either_x {
                            (0, ow, true)
                        } else {
                            (xe % ye, ow, false)
                        }
                    }
                    BinOp::Shl | BinOp::Shr | BinOp::Sshl | BinOp::Sshr => {
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
                                    BinOp::Shl | BinOp::Sshl => (xe << amt) & om,
                                    BinOp::Shr | BinOp::Sshr => (xe >> amt) & om,
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
                        (((x << sh) | y) & om, ow, either_x)
                    }
                    BinOp::Inside => {
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
                        ((if xe == ye { 1 } else { 0 }, w.max(1), false))
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
                        let (_, lw0, _) = l.eval_w128(cl, W, a, b);
                        let (_, rw0, _) = r.eval_w128(cr, W, a, b);
                        let (x1, lw1, _) = l.eval_w128(cl.max(rw0), W, a, b);
                        let (y1, rw1, _) = r.eval_w128(cr.max(lw0), W, a, b);
                        let (x, lw, hx) = l.eval_w128(cl.max(rw1), W, a, b);
                        let (y, rw, hy) = r.eval_w128(cr.max(lw1), W, a, b);
                        if hx || hy {
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
                        (v, w.max(1), false)
                    }
                }
            }
            Expr::Ternary(cond, t, f) => {
                let (cv, _, chx) = cond.eval_w128(0, W, a, b);
                let (tv, tw, thx) = t.eval_w128(w, W, a, b);
                let (fv, fw, fhx) = f.eval_w128(w, W, a, b);
                let ow = tw.max(fw).max(w);
                let om = mask_of128(ow);
                let truthy = cv != 0;
                let chosen = if truthy { tv } else { fv };
                let hx = thx || fhx || (chx && (tv & om) != (fv & om));
                (chosen & om, ow, hx)
            }
            Expr::Repl(count, e) => {
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
                let x = if *c == 'a' { a } else { b };
                if *idx >= W {
                    return (0, 1, true);
                }
                let v = if *idx >= 128 { 0 } else { (x >> *idx) & 1 };
                (v, 1, false)
            }
            Expr::PartSel(c, hi, lo) => {
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
}

// ── Generasi pernyataan ─────────────────────────────────────────────────────

fn gen_stmt(w: u32, rng: &mut Rng, depth: u32) -> Stmt {
    if depth >= 3 {
        return gen_simple_stmt(w, rng);
    }
    match rng.usize(0..7) {
        0..=1 => gen_simple_stmt(w, rng),
        2 => {
            let cond = gen_expr(w, rng, 0);
            let then_body = vec![gen_simple_stmt(w, rng)];
            let else_body = if rng.bool() {
                Some(vec![gen_simple_stmt(w, rng)])
            } else {
                None
            };
            Stmt::If {
                cond,
                then_body,
                else_body,
            }
        }
        3 => {
            let n_cases = rng.usize(2..=4);
            let mut items = Vec::new();
            for _ in 0..n_cases {
                let val = rng.u64(0..mask_of(w));
                items.push((vec![val], vec![gen_simple_stmt(w, rng)]));
            }
            let default = if rng.bool() {
                Some(vec![gen_simple_stmt(w, rng)])
            } else {
                None
            };
            Stmt::Case {
                expr: gen_expr(w, rng, 0),
                items,
                default,
            }
        }
        4 => {
            let bound = rng.u32(1..=8);
            Stmt::For {
                var: "i".to_string(),
                from: Expr::Lit(0),
                to: Expr::Lit(bound as u64),
                body: vec![gen_simple_stmt(w, rng)],
            }
        }
        _ => Stmt::Display {
            format: "%0h".to_string(),
            args: vec!["y".to_string()],
        },
    }
}

fn gen_simple_stmt(w: u32, rng: &mut Rng) -> Stmt {
    match rng.usize(0..4) {
        0 => Stmt::Assign {
            lhs: "y".to_string(),
            rhs: gen_expr(w, rng, 2),
            kind: if rng.bool() {
                AssignKind::Blocking
            } else {
                AssignKind::NonBlocking
            },
        },
        1 => Stmt::Assign {
            lhs: "y".to_string(),
            rhs: Expr::Bin(
                BinOp::Add,
                Box::new(Expr::Var('a')),
                Box::new(Expr::Var('b')),
            ),
            kind: AssignKind::Blocking,
        },
        2 => Stmt::Delay(1),
        _ => Stmt::Display {
            format: "%0h".to_string(),
            args: vec!["y".to_string()],
        },
    }
}

// ── Render pernyataan → SV ──────────────────────────────────────────────────

fn stmt_to_sv(s: &Stmt, indent: &str) -> String {
    match s {
        Stmt::VarDecl { name, width } => format!("{}reg [{}:0] {};", indent, width - 1, name),
        Stmt::Assign { lhs, rhs, kind } => {
            let op = match kind {
                AssignKind::Blocking => "=",
                AssignKind::NonBlocking => "<=",
            };
            format!("{}{} {} {};", indent, lhs, op, rhs.to_sv(32))
        }
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            let mut out = format!("{}if ({}) begin\n", indent, cond.to_sv(32));
            for s in then_body {
                out.push_str(&stmt_to_sv(s, &format!("{}    ", indent)));
                out.push('\n');
            }
            if let Some(eb) = else_body {
                out.push_str(&format!("{}end else begin\n", indent));
                for s in eb {
                    out.push_str(&stmt_to_sv(s, &format!("{}    ", indent)));
                    out.push('\n');
                }
            }
            out.push_str(&format!("{}end\n", indent));
            out
        }
        Stmt::Case {
            expr,
            items,
            default,
        } => {
            let mut out = format!("{}case ({})\n", indent, expr.to_sv(32));
            for (vals, body) in items {
                let pat = vals
                    .iter()
                    .map(|v| format!("{}", v))
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!("{}    {}: ", indent, pat));
                for s in body {
                    out.push_str(&stmt_to_sv(s, &format!("{}        ", indent)));
                }
                out.push('\n');
            }
            if let Some(db) = default {
                out.push_str(&format!("{}    default: ", indent));
                for s in db {
                    out.push_str(&stmt_to_sv(s, &format!("{}            ", indent)));
                }
                out.push('\n');
            }
            out.push_str(&format!("{}endcase\n", indent));
            out
        }
        Stmt::For {
            var,
            from,
            to,
            body,
        } => {
            let mut out = format!(
                "{}for ({} = {}; {} < {}; {} = {} + 1) begin\n",
                indent,
                var,
                from.to_sv(32),
                var,
                to.to_sv(32),
                var,
                var
            );
            for s in body {
                out.push_str(&stmt_to_sv(s, &format!("{}    ", indent)));
                out.push('\n');
            }
            out.push_str(&format!("{}end\n", indent));
            out
        }
        Stmt::Display { format: fmt, args } => {
            let arg_str = args.iter().map(|a| format!(", {}", a)).collect::<String>();
            format!("{}$display(\"{}\"{});\n", indent, fmt, arg_str)
        }
        Stmt::Delay(n) => format!("{}#{};\n", indent, n),
    }
}

// ── Render top-level → SV source ────────────────────────────────────────────

impl SVAst {
    pub fn to_source(&self) -> String {
        match self {
            SVAst::Module(m) => m.render(),
        }
    }
}

impl SvModule {
    fn render(&self) -> String {
        let w = self.width;
        let hi = w - 1;
        let mut out = String::new();

        // Header
        out.push_str("module fuzz_mod;\n");
        out.push_str(&format!("    reg [{}:0] a;\n", hi));
        out.push_str(&format!("    reg [{}:0] b;\n", hi));

        // Params
        for (pname, pval) in &self.params {
            out.push_str(&format!(
                "    parameter [{}:0] {} = {};\n",
                pval - 1,
                pname,
                pval
            ));
        }

        // y: wire bila hanya continuous/combinational assign, reg bila
        // di-drive dari procedural (fork/join atau always with blocking).
        let y_is_reg = self.fork_join.is_some()
            || self.seq.as_ref().is_some_and(|s| {
                s.body.iter().any(|st| {
                    matches!(
                        st,
                        Stmt::Assign {
                            kind: AssignKind::Blocking,
                            ..
                        }
                    )
                })
            });
        if y_is_reg {
            out.push_str(&format!("    reg [{}:0] y;\n", hi));
        } else {
            out.push_str(&format!("    wire [{}:0] y;\n", hi));
        }

        // Class
        if let Some(cls) = &self.class {
            out.push_str(&cls.render());
        }

        // Continuous assignment
        if let Some(comb) = &self.comb {
            out.push_str(&format!("    assign y = {};\n", comb.expr.to_sv(w)));
        }

        // Always block
        if let Some(seq) = &self.seq {
            out.push_str(&self.render_seq(seq));
        }

        // Generate blocks
        for gb in &self.gen_blocks {
            out.push_str(&self.render_gen(gb));
        }

        // Fork/join
        if let Some(fj) = &self.fork_join {
            out.push_str(&self.render_fork(fj));
        }

        // Extra statements (inside initial)
        out.push_str("    initial begin\n");
        out.push_str(&format!("        a = {};\n", lit_sv(0, w)));
        out.push_str(&format!("        b = {};\n", lit_sv(0, w)));
        for s in &self.extra_stmts {
            out.push_str(&stmt_to_sv(s, "        "));
        }
        if self.comb.is_none() && self.seq.is_none() && self.fork_join.is_none() {
            // Combinational-only: add default assign
            out.push_str(&format!("        y = a + b;\n"));
        }
        out.push_str("        #10;\n");
        out.push_str("        $finish;\n");
        out.push_str("    end\n");

        out.push_str("endmodule\n");
        out
    }

    fn render_seq(&self, seq: &SeqBlock) -> String {
        let mut out = String::new();
        // Build sensitivity list
        let sens: Vec<String> = seq
            .sensitivity
            .iter()
            .map(|s| match &s.edge {
                Some(SensEdge::Posedge) => format!("posedge {}", s.signal),
                Some(SensEdge::Negedge) => format!("negedge {}", s.signal),
                None => s.signal.clone(),
            })
            .collect();
        let sens_str = if sens.is_empty() {
            "*".to_string()
        } else {
            sens.join(", ")
        };

        out.push_str(&format!("    always @({}) begin\n", sens_str));
        for s in &seq.body {
            out.push_str(&stmt_to_sv(s, "        "));
        }
        out.push_str("    end\n");
        out
    }

    fn render_gen(&self, gb: &GenKind) -> String {
        match gb {
            GenKind::For { var, bound, body } => {
                let mut out = format!("    genvar {};\n", var);
                out.push_str(&format!("    generate\n"));
                out.push_str(&format!(
                    "        for ({var} = 0; {var} < {bound}; {var} = {var} + 1) begin : gen_blk\n"
                ));
                for s in body {
                    out.push_str(&stmt_to_sv(s, "            "));
                }
                out.push_str("        end\n");
                out.push_str("    endgenerate\n");
                out
            }
            GenKind::If {
                cond,
                then_body,
                else_body,
            } => {
                let mut out = format!("    generate\n");
                out.push_str(&format!(
                    "        if ({}) begin : gen_if\n",
                    cond.to_sv(self.width)
                ));
                for s in then_body {
                    out.push_str(&stmt_to_sv(s, "            "));
                }
                if let Some(eb) = else_body {
                    out.push_str("        end else begin : gen_else\n");
                    for s in eb {
                        out.push_str(&stmt_to_sv(s, "            "));
                    }
                }
                out.push_str("        end\n");
                out.push_str("    endgenerate\n");
                out
            }
        }
    }

    fn render_fork(&self, fj: &ForkJoin) -> String {
        let kw = match fj.mode {
            ForkMode::Join => "join",
            ForkMode::JoinAny => "join_any",
            ForkMode::JoinNone => "join_none",
        };
        let mut out = format!("    fork\n");
        for (i, branch) in fj.branches.iter().enumerate() {
            out.push_str(&format!("    begin : fork_branch_{}\n", i));
            for s in branch {
                out.push_str(&stmt_to_sv(s, "        "));
            }
            out.push_str("    end\n");
        }
        out.push_str(&format!("    {}\n", kw));
        out
    }
}

impl SvClass {
    fn render(&self) -> String {
        let mut out = format!("    class {};\n", self.name);
        for (fname, fwidth) in &self.rand_fields {
            out.push_str(&format!("        rand bit [{}:0] {};\n", fwidth - 1, fname));
        }
        for c in &self.constraints {
            out.push_str(&format!("        constraint {} {{", c.name));
            for (i, expr) in c.body.iter().enumerate() {
                if i > 0 {
                    out.push_str(" && ");
                }
                out.push_str(&expr.to_sv(32));
            }
            out.push_str("}\n");
        }
        out.push_str("    endclass\n");
        out
    }
}

// ── Generasi AST lengkap ────────────────────────────────────────────────────

/// Pilihan mode pembangkitan — menentukan blind spot mana yang dikeksplorasi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GenMode {
    /// Kombinasional murni (sudah tercakup Expr lama, tapi baseline).
    Combinational,
    /// Sequential: always_ff / always_comb dengan sensitivity list.
    Sequential,
    /// Concurren: fork/join.
    ForkJoin,
    /// Class + constraint (UVM-like).
    Class,
    /// Generate block (generate for / generate if).
    Generate,
}

impl GenMode {
    pub fn all() -> &'static [GenMode] {
        &[
            Self::Combinational,
            Self::Sequential,
            Self::ForkJoin,
            Self::Class,
            Self::Generate,
        ]
    }
}

/// Generate satu SVAst acak dari seed.
pub fn generate_svast(seed: u64) -> SVAst {
    let mut rng = Rng::with_seed(seed);
    let w = WIDTH_CHOICES[rng.usize(0..WIDTH_CHOICES.len())];
    let mode = GenMode::all()[rng.usize(0..GenMode::all().len())];

    let mut m = SvModule {
        params: Vec::new(),
        width: w,
        comb: None,
        seq: None,
        fork_join: None,
        class: None,
        gen_blocks: Vec::new(),
        extra_stmts: Vec::new(),
    };

    match mode {
        GenMode::Combinational => {
            m.comb = Some(CombAssign {
                expr: gen_expr(w, &mut rng, 0),
            });
        }
        GenMode::Sequential => {
            let sens = vec![SensEntry {
                signal: "clk".to_string(),
                edge: Some(SensEdge::Posedge),
            }];
            // always_ff with posedge clk
            let body = vec![
                Stmt::VarDecl {
                    name: "y_reg".to_string(),
                    width: w,
                },
                Stmt::Assign {
                    lhs: "y_reg".to_string(),
                    rhs: gen_expr(w, &mut rng, 2),
                    kind: AssignKind::NonBlocking,
                },
            ];
            m.seq = Some(SeqBlock {
                sensitivity: sens,
                body,
            });
            // Connect y to y_reg via assign
            m.comb = Some(CombAssign {
                expr: Expr::Var('a'), // placeholder — y_reg will drive y
            });
        }
        GenMode::ForkJoin => {
            let n_branches = rng.usize(2..=4);
            let mut branches = Vec::new();
            for _ in 0..n_branches {
                let branch = vec![
                    Stmt::Assign {
                        lhs: "y".to_string(),
                        rhs: gen_expr(w, &mut rng, 1),
                        kind: AssignKind::Blocking,
                    },
                    Stmt::Delay(1),
                ];
                branches.push(branch);
            }
            let mode = match rng.usize(0..3) {
                0 => ForkMode::Join,
                1 => ForkMode::JoinAny,
                _ => ForkMode::JoinNone,
            };
            m.fork_join = Some(ForkJoin { mode, branches });
        }
        GenMode::Class => {
            let n_fields = rng.usize(1..=3);
            let mut fields = Vec::new();
            for i in 0..n_fields {
                fields.push((format!("x{}", i), w));
            }
            let mut constraints = Vec::new();
            // Equality constraint
            constraints.push(Constraint {
                name: "c_eq".to_string(),
                body: vec![Expr::Bin(
                    BinOp::Eq,
                    Box::new(Expr::Var('a')),
                    Box::new(Expr::Lit(rng.u64(0..) & mask_of(w))),
                )],
            });
            if rng.bool() {
                // Range constraint — clamp batas atas agar range non-kosong.
                let msk = mask_of(w).max(8); // minimal range 0..8
                let lo = rng.u64(0..msk / 2);
                let hi = lo + rng.u64(1..=(msk / 4).max(1)) as u64;
                constraints.push(Constraint {
                    name: "c_range".to_string(),
                    body: vec![
                        Expr::Bin(BinOp::Ge, Box::new(Expr::Var('a')), Box::new(Expr::Lit(lo))),
                        Expr::Bin(BinOp::Le, Box::new(Expr::Var('a')), Box::new(Expr::Lit(hi))),
                    ],
                });
            }
            m.class = Some(SvClass {
                name: "test_obj".to_string(),
                rand_fields: fields,
                constraints,
            });
        }
        GenMode::Generate => {
            if rng.bool() {
                // Generate for
                let bound = rng.u32(1..=4);
                m.gen_blocks.push(GenKind::For {
                    var: "gi".to_string(),
                    bound,
                    body: vec![
                        Stmt::VarDecl {
                            name: "tmp".to_string(),
                            width: w,
                        },
                        Stmt::Assign {
                            lhs: "tmp".to_string(),
                            rhs: Expr::Bin(
                                BinOp::Add,
                                Box::new(Expr::Var('a')),
                                Box::new(Expr::Lit(bound as u64)),
                            ),
                            kind: AssignKind::Blocking,
                        },
                    ],
                });
            } else {
                // Generate if
                let cond = Expr::Bin(
                    BinOp::Gt,
                    Box::new(Expr::Var('a')),
                    Box::new(Expr::Lit(mask_of(w) / 2)),
                );
                m.gen_blocks.push(GenKind::If {
                    cond,
                    then_body: vec![Stmt::Assign {
                        lhs: "y".to_string(),
                        rhs: Expr::Var('a'),
                        kind: AssignKind::Blocking,
                    }],
                    else_body: Some(vec![Stmt::Assign {
                        lhs: "y".to_string(),
                        rhs: Expr::Var('b'),
                        kind: AssignKind::Blocking,
                    }]),
                });
            }
        }
    }

    SVAst::Module(m)
}

/// Generate SVAst dengan mode tertentu (untuk targeted testing).
pub fn generate_svast_mode(seed: u64, mode: GenMode) -> SVAst {
    let mut rng = Rng::with_seed(seed);
    let w = WIDTH_CHOICES[rng.usize(0..WIDTH_CHOICES.len())];

    let mut m = SvModule {
        params: Vec::new(),
        width: w,
        comb: None,
        seq: None,
        fork_join: None,
        class: None,
        gen_blocks: Vec::new(),
        extra_stmts: Vec::new(),
    };

    match mode {
        GenMode::Combinational => {
            m.comb = Some(CombAssign {
                expr: gen_expr(w, &mut rng, 0),
            });
        }
        GenMode::Sequential => {
            let sens = vec![SensEntry {
                signal: "clk".to_string(),
                edge: Some(SensEdge::Posedge),
            }];
            // Body: satu assign + opsional statement bertingkat (If/Case/For)
            let mut body = vec![Stmt::Assign {
                lhs: "y".to_string(),
                rhs: gen_expr(w, &mut rng, 2),
                kind: AssignKind::NonBlocking,
            }];
            if rng.bool() {
                // Selipkan statement kontrol untuk pipeline coverage
                // if_in_always / case_in_always / loop_in_always.
                match rng.usize(0..3) {
                    0 => {
                        body.push(Stmt::If {
                            cond: Expr::Bin(
                                BinOp::Gt,
                                Box::new(Expr::Var('a')),
                                Box::new(Expr::Lit(mask_of(w) / 2)),
                            ),
                            then_body: vec![Stmt::Assign {
                                lhs: "y".to_string(),
                                rhs: Expr::Var('a'),
                                kind: AssignKind::NonBlocking,
                            }],
                            else_body: Some(vec![Stmt::Assign {
                                lhs: "y".to_string(),
                                rhs: Expr::Var('b'),
                                kind: AssignKind::NonBlocking,
                            }]),
                        });
                    }
                    1 => {
                        body.push(Stmt::Case {
                            expr: gen_expr(w, &mut rng, 1),
                            items: vec![
                                (
                                    vec![1],
                                    vec![Stmt::Assign {
                                        lhs: "y".to_string(),
                                        rhs: Expr::Lit(1),
                                        kind: AssignKind::NonBlocking,
                                    }],
                                ),
                                (
                                    vec![2],
                                    vec![Stmt::Assign {
                                        lhs: "y".to_string(),
                                        rhs: Expr::Lit(2),
                                        kind: AssignKind::NonBlocking,
                                    }],
                                ),
                            ],
                            default: Some(vec![Stmt::Assign {
                                lhs: "y".to_string(),
                                rhs: Expr::Var('b'),
                                kind: AssignKind::NonBlocking,
                            }]),
                        });
                    }
                    _ => {
                        body.push(Stmt::For {
                            var: "li".to_string(),
                            from: Expr::Lit(0),
                            to: Expr::Lit(3),
                            body: vec![Stmt::Assign {
                                lhs: "y".to_string(),
                                rhs: Expr::Bin(
                                    BinOp::Add,
                                    Box::new(Expr::Var('a')),
                                    Box::new(Expr::Var('b')),
                                ),
                                kind: AssignKind::NonBlocking,
                            }],
                        });
                    }
                }
            }
            m.seq = Some(SeqBlock {
                sensitivity: sens,
                body,
            });
        }
        GenMode::ForkJoin => {
            let n_branches = rng.usize(2..=4);
            let mut branches = Vec::new();
            for _ in 0..n_branches {
                branches.push(vec![
                    Stmt::Assign {
                        lhs: "y".to_string(),
                        rhs: gen_expr(w, &mut rng, 1),
                        kind: AssignKind::Blocking,
                    },
                    Stmt::Delay(1),
                ]);
            }
            let fm = match rng.usize(0..3) {
                0 => ForkMode::Join,
                1 => ForkMode::JoinAny,
                _ => ForkMode::JoinNone,
            };
            m.fork_join = Some(ForkJoin { mode: fm, branches });
        }
        GenMode::Class => {
            let fields = vec![("x0".to_string(), w)];
            let constraints = vec![Constraint {
                name: "c1".to_string(),
                body: vec![Expr::Bin(
                    BinOp::Eq,
                    Box::new(Expr::Var('a')),
                    Box::new(Expr::Lit(rng.u64(0..) & mask_of(w))),
                )],
            }];
            m.class = Some(SvClass {
                name: "test_obj".to_string(),
                rand_fields: fields,
                constraints,
            });
        }
        GenMode::Generate => {
            m.gen_blocks.push(GenKind::For {
                var: "gi".to_string(),
                bound: rng.u32(1..=4),
                body: vec![Stmt::Assign {
                    lhs: "y".to_string(),
                    rhs: Expr::Bin(
                        BinOp::Add,
                        Box::new(Expr::Var('a')),
                        Box::new(Expr::Var('b')),
                    ),
                    kind: AssignKind::Blocking,
                }],
            });
        }
    }

    SVAst::Module(m)
}

// ── Pipeline features (coverage tagging) ────────────────────────────────────

/// Ekstrak tag fitur pipeline dari AST — dipakai CoverageGuide.
pub fn pipeline_features(ast: &SVAst) -> Vec<String> {
    let mut tags = Vec::new();
    match ast {
        SVAst::Module(m) => {
            if m.comb.is_some() {
                tags.push("pipeline:combinational".into());
            }
            if let Some(seq) = &m.seq {
                tags.push("pipeline:sequential".into());
                if seq.sensitivity.iter().any(|s| s.edge.is_some()) {
                    tags.push("pipeline:posedge".into());
                }
                if seq.body.iter().any(|s| {
                    matches!(
                        s,
                        Stmt::Assign {
                            kind: AssignKind::NonBlocking,
                            ..
                        }
                    )
                }) {
                    tags.push("pipeline:nonblocking".into());
                }
                if seq.body.iter().any(|s| matches!(s, Stmt::If { .. })) {
                    tags.push("pipeline:if_in_always".into());
                }
                if seq.body.iter().any(|s| matches!(s, Stmt::Case { .. })) {
                    tags.push("pipeline:case_in_always".into());
                }
                if seq.body.iter().any(|s| matches!(s, Stmt::For { .. })) {
                    tags.push("pipeline:loop_in_always".into());
                }
            }
            if let Some(fj) = &m.fork_join {
                tags.push("pipeline:concurrent".into());
                let mode_tag = match fj.mode {
                    ForkMode::Join => "pipeline:fork_join",
                    ForkMode::JoinAny => "pipeline:fork_join_any",
                    ForkMode::JoinNone => "pipeline:fork_join_none",
                };
                tags.push(mode_tag.into());
                tags.push(format!("pipeline:fork_branches:{}", fj.branches.len()));
            }
            if let Some(cls) = &m.class {
                tags.push("pipeline:class".into());
                tags.push(format!("pipeline:class_fields:{}", cls.rand_fields.len()));
                if !cls.constraints.is_empty() {
                    tags.push("pipeline:constraint".into());
                    tags.push(format!("pipeline:constraints:{}", cls.constraints.len()));
                }
            }
            for gb in &m.gen_blocks {
                match gb {
                    GenKind::For { .. } => {
                        tags.push("pipeline:generate_for".into());
                    }
                    GenKind::If { .. } => {
                        tags.push("pipeline:generate_if".into());
                    }
                }
            }
            // Expression features
            if let Some(comb) = &m.comb {
                let mut expr_tags = Vec::new();
                comb.expr.features(&mut expr_tags);
                tags.extend(expr_tags);
            }
            tags.push(format!("pipeline:w:{}", m.width));
        }
    }
    tags
}

// ── Evaluasi emas tingkat modul ─────────────────────────────────────────────

/// Evaluasi emas untuk SVAst. Mengembalikan Some(y) hanya untuk kasus
/// kombinasional murni (assign y = expr) tanpa X-state. Untuk fitur lain
/// atau ekspresi dengan X, None (verifikasi pakai determinism check).
pub fn eval_golden(ast: &SVAst, a: u64, b: u64) -> Option<u64> {
    match ast {
        SVAst::Module(m) => {
            if let Some(comb) = &m.comb {
                // Hanya kombinasional murni tanpa always block
                if m.seq.is_none() && m.fork_join.is_none() && m.class.is_none() {
                    // Skip X-state expressions
                    if comb.expr.eval_has_x(m.width, a, b) {
                        return None;
                    }
                    return Some(comb.expr.eval(m.width, a, b));
                }
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svast_to_source_compile() {
        for seed in 0..50u64 {
            let ast = generate_svast(seed);
            let src = ast.to_source();
            // Source harus mengandung module
            assert!(
                src.contains("module fuzz_mod"),
                "seed={}: missing module",
                seed
            );
            assert!(
                src.contains("endmodule"),
                "seed={}: missing endmodule",
                seed
            );
        }
    }

    #[test]
    fn svast_combinational_golden_matches_sim() {
        let mut mismatch = Vec::new();
        let mut checked = 0u32;
        for seed in 0..100u64 {
            let ast = generate_svast_mode(seed, GenMode::Combinational);
            let m = match &ast {
                SVAst::Module(m) => m,
            };
            let w = m.width;
            if m.comb.is_none() {
                continue;
            }
            let comb = m.comb.as_ref().unwrap();

            let mut rng = Rng::with_seed(seed ^ 0xABCD);
            let a = rng.u64(0..) & mask_of(w);
            let b = rng.u64(0..) & mask_of(w);

            // Skip X-state expressions (X-literal, div-by-zero, X dalam
            // operator logical yang di-treat-as-false oleh golden)
            if comb.expr.eval_has_x(w, a, b) || has_x_literal(&comb.expr) {
                continue;
            }

            let expected = comb.expr.eval(w, a, b);
            let src = ast
                .to_source()
                .replace(
                    &format!("a = {};", lit_sv(0, w)),
                    &format!("a = {};", lit_sv(a, w)),
                )
                .replace(
                    &format!("b = {};", lit_sv(0, w)),
                    &format!("b = {};", lit_sv(b, w)),
                );
            let src_for_sim = src.clone();
            let actual = std::thread::Builder::new()
                .name("svast-test-sim".to_string())
                .stack_size(256 * 1024 * 1024)
                .spawn(move || {
                    crate::simulate_signals(&src_for_sim, 20)
                        .ok()
                        .and_then(|s| s.iter().find(|(n, _)| n == "y").map(|(_, v)| v.to_u64()))
                })
                .expect("spawn")
                .join()
                .expect("sim panic");

            if actual != Some(expected) {
                mismatch.push(format!(
                    "seed={} w={} exp={:#x} act={:?}\n{}",
                    seed, w, expected, actual, src
                ));
            }
            checked += 1;
        }
        assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
        assert!(
            mismatch.is_empty(),
            "{} mismatch:\n{}",
            mismatch.len(),
            mismatch.join("\n---\n")
        );
    }

    #[test]
    fn svast_all_modes_produce_valid_source() {
        for &mode in GenMode::all() {
            for seed in 0..20u64 {
                let ast = generate_svast_mode(seed, mode);
                let src = ast.to_source();
                assert!(
                    src.contains("module fuzz_mod"),
                    "mode={:?} seed={}: no module",
                    mode,
                    seed
                );
                assert!(
                    src.contains("endmodule"),
                    "mode={:?} seed={}: no endmodule",
                    mode,
                    seed
                );
                assert!(
                    src.contains("reg ["),
                    "mode={:?} seed={}: no reg decl",
                    mode,
                    seed
                );
            }
        }
    }

    #[test]
    fn svast_pipeline_features_nonempty() {
        for seed in 0..30u64 {
            let ast = generate_svast(seed);
            let features = pipeline_features(&ast);
            assert!(!features.is_empty(), "seed={}: no features", seed);
        }
    }
}
