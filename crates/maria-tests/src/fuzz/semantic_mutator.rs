//! Semantic Mutator — mutasi berdasarkan tipe & konteks semantik (bukan acak).
//!
//! Berbeda dari `gen.rs` yang memutasi struktur ekspresi sintaksis saja,
//! ini memahami tipe sinyal (lebar, signedness, packed/unpacked, dll.)
//   dan hanya menghasilkan mutasi yang valid secara semantik.

use super::expr::{BinOp, Expr, UnOp};
use super::gen::GenInput;
use fastrand::Rng;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    Wire,
    Reg,
    Logic,
    Parameter,
    LocalParam,
    Input,
    Output,
    Inout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WidthKind {
    Fixed(u32),
    Parameterized(String),
    Expression(Expr),
}

#[derive(Debug, Clone)]
pub struct SignalInfo {
    pub name: String,
    pub kind: SignalKind,
    pub width: WidthKind,
    pub is_signed: bool,
    pub is_packed: bool,
    pub dimensions: Vec<WidthKind>,
}

#[derive(Debug, Clone)]
pub struct ModuleContext {
    pub name: String,
    pub signals: Vec<SignalInfo>,
    pub parameters: Vec<(String, Expr)>,
    pub generate_blocks: Vec<GenerateBlock>,
    pub instances: Vec<InstanceInfo>,
}

#[derive(Debug, Clone)]
pub struct GenerateBlock {
    pub kind: GenerateKind,
    pub condition: Option<Expr>,
    pub genvar: Option<String>,
    pub init: Option<Expr>,
    pub body: Vec<Expr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerateKind {
    For,
    If,
    Case,
}

#[derive(Debug, Clone)]
pub struct InstanceInfo {
    pub module_name: String,
    pub instance_name: String,
    pub port_connections: Vec<(String, Expr)>,
    pub param_overrides: Vec<(String, Expr)>,
}

#[derive(Debug, Clone)]
pub struct RiskScore {
    pub node_type: String,
    pub score: f64,
    pub reasons: Vec<String>,
}

#[derive(Clone)]
pub struct SemanticMutator {
    rng: Rng,
    context: Option<ModuleContext>,
}

impl SemanticMutator {
    pub fn new(seed: u64) -> Self {
        SemanticMutator {
            rng: Rng::with_seed(seed),
            context: None,
        }
    }

    pub fn with_context(mut self, ctx: ModuleContext) -> Self {
        self.context = Some(ctx);
        self
    }

    pub fn mutate_input(&mut self, input: &GenInput) -> GenInput {
        let mut new_input = input.clone();
        new_input.seed = self.rng.u64(..);

        let mut mutation_count = 0;
        let max_mutations = 3;

        while mutation_count < max_mutations && self.rng.f32() < 0.7 {
            let choice = self.rng.usize(0..5);
            match choice {
                0 => self.mutate_width(&mut new_input),
                1 => self.mutate_signedness(&mut new_input),
                2 => self.mutate_signal_kind(&mut new_input),
                3 => self.mutate_expression(&mut new_input),
                4 => self.mutate_generate_bounds(&mut new_input),
                _ => {}
            }
            mutation_count += 1;
        }

        new_input
    }

    fn mutate_width(&mut self, input: &mut GenInput) {
        let w = input.w;
        let new_w = match w {
            1 => {
                let choices = [1, 2, 4];
                choices[self.rng.usize(0..choices.len())]
            }
            2 => {
                let choices = [1, 2, 4];
                choices[self.rng.usize(0..choices.len())]
            }
            4 => {
                let choices = [2, 4, 8, 16];
                choices[self.rng.usize(0..choices.len())]
            }
            8 => {
                let choices = [4, 8, 16, 32];
                choices[self.rng.usize(0..choices.len())]
            }
            16 => {
                let choices = [8, 16, 32];
                choices[self.rng.usize(0..choices.len())]
            }
            _ => w,
        };

        if new_w != w {
            input.w = new_w as u32;
            input.wb = input.wb.min(input.w);
            let mask = if new_w >= 64 {
                u64::MAX
            } else {
                (1u64 << new_w) - 1
            };
            input.a &= mask;
            input.b &= mask;
            input.expr = self.rescale_expr_width(&input.expr, w, new_w);
        }
        input.normalize();
    }

