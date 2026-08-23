//! Testcase Minimizer — mengecilkan testcase hingga minimal reproducer.

use super::gen::GenInput;
use crate::compile_str;
use crate::simulate_signals;

#[derive(Debug, Clone)]
pub struct MinimizeResult {
    pub original: GenInput,
    pub minimized: GenInput,
    pub steps: Vec<MinimizeStep>,
    pub still_fails: bool,
}

#[derive(Debug, Clone)]
pub struct MinimizeStep {
    pub action: MinimizeAction,
    pub before: GenInput,
    pub after: GenInput,
    pub still_fails: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinimizeAction {
    ReduceWidth,
    SimplifyExpression,
    RemoveSignals,
    ReduceStimulus,
    FlattenHierarchy,
}

pub struct TestcaseMinimizer {
    pub max_iterations: usize,
    pub timeout_secs: u64,
}

impl Default for TestcaseMinimizer {
    fn default() -> Self {
        TestcaseMinimizer {
            max_iterations: 100,
            timeout_secs: 30,
        }
    }
}

impl TestcaseMinimizer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn minimize(&self, input: &GenInput, check_fn: impl Fn(&GenInput) -> bool) -> MinimizeResult {
        let mut current = input.clone();
        let mut steps = Vec::new();
        let mut iteration = 0;

        while iteration < self.max_iterations && check_fn(&current) {
            let mut improved = false;

            if current.w > 1 {
                if let Some(smaller) = self.try_reduce_width(&current, &check_fn) {
                    steps.push(MinimizeStep {
                        action: MinimizeAction::ReduceWidth,
                        before: current.clone(),
                        after: smaller.clone(),
                        still_fails: check_fn(&smaller),
                    });
                    current = smaller;
                    improved = true;
                    continue;
                }
            }

            if let Some(simpler) = self.try_simplify_expr(&current, &check_fn) {
                steps.push(MinimizeStep {
                    action: MinimizeAction::SimplifyExpression,
                    before: current.clone(),
                    after: simpler.clone(),
                    still_fails: check_fn(&simpler),
                });
                current = simpler;
                improved = true;
                continue;
            }

            if let Some(reduced) = self.try_reduce_stimulus(&current, &check_fn) {
                steps.push(MinimizeStep {
                    action: MinimizeAction::ReduceStimulus,
                    before: current.clone(),
                    after: reduced.clone(),
                    still_fails: check_fn(&reduced),
                });
                current = reduced;
                improved = true;
                continue;
            }

            if !improved {
                break;
            }

            iteration += 1;
        }

        MinimizeResult {
            original: input.clone(),
            minimized: current.clone(),
            steps,
            still_fails: check_fn(&current),
        }
    }

