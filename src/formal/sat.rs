//! SAT/SMT Solver Bridge — SystemVerilog Expressions ⟷ Z3 Bit-Vector Formulas.
//!
//! Translates IrExpr trees into Z3 bit-vector expressions for formal analysis.
//! Uses z3 0.20 thread-local context — constructors take no `&Context` argument.

use super::FormalEngine;
use crate::ir::*;

impl FormalEngine {
    /// Convert an IrExpr to a Z3 Boolean formula.
    pub fn expr_to_z3_bool(&self, expr: &IrExpr) -> Option<z3::ast::Bool> {
        match expr {
            IrExpr::Const(lv) => {
                let val = lv.to_u64();
                Some(z3::ast::Bool::from_bool(val != 0))
            }
            IrExpr::BinaryOp(op, lhs, rhs) => {
                match op {
                    BinaryIrOp::Eq | BinaryIrOp::CaseEq | BinaryIrOp::EqWild => {
                        let l = self.expr_to_z3_int(lhs)?;
                        let r = self.expr_to_z3_int(rhs)?;
                        let (l, r) = self.zero_extend_match(&l, &r);
                        Some(l.eq(&r))
                    }
                    BinaryIrOp::Neq | BinaryIrOp::CaseNeq | BinaryIrOp::NeqWild => {
                        let l = self.expr_to_z3_int(lhs)?;
                        let r = self.expr_to_z3_int(rhs)?;
                        let (l, r) = self.zero_extend_match(&l, &r);
                        Some(l.eq(&r).not())
                    }
                    BinaryIrOp::Lt => {
                        let l = self.expr_to_z3_int(lhs)?;
                        let r = self.expr_to_z3_int(rhs)?;
                        let (l, r) = self.zero_extend_match(&l, &r);
                        Some(l.bvslt(&r))
                    }
                    BinaryIrOp::Le => {
                        let l = self.expr_to_z3_int(lhs)?;
                        let r = self.expr_to_z3_int(rhs)?;
                        let (l, r) = self.zero_extend_match(&l, &r);
                        Some(l.bvsle(&r))
                    }
                    BinaryIrOp::Gt => {
                        let l = self.expr_to_z3_int(lhs)?;
                        let r = self.expr_to_z3_int(rhs)?;
                        let (l, r) = self.zero_extend_match(&l, &r);
                        Some(l.bvsgt(&r))
                    }
                    BinaryIrOp::Ge => {
                        let l = self.expr_to_z3_int(lhs)?;
                        let r = self.expr_to_z3_int(rhs)?;
                        let (l, r) = self.zero_extend_match(&l, &r);
                        Some(l.bvsge(&r))
                    }
                    BinaryIrOp::LogicalAnd => {
                        let l = self.expr_to_z3_bool(lhs)?;
                        let r = self.expr_to_z3_bool(rhs)?;
                        Some(z3::ast::Bool::and(&[&l, &r]))
// Note: Bool::and(&[&l, &r]) works because &T: Into<T> for T: Clone
                    }
                    BinaryIrOp::LogicalOr => {
                        let l = self.expr_to_z3_bool(lhs)?;
                        let r = self.expr_to_z3_bool(rhs)?;
                        Some(z3::ast::Bool::or(&[&l, &r]))
                    }
                    _ => None,
                }
            }
            IrExpr::UnaryOp(UnaryIrOp::Not, inner) => {
                let v = self.expr_to_z3_bool(inner)?;
                Some(v.not())
            }
            IrExpr::Cond(cond, t, f) => {
                let c = self.expr_to_z3_bool(cond)?;
                let tv = self.expr_to_z3_bool(t)?;
                let fv = self.expr_to_z3_bool(f)?;
                Some(c.ite(&tv, &fv))
            }
            _ => {
                // Fallback: non-bool expressions treated as (val != 0)
                let val = self.expr_to_z3_int(expr)?;
                let zero = z3::ast::BV::from_u64(0, val.get_size());
                Some(val.eq(&zero).not())
            }
        }
    }