    fn rescale_expr_width(&self, expr: &Expr, old_w: u32, new_w: u32) -> Expr {
        match expr {
            Expr::Lit(v) => Expr::Lit(
                v & if new_w >= 64 {
                    u64::MAX
                } else {
                    (1u64 << new_w) - 1
                },
            ),
            Expr::XLit { v, m } => {
                let mask = if new_w >= 64 {
                    u64::MAX
                } else {
                    (1u64 << new_w) - 1
                };
                Expr::XLit {
                    v: v & mask,
                    m: m & mask,
                }
            }
            Expr::Var(c) => Expr::Var(*c),
            Expr::Un(op, inner) => {
                Expr::Un(*op, Box::new(self.rescale_expr_width(inner, old_w, new_w)))
            }
            Expr::Bin(op, lhs, rhs) => Expr::Bin(
                *op,
                Box::new(self.rescale_expr_width(lhs, old_w, new_w)),
                Box::new(self.rescale_expr_width(rhs, old_w, new_w)),
            ),
            Expr::Ternary(c, t, f) => Expr::Ternary(
                Box::new(self.rescale_expr_width(c, old_w, new_w)),
                Box::new(self.rescale_expr_width(t, old_w, new_w)),
                Box::new(self.rescale_expr_width(f, old_w, new_w)),
            ),
            Expr::Repl(count, e) => {
                let max_count = ((128 / new_w.max(1)) as u32).clamp(1, u32::MAX);
                Expr::Repl(
                    (*count).min(max_count),
                    Box::new(self.rescale_expr_width(e, old_w, new_w)),
                )
            }
            Expr::BitSel(c, idx) => Expr::BitSel(*c, (*idx).min(new_w.saturating_sub(1))),
            Expr::PartSel(c, hi, lo) => {
                let max_idx = new_w.saturating_sub(1);
                let hi = (*hi).min(max_idx);
                let lo = (*lo).min(hi);
                Expr::PartSel(*c, hi, lo)
            }
        }
    }

    fn mutate_signedness(&mut self, input: &mut GenInput) {
        input.expr = self.inject_signed_cast(&input.expr);
    }

    fn inject_signed_cast(&mut self, expr: &Expr) -> Expr {
        if self.rng.f32() < 0.3 {
            match expr {
                Expr::Bin(op, lhs, rhs)
                    if matches!(op, BinOp::Lt | BinOp::Gt | BinOp::Eq | BinOp::Ne) =>
                {
                    Expr::Bin(
                        *op,
                        Box::new(Expr::Un(UnOp::Neg, Box::new(*lhs.clone()))),
                        rhs.clone(),
                    )
                }
                _ => expr.clone(),
            }
        } else {
            expr.clone()
        }
    }

    fn mutate_signal_kind(&mut self, input: &mut GenInput) {
        input.expr = self.mutate_expr_structure(&input.expr);
    }

    fn mutate_expr_structure(&mut self, expr: &Expr) -> Expr {
        match expr {
            Expr::Bin(op, lhs, rhs) => {
                let new_op = if self.rng.f32() < 0.3 {
                    self.random_compatible_op(*op)
                } else {
                    *op
                };
                Expr::Bin(
                    new_op,
                    Box::new(self.mutate_expr_structure(lhs)),
                    Box::new(self.mutate_expr_structure(rhs)),
                )
            }
            Expr::Un(op, inner) => {
                let new_op = if self.rng.f32() < 0.2 {
                    self.random_unary_op()
                } else {
                    *op
                };
                Expr::Un(new_op, Box::new(self.mutate_expr_structure(inner)))
            }
            Expr::Lit(v) => {
                if self.rng.f32() < 0.3 {
                    Expr::Lit(self.rng.u64(..))
                } else {
                    expr.clone()
                }
            }
            Expr::Var(c) => {
                if self.rng.f32() < 0.3 {
                    Expr::Var(if *c == 'a' { 'b' } else { 'a' })
                } else {
                    expr.clone()
                }
            }
            Expr::Ternary(c, t, f) => Expr::Ternary(
                Box::new(self.mutate_expr_structure(c)),
                Box::new(self.mutate_expr_structure(t)),
                Box::new(self.mutate_expr_structure(f)),
            ),
            Expr::Repl(count, e) => Expr::Repl(*count, Box::new(self.mutate_expr_structure(e))),
            Expr::BitSel(..) | Expr::PartSel(..) => expr.clone(),
            Expr::XLit { v, m } => {
                if self.rng.f32() < 0.3 {
                    // Geser pola X ke bit lain.
                    Expr::XLit {
                        v: *v,
                        m: m.rotate_left(1),
                    }
                } else {
                    expr.clone()
                }
            }
        }
    }