    fn try_reduce_width(&self, input: &GenInput, check_fn: &impl Fn(&GenInput) -> bool) -> Option<GenInput> {
        let current_w = input.w;
        let candidates = [1, 2, 4, 8, 16, 32, 64].into_iter()
            .filter(|&w| w < current_w)
            .collect::<Vec<_>>();

        for &new_w in &candidates {
            let mut candidate = input.clone();
            candidate.w = new_w;
            let mask = if new_w >= 64 { u64::MAX } else { (1u64 << new_w) - 1 };
            candidate.a &= mask;
            candidate.b &= mask;
            candidate.expr = self.rescale_expr(&input.expr, current_w, new_w);

            if check_fn(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    fn rescale_expr(&self, expr: &super::expr::Expr, old_w: u32, new_w: u32) -> super::expr::Expr {
        use super::expr::{Expr, BinOp, UnOp};
        match expr {
            Expr::Lit(v) => Expr::Lit(v & if new_w >= 64 { u64::MAX } else { (1u64 << new_w) - 1 }),
            Expr::Var(c) => Expr::Var(*c),
            Expr::Un(op, inner) => Expr::Un(*op, Box::new(self.rescale_expr(inner, old_w, new_w))),
            Expr::Bin(op, lhs, rhs) => Expr::Bin(
                *op,
                Box::new(self.rescale_expr(lhs, old_w, new_w)),
                Box::new(self.rescale_expr(rhs, old_w, new_w)),
            ),
        }
    }

    fn try_simplify_expr(&self, input: &GenInput, check_fn: &impl Fn(&GenInput) -> bool) -> Option<GenInput> {
        let simplified = self.simplify_expr(&input.expr);
        if simplified != input.expr {
            let mut candidate = input.clone();
            candidate.expr = simplified;
            if check_fn(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    fn simplify_expr(&self, expr: &super::expr::Expr) -> super::expr::Expr {
        use super::expr::{Expr, BinOp, UnOp};
        match expr {
            Expr::Bin(op, lhs, rhs) => {
                let simplified_lhs = self.simplify_expr(lhs);
                let simplified_rhs = self.simplify_expr(rhs);

                // Match on op first for better exhaustiveness checking
                match op {
                    BinOp::Add => match (&simplified_lhs, &simplified_rhs) {
                        (Expr::Lit(0), r) | (r, Expr::Lit(0)) => r.clone(),
                        _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                    },
                    BinOp::Sub => match (&simplified_lhs, &simplified_rhs) {
                        (l, Expr::Lit(0)) => l.clone(),
                        _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                    },
                    BinOp::Mul => match (&simplified_lhs, &simplified_rhs) {
                        (Expr::Lit(1), r) | (r, Expr::Lit(1)) => r.clone(),
                        (Expr::Lit(0), _) | (_, Expr::Lit(0)) => Expr::Lit(0),
                        _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                    },
                    BinOp::And => match (&simplified_lhs, &simplified_rhs) {
                        (Expr::Lit(v), r) | (r, Expr::Lit(v)) if *v == 0 => Expr::Lit(0),
                        _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                    },
                    BinOp::Or => match (&simplified_lhs, &simplified_rhs) {
                        (Expr::Lit(v), r) | (r, Expr::Lit(v)) if *v == u64::MAX => Expr::Lit(*v),
                        _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                    },
                    BinOp::Xor => match (&simplified_lhs, &simplified_rhs) {
                        (l, r) if l == r => Expr::Lit(0),
                        _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                    },
                    BinOp::Xnor => match (&simplified_lhs, &simplified_rhs) {
                        (Expr::Lit(v), r) | (r, Expr::Lit(v)) if *v == 0 => r.clone(),
                        (Expr::Lit(v), r) | (r, Expr::Lit(v)) if *v == u64::MAX => Expr::Lit(!*v),
                        _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                    },
                    BinOp::LogicAnd => match (&simplified_lhs, &simplified_rhs) {
                        (Expr::Lit(v), r) | (r, Expr::Lit(v)) if *v == 0 => Expr::Lit(0),
                        _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                    },
                    BinOp::LogicOr => match (&simplified_lhs, &simplified_rhs) {
                        (Expr::Lit(v), r) | (r, Expr::Lit(v)) if *v != 0 => Expr::Lit(1),
                        _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                    },
                    BinOp::Min => match (&simplified_lhs, &simplified_rhs) {
                        (Expr::Lit(v), r) | (r, Expr::Lit(v)) => r.clone(),
                        _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                    },
                    BinOp::Max => match (&simplified_lhs, &simplified_rhs) {
                        (Expr::Lit(v), r) | (r, Expr::Lit(v)) => r.clone(),
                        _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                    },
                    BinOp::Equiv => match (&simplified_lhs, &simplified_rhs) {
                        (l, r) if l == r => Expr::Lit(1),
                        _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                    },
                    BinOp::Implies => match (&simplified_lhs, &simplified_rhs) {
                        (Expr::Lit(0), r) | (r, Expr::Lit(0)) => Expr::Lit(1),
                        (Expr::Lit(v), r) | (r, Expr::Lit(v)) if *v != 0 => r.clone(),
                        _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                    },
                    BinOp::Concat => match (&simplified_lhs, &simplified_rhs) {
                        (Expr::Lit(_), Expr::Lit(_)) => expr.clone(),
                        _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                    },
                    _ => Expr::Bin(*op, Box::new(simplified_lhs), Box::new(simplified_rhs)),
                }
            }
            Expr::Un(op, inner) => {
                let simplified = self.simplify_expr(inner);
                match (op, &simplified) {
                    (UnOp::Not, Expr::Lit(v)) => Expr::Lit(!v),
                    (UnOp::LogicNot, Expr::Lit(0)) => Expr::Lit(1),
                    (UnOp::LogicNot, Expr::Lit(_)) => Expr::Lit(0),
                    (UnOp::Neg, Expr::Lit(0)) => Expr::Lit(0),
                    _ => Expr::Un(*op, Box::new(simplified)),
                }
            }
            Expr::Lit(_) | Expr::Var(_) => expr.clone(),
        }
    }

    fn try_reduce_stimulus(&self, input: &GenInput, check_fn: &impl Fn(&GenInput) -> bool) -> Option<GenInput> {
        let test_values = [0u64, 1, !0u64];

        for &a in &test_values {
            for &b in &test_values {
                let mut candidate = input.clone();
                let mask = if input.w >= 64 { u64::MAX } else { (1u64 << input.w) - 1 };
                candidate.a = a & mask;
                candidate.b = b & mask;
                if check_fn(&candidate) {
                    return Some(candidate);
                }
            }
        }
        None
    }

    pub fn minimize_source_file(&self, source: &str, check_fn: impl Fn(&str) -> bool) -> String {
        let mut current = source.to_string();
        let mut improved = true;

        while improved {
            improved = false;

            if let Some(shorter) = self.try_remove_unused_lines(&current, &check_fn) {
                current = shorter;
                improved = true;
                continue;
            }

            if let Some(shorter) = self.try_reduce_constants(&current, &check_fn) {
                current = shorter;
                improved = true;
                continue;
            }

            if let Some(shorter) = self.try_simplify_expressions(&current, &check_fn) {
                current = shorter;
                improved = true;
                continue;
            }
        }

        current
    }

    fn try_remove_unused_lines(&self, source: &str, check_fn: &impl Fn(&str) -> bool) -> Option<String> {
        let lines: Vec<&str> = source.lines().collect();
        for i in 0..lines.len() {
            let mut new_lines = lines.clone();
            new_lines.remove(i);
            let candidate = new_lines.join("\n");
            if check_fn(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    fn try_reduce_constants(&self, source: &str, check_fn: &impl Fn(&str) -> bool) -> Option<String> {
        let test_values = ["0", "1", "1'b0", "1'b1", "8'h00", "8'hFF", "16'h0000", "16'hFFFF"];
        for val in test_values {
            let candidate = source.replace("16'h", &format!("{}", val)).replace("8'h", &format!("{}", val));
            if candidate != source && check_fn(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    fn try_simplify_expressions(&self, source: &str, check_fn: &impl Fn(&str) -> bool) -> Option<String> {
        let simplifications = [
            (" + 0", ""),
            (" - 0", ""),
            (" * 1", ""),
            (" * 0", " 0"),
            (" | 0", ""),
            (" & 1", ""),
            (" ^ 0", ""),
            (" == 1", ""),
            (" != 0", ""),
        ];

        for (from, to) in simplifications {
            let candidate = source.replace(from, to);
            if candidate != source && check_fn(&candidate) {
                return Some(candidate);
            }
        }
        None
    }
}