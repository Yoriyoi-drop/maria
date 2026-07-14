use std::collections::HashMap;

use crate::ast::*;
use crate::ir::*;

pub fn map_ast_binary_op(op: &BinaryOp) -> Result<BinaryIrOp, String> {
    match op {
        BinaryOp::Add => Ok(BinaryIrOp::Add),
        BinaryOp::Sub => Ok(BinaryIrOp::Sub),
        BinaryOp::Mul => Ok(BinaryIrOp::Mul),
        BinaryOp::Div => Ok(BinaryIrOp::Div),
        BinaryOp::Mod => Ok(BinaryIrOp::Mod),
        BinaryOp::Power => Ok(BinaryIrOp::Power),
        BinaryOp::Eq => Ok(BinaryIrOp::Eq),
        BinaryOp::Neq => Ok(BinaryIrOp::Neq),
        BinaryOp::CaseEq => Ok(BinaryIrOp::CaseEq),
        BinaryOp::CaseNeq => Ok(BinaryIrOp::CaseNeq),
        BinaryOp::EqWild => Ok(BinaryIrOp::Eq),
        BinaryOp::NeqWild => Ok(BinaryIrOp::Neq),
        BinaryOp::Lt => Ok(BinaryIrOp::Lt),
        BinaryOp::Le => Ok(BinaryIrOp::Le),
        BinaryOp::Gt => Ok(BinaryIrOp::Gt),
        BinaryOp::Ge => Ok(BinaryIrOp::Ge),
        BinaryOp::BitAnd => Ok(BinaryIrOp::BitAnd),
        BinaryOp::BitOr => Ok(BinaryIrOp::BitOr),
        BinaryOp::BitXor => Ok(BinaryIrOp::BitXor),
        BinaryOp::BitXnor => Ok(BinaryIrOp::BitXnor),
        BinaryOp::Shl => Ok(BinaryIrOp::Shl),
        BinaryOp::Shr => Ok(BinaryIrOp::Shr),
        BinaryOp::Sshl => Ok(BinaryIrOp::Sshl),
        BinaryOp::Sshr => Ok(BinaryIrOp::Sshr),
        BinaryOp::LogicalAnd => Ok(BinaryIrOp::LogicalAnd),
        BinaryOp::LogicalOr => Ok(BinaryIrOp::LogicalOr),
    }
}

pub fn map_ast_unary_op(op: &UnaryOp) -> Result<UnaryIrOp, String> {
    match op {
        UnaryOp::Plus => Ok(UnaryIrOp::Plus),
        UnaryOp::Minus => Ok(UnaryIrOp::Minus),
        UnaryOp::BitNot => Ok(UnaryIrOp::BitNot),
        UnaryOp::Not => Ok(UnaryIrOp::Not),
        UnaryOp::ReductionAnd => Ok(UnaryIrOp::RedAnd),
        UnaryOp::ReductionNand => Ok(UnaryIrOp::RedNand),
        UnaryOp::ReductionOr => Ok(UnaryIrOp::RedOr),
        UnaryOp::ReductionNor => Ok(UnaryIrOp::RedNor),
        UnaryOp::ReductionXor => Ok(UnaryIrOp::RedXor),
        UnaryOp::ReductionXnor => Ok(UnaryIrOp::RedXnor),
    }
}

pub fn extract_signal_deps(expr: &IrExpr) -> Vec<SignalId> {
    let mut deps = Vec::new();
    extract_signal_deps_inner(expr, &mut deps);
    deps
}

pub fn extract_signal_deps_inner(expr: &IrExpr, deps: &mut Vec<SignalId>) {
    match expr {
        IrExpr::Signal(id, _) => {
            if !deps.contains(id) {
                deps.push(*id);
            }
        }
        IrExpr::RangeSelect(id, _, _) | IrExpr::BitSelect(id, _) | IrExpr::ArrayIndex { sig_id: id, .. } => {
            if !deps.contains(id) {
                deps.push(*id);
            }
        }
        IrExpr::ExprRangeSelect(e, _, _) | IrExpr::ExprBitSelect(e, _) => {
            extract_signal_deps_inner(e, deps);
        }
        IrExpr::ExprPartSelect(e1, e2, e3) => {
            extract_signal_deps_inner(e1, deps);
            extract_signal_deps_inner(e2, deps);
            extract_signal_deps_inner(e3, deps);
        }
        IrExpr::Concat(exprs) => {
            for e in exprs {
                extract_signal_deps_inner(e, deps);
            }
        }
        IrExpr::Replicate(_, e) => {
            extract_signal_deps_inner(e, deps);
        }
        IrExpr::UnaryOp(_, e) => {
            extract_signal_deps_inner(e, deps);
        }
        IrExpr::BinaryOp(_, e1, e2) => {
            extract_signal_deps_inner(e1, deps);
            extract_signal_deps_inner(e2, deps);
        }
        IrExpr::Cond(c, t, e) => {
            extract_signal_deps_inner(c, deps);
            extract_signal_deps_inner(t, deps);
            extract_signal_deps_inner(e, deps);
        }
        IrExpr::Signed(e) => {
            extract_signal_deps_inner(e, deps);
        }
        IrExpr::MethodCall { obj, args, .. } => {
            extract_signal_deps_inner(obj, deps);
            for a in args {
                extract_signal_deps_inner(a, deps);
            }
        }
        IrExpr::MemberAccess { obj, .. } => {
            extract_signal_deps_inner(obj, deps);
        }
        IrExpr::NewCall { args, .. } => {
            for a in args {
                extract_signal_deps_inner(a, deps);
            }
        }
        IrExpr::SysFunc { args, .. } => {
            for a in args {
                extract_signal_deps_inner(a, deps);
            }
        }
        IrExpr::DpiCall { args, .. } => {
            for a in args {
                extract_signal_deps_inner(a, deps);
            }
        }
        IrExpr::HierRef(_) => {}
        IrExpr::Inside { expr, list } => {
            extract_signal_deps_inner(expr, deps);
            for item in list {
                extract_signal_deps_inner(item, deps);
            }
        }
        IrExpr::Cast { expr, .. } => {
            extract_signal_deps_inner(expr, deps);
        }
        IrExpr::StreamingConcat { slices, .. } => {
            for e in slices {
                extract_signal_deps_inner(e, deps);
            }
        }
        IrExpr::Const(_) | IrExpr::FillLit(_) | IrExpr::String(_) | IrExpr::This => {}
    }
}

pub fn is_signed_expr(expr: &IrExpr, signals: &[SignalInfo]) -> bool {
    match expr {
        IrExpr::Signed(_) => true,
        IrExpr::Signal(id, _) | IrExpr::BitSelect(id, _) | IrExpr::RangeSelect(id, ..) => {
            signals.get(*id).map(|s| s.is_signed).unwrap_or(false)
        }
        IrExpr::ArrayIndex { sig_id, .. } => {
            signals.get(*sig_id).map(|s| s.is_signed).unwrap_or(false)
        }
        _ => false,
    }
}