    fn random_compatible_op(&mut self, current: BinOp) -> BinOp {
        let compatible = match current {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                vec![BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::Mod]
            }
            BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Xnor => {
                vec![BinOp::And, BinOp::Or, BinOp::Xor, BinOp::Xnor]
            }
            BinOp::Shl | BinOp::Shr | BinOp::Sshl | BinOp::Sshr => {
                vec![BinOp::Shl, BinOp::Shr, BinOp::Sshl, BinOp::Sshr]
            }
            BinOp::Eq | BinOp::Ne => vec![
                BinOp::Eq,
                BinOp::Ne,
                BinOp::Lt,
                BinOp::Gt,
                BinOp::Le,
                BinOp::Ge,
            ],
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                vec![
                    BinOp::Lt,
                    BinOp::Gt,
                    BinOp::Le,
                    BinOp::Ge,
                    BinOp::Eq,
                    BinOp::Ne,
                ]
            }
            BinOp::LogicAnd | BinOp::LogicOr => vec![BinOp::LogicAnd, BinOp::LogicOr],
            BinOp::CaseEq | BinOp::CaseNeq => {
                vec![BinOp::CaseEq, BinOp::CaseNeq, BinOp::Eq, BinOp::Ne]
            }
            BinOp::Power => vec![BinOp::Power, BinOp::Mul],
            BinOp::Concat => vec![BinOp::Concat, BinOp::Add],
            BinOp::Inside => vec![BinOp::Inside, BinOp::Eq, BinOp::Ne],
        };
        compatible[self.rng.usize(0..compatible.len())]
    }

    fn random_unary_op(&mut self) -> UnOp {
        [UnOp::Not, UnOp::LogicNot, UnOp::Neg][self.rng.usize(0..3)]
    }

    fn mutate_expression(&mut self, input: &mut GenInput) {
        if self.rng.f32() < 0.4 {
            input.expr = super::expr::gen_node(input.w, &mut self.rng, 0);
        }
    }

    fn mutate_generate_bounds(&mut self, input: &mut GenInput) {
        let boundaries = [1u32, 2, 4, 8, 16, 31, 32, 33, 64, 128, 255, 256, 512, 1024];
        let boundary = boundaries[self.rng.usize(0..boundaries.len())];
        input.w = boundary;
        input.wb = boundary;
        let mask = if boundary >= 64 {
            u64::MAX
        } else {
            (1u64 << boundary) - 1
        };
        input.a = self.rng.u64(..) & mask;
        input.b = self.rng.u64(..) & mask;
        input.expr = super::expr::gen_node(boundary, &mut self.rng, 0);
        input.normalize();
    }

    pub fn compute_risk(&self, expr: &Expr) -> RiskScore {
        let mut score = 0.0;
        let mut reasons = Vec::new();
        let node_type = self.expr_type_name(expr);

        match expr {
            Expr::Bin(op, lhs, rhs) => {
                match op {
                    BinOp::Shl | BinOp::Shr => {
                        score += 0.3;
                        reasons.push("shift operation".to_string());
                    }
                    BinOp::Lt | BinOp::Gt => {
                        score += 0.25;
                        reasons.push("comparison".to_string());
                    }
                    BinOp::Div | BinOp::Mod => {
                        score += 0.4;
                        reasons.push("division/modulo".to_string());
                    }
                    _ => {}
                }
                score += self.compute_risk(lhs).score * 0.3;
                score += self.compute_risk(rhs).score * 0.3;
            }
            Expr::Un(op, inner) => {
                if matches!(op, UnOp::Neg) {
                    score += 0.2;
                    reasons.push("unary minus".to_string());
                }
                score += self.compute_risk(inner).score * 0.5;
            }
            Expr::Lit(_) => {
                score += 0.05;
            }
            Expr::Var(_) => {
                score += 0.1;
            }
            // Literal X/Z: jalur X-propagation berisiko tinggi.
            Expr::XLit { .. } => {
                score += 0.5;
                reasons.push("4-state literal".to_string());
            }
            Expr::Ternary(c, t, f) => {
                score += self.compute_risk(c).score * 0.2
                    + self.compute_risk(t).score * 0.3
                    + self.compute_risk(f).score * 0.3;
            }
            Expr::Repl(_, e) => {
                score += self.compute_risk(e).score * 0.4;
            }
            // Select di luar range = sumber X — risiko sedang.
            Expr::BitSel(..) | Expr::PartSel(..) => {
                score += 0.2;
            }
        }

        RiskScore {
            node_type,
            score: score.min(1.0),
            reasons,
        }
    }

    fn expr_type_name(&self, expr: &Expr) -> String {
        match expr {
            Expr::Lit(_) => "Literal".to_string(),
            Expr::XLit { .. } => "XLiteral".to_string(),
            Expr::Var(_) => "Variable".to_string(),
            Expr::Un(op, _) => format!("Unary({:?})", op),
            Expr::Bin(op, _, _) => format!("Binary({:?})", op),
            Expr::Ternary(..) => "Ternary".to_string(),
            Expr::Repl(..) => "Replication".to_string(),
            Expr::BitSel(..) => "BitSelect".to_string(),
            Expr::PartSel(..) => "PartSelect".to_string(),
        }
    }
}

impl Default for SemanticMutator {
    fn default() -> Self {
        Self::new(0)
    }
}
