//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan util.rs (SRP Refactoring).
//! Tanggung jawab: Operator mapping & gate expression building.
//!
//! Fungsi:
//!   - map_unary_op()     — mapping UnaryOp → UnaryIrOp
//!   - map_binary_op()    — mapping BinaryOp → BinaryIrOp
//!   - build_gate_expr()  — buat IR expression untuk gate primitif
//!   - fold_binary()      — fold binary operation tree
//!
//! ──────────────────────────────────────────────────────────────────────────────

use maria_ir::*;
use maria_ast::*;

/// Mapping operator unary AST ke operator unary IR.
pub fn map_unary_op(op: &UnaryOp) -> Result<UnaryIrOp, String> {
    match op {
        UnaryOp::Plus => Ok(UnaryIrOp::Plus),
        UnaryOp::Minus => Ok(UnaryIrOp::Minus),
        UnaryOp::Not => Ok(UnaryIrOp::Not),
        UnaryOp::BitNot => Ok(UnaryIrOp::BitNot),
        UnaryOp::ReductionAnd => Ok(UnaryIrOp::RedAnd),
        UnaryOp::ReductionNand => Ok(UnaryIrOp::RedNand),
        UnaryOp::ReductionOr => Ok(UnaryIrOp::RedOr),
        UnaryOp::ReductionNor => Ok(UnaryIrOp::RedNor),
        UnaryOp::ReductionXor => Ok(UnaryIrOp::RedXor),
        UnaryOp::ReductionXnor => Ok(UnaryIrOp::RedXnor),
    }
}

/// Mapping operator binary AST ke operator binary IR.
pub fn map_binary_op(op: &BinaryOp) -> Result<BinaryIrOp, String> {
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
        BinaryOp::EqWild => Ok(BinaryIrOp::EqWild),
        BinaryOp::NeqWild => Ok(BinaryIrOp::NeqWild),
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

/// Bangun IR expression untuk gate primitif (and, or, nand, nor, xor, xnor, buf, not).
pub fn build_gate_expr(gate_type: &GateType, inputs: &[IrExpr]) -> IrExpr {
    match gate_type {
        GateType::And => fold_binary(BinaryIrOp::BitAnd, inputs),
        GateType::Or => fold_binary(BinaryIrOp::BitOr, inputs),
        GateType::Nand => IrExpr::UnaryOp(
            UnaryIrOp::BitNot,
            Box::new(fold_binary(BinaryIrOp::BitAnd, inputs)),
        ),
        GateType::Nor => IrExpr::UnaryOp(
            UnaryIrOp::BitNot,
            Box::new(fold_binary(BinaryIrOp::BitOr, inputs)),
        ),
        GateType::Xor => fold_binary(BinaryIrOp::BitXor, inputs),
        GateType::Xnor => IrExpr::UnaryOp(
            UnaryIrOp::BitNot,
            Box::new(fold_binary(BinaryIrOp::BitXor, inputs)),
        ),
        GateType::Buf => inputs[0].clone(),
        GateType::Not => IrExpr::UnaryOp(UnaryIrOp::BitNot, Box::new(inputs[0].clone())),
    }
}

/// Fold binary operation tree — gabungkan beberapa expression dengan operator yang sama.
pub fn fold_binary(op: BinaryIrOp, exprs: &[IrExpr]) -> IrExpr {
    if exprs.is_empty() {
        return IrExpr::Const(LogicVec::from_u64(0, 1));
    }
    let mut result = exprs[0].clone();
    for e in &exprs[1..] {
        result = IrExpr::BinaryOp(op.clone(), Box::new(result), Box::new(e.clone()));
    }
    result
}