    /// Convert an IrExpr to a Z3 bit-vector expression.
    pub fn expr_to_z3_int(&self, expr: &IrExpr) -> Option<z3::ast::BV> {
        match expr {
            IrExpr::Const(lv) => {
                let val = lv.to_u64();
                let width = lv.width.max(1).min(64) as u32;
                Some(z3::ast::BV::from_u64(val, width))
            }
            IrExpr::FillLit(v) => {
                let bit = match v {
                    LogicVal::Zero => 0u64,
                    _ => 1u64,
                };
                Some(z3::ast::BV::from_u64(bit, 1))
            }
            IrExpr::Signal(id, _) => {
                let name = format!("sig_{}", id);
                Some(z3::ast::BV::new_const(name, 64))
            }
            IrExpr::BinaryOp(op, lhs, rhs) => {
                let l = self.expr_to_z3_int(lhs)?;
                let r = self.expr_to_z3_int(rhs)?;
                let (l, r) = self.zero_extend_match(&l, &r);
                match op {
                    BinaryIrOp::Add => Some(l.bvadd(&r)),
                    BinaryIrOp::Sub => Some(l.bvsub(&r)),
                    BinaryIrOp::Mul => Some(l.bvmul(&r)),
                    BinaryIrOp::BitAnd => Some(l.bvand(&r)),
                    BinaryIrOp::BitOr => Some(l.bvor(&r)),
                    BinaryIrOp::BitXor => Some(l.bvxor(&r)),
                    BinaryIrOp::Shl => Some(l.bvshl(&r)),
                    BinaryIrOp::Shr => Some(l.bvlshr(&r)),
                    BinaryIrOp::Sshr => Some(l.bvashr(&r)),
                    BinaryIrOp::Eq | BinaryIrOp::CaseEq | BinaryIrOp::EqWild => {
                        let eq = l.eq(&r);
                        let one = z3::ast::BV::from_u64(1, 1);
                        let zero = z3::ast::BV::from_u64(0, 1);
                        Some(eq.ite(&one, &zero))
                    }
                    BinaryIrOp::Neq | BinaryIrOp::CaseNeq | BinaryIrOp::NeqWild => {
                        let eq = l.eq(&r);
                        let one = z3::ast::BV::from_u64(1, 1);
                        let zero = z3::ast::BV::from_u64(0, 1);
                        Some(eq.ite(&zero, &one))
                    }
                    BinaryIrOp::Lt => {
                        let cmp = l.bvslt(&r);
                        let one = z3::ast::BV::from_u64(1, 1);
                        let zero = z3::ast::BV::from_u64(0, 1);
                        Some(cmp.ite(&one, &zero))
                    }
                    BinaryIrOp::Le => {
                        let cmp = l.bvsle(&r);
                        let one = z3::ast::BV::from_u64(1, 1);
                        let zero = z3::ast::BV::from_u64(0, 1);
                        Some(cmp.ite(&one, &zero))
                    }
                    BinaryIrOp::Gt => {
                        let cmp = l.bvsgt(&r);
                        let one = z3::ast::BV::from_u64(1, 1);
                        let zero = z3::ast::BV::from_u64(0, 1);
                        Some(cmp.ite(&one, &zero))
                    }
                    BinaryIrOp::Ge => {
                        let cmp = l.bvsge(&r);
                        let one = z3::ast::BV::from_u64(1, 1);
                        let zero = z3::ast::BV::from_u64(0, 1);
                        Some(cmp.ite(&one, &zero))
                    }
                    _ => None,
                }
            }
            IrExpr::UnaryOp(op, inner) => {
                let v = self.expr_to_z3_int(inner)?;
                match op {
                    UnaryIrOp::Minus => {
                        let zero = z3::ast::BV::from_u64(0, v.get_size());
                        Some(zero.bvsub(&v))
                    }
                    UnaryIrOp::Not => {
                        let zero = z3::ast::BV::from_u64(0, 1);
                        let one = z3::ast::BV::from_u64(1, 1);
                        let is_zero = v.eq(&zero);
                        Some(is_zero.ite(&one, &zero))
                    }
                    UnaryIrOp::BitNot => Some(v.bvnot()),
                    _ => None,
                }
            }
            IrExpr::Cond(cond, t, f) => {
                let c = self.expr_to_z3_bool(cond)?;
                let tv = self.expr_to_z3_int(t)?;
                let fv = self.expr_to_z3_int(f)?;
                Some(c.ite(&tv, &fv))
            }
            _ => None,
        }
    }

    /// Build a Z3 constraint for a signal assignment.
    pub fn assign_to_z3(&self, lhs: &IrLValue, rhs: &IrExpr) -> Option<z3::ast::Bool> {
        match lhs {
            IrLValue::Signal(id, _) => {
                let rhs_z3 = self.expr_to_z3_int(rhs)?;
                let lhs_var = z3::ast::BV::new_const(format!("sig_{}", id), rhs_z3.get_size());
                Some(lhs_var.eq(&rhs_z3))
            }
            _ => None,
        }
    }

    /// Convert an IrExpr assertion condition to a Z3 Boolean constraint.
    pub fn assertion_to_z3(&self, cond: &IrExpr) -> Option<z3::ast::Bool> {
        self.expr_to_z3_bool(cond)
    }
}
