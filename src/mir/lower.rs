//! HIR → MIR Lowering.
//!
//! Converts high-level HIR into flat MIR instructions suitable for
//! fast event-driven simulation.

use std::collections::HashMap;

use super::mir::*;
use crate::hir::hir::{HirModule, HirStmt, HirExpr, HirBinOp, HirUnOp};
use crate::intern::Symbol;

/// Lower HIR module to MIR module.
pub fn lower_module(hir: &HirModule) -> MirModule {
    let mut mir = MirModule::new(hir.name);

    // Lower signals
    for sig in &hir.signals {
        let kind = if sig.is_input {
            MirSignalKind::Input
        } else if sig.is_output {
            MirSignalKind::Output
        } else {
            MirSignalKind::Reg
        };

        mir.add_signal(MirSignal {
            name: sig.name,
            width: sig.width,
            kind,
            initial_value: None,
        });
    }

    // Lower statements to processes
    let mut instrs = Vec::new();
    for stmt in &hir.stmts {
        lower_stmt(stmt, &mut instrs, &mut mir);
    }

    if !instrs.is_empty() {
        mir.processes.push(MirProcess {
            name: Symbol::intern("__init"),
            sensitivity: MirSensitivity::Initial,
            instrs,
        });
    }

    mir
}

/// Lower a single HIR statement to MIR instructions.
fn lower_stmt(stmt: &HirStmt, instrs: &mut Vec<MirInstr>, mir: &mut MirModule) {
    match stmt {
        HirStmt::Block { stmts, .. } => {
            for s in stmts {
                lower_stmt(s, instrs, mir);
            }
        }
        HirStmt::BlockingAssign { lhs, rhs } => {
            let dest_reg = alloc_temp(mir);
            lower_expr(rhs, instrs, mir, dest_reg);
            // For blocking assign, directly store
            if let HirExpr::Ident(name) = lhs.as_ref() {
                if let Some(sig_idx) = mir.signal_index(*name) {
                    instrs.push(MirInstr::Store { signal: sig_idx, src: dest_reg });
                }
            }
        }
        HirStmt::NonBlockingAssign { lhs, rhs } => {
            let dest_reg = alloc_temp(mir);
            lower_expr(rhs, instrs, mir, dest_reg);
            if let HirExpr::Ident(name) = lhs.as_ref() {
                if let Some(sig_idx) = mir.signal_index(*name) {
                    instrs.push(MirInstr::NonBlocking {
                        signal: sig_idx,
                        src: dest_reg,
                        delay: None,
                    });
                }
            }
        }
        HirStmt::If { cond, then, else_ } => {
            let cond_reg = alloc_temp(mir);
            lower_expr(cond, instrs, mir, cond_reg);
            let then_label = mir.processes.len();
            instrs.push(MirInstr::Label(then_label));
            lower_stmt(then, instrs, mir);
            if let Some(else_stmt) = else_ {
                let else_label = mir.processes.len();
                instrs.push(MirInstr::Label(else_label));
                lower_stmt(else_stmt, instrs, mir);
            }
        }
        HirStmt::Display { args } => {
            let mir_args: Vec<MirDisplayArg> = args.iter().map(|a| {
                match a {
                    HirExpr::StringLiteral(s) => MirDisplayArg::Str(s.as_str().to_string()),
                    HirExpr::Ident(name) => {
                        if let Some(idx) = mir.signal_index(*name) {
                            MirDisplayArg::Signal(idx)
                        } else {
                            MirDisplayArg::Str(format!("{}", name))
                        }
                    }
                    _ => MirDisplayArg::Str(format!("<expr>")),
                }
            }).collect();
            instrs.push(MirInstr::Display { args: mir_args });
        }
        HirStmt::Finish => {
            instrs.push(MirInstr::Finish);
        }
        _ => {
            instrs.push(MirInstr::Nop);
        }
    }
}

/// Lower a HIR expression to MIR instructions.
fn lower_expr(expr: &HirExpr, instrs: &mut Vec<MirInstr>, mir: &mut MirModule, dest: usize) {
    match expr {
        HirExpr::IntLiteral(val, width) => {
            instrs.push(MirInstr::Const { dest, value: *val, width: *width });
        }
        HirExpr::Ident(name) => {
            if let Some(sig_idx) = mir.signal_index(*name) {
                instrs.push(MirInstr::Load { dest, signal: sig_idx });
            }
        }
        HirExpr::Binary { op, lhs, rhs, width } => {
            let lhs_reg = alloc_temp(mir);
            let rhs_reg = alloc_temp(mir);
            lower_expr(lhs, instrs, mir, lhs_reg);
            lower_expr(rhs, instrs, mir, rhs_reg);
            let mir_op = match op {
                HirBinOp::Add => MirBinOp::Add,
                HirBinOp::Sub => MirBinOp::Sub,
                HirBinOp::Mul => MirBinOp::Mul,
                HirBinOp::BitAnd => MirBinOp::And,
                HirBinOp::BitOr => MirBinOp::Or,
                HirBinOp::Eq => MirBinOp::Eq,
                HirBinOp::Ne => MirBinOp::Ne,
                HirBinOp::Lt => MirBinOp::Lt,
                _ => MirBinOp::Add,
            };
            instrs.push(MirInstr::Binary { op: mir_op, dest, lhs: lhs_reg, rhs: rhs_reg, width: *width });
        }
        HirExpr::Unary { op, operand, width } => {
            let op_reg = alloc_temp(mir);
            lower_expr(operand, instrs, mir, op_reg);
            let mir_op = match op {
                HirUnOp::LogicNot => MirUnOp::Not,
                HirUnOp::Neg => MirUnOp::Neg,
                HirUnOp::BitNot => MirUnOp::Not,
            };
            instrs.push(MirInstr::Unary { op: mir_op, dest, operand: op_reg, width: *width });
        }
        HirExpr::FillLit { val, width } => {
            let fill_val = match val {
                0 => 0u64,
                1 => u64::MAX >> (64 - width.min(64)),
                _ => 0,
            };
            instrs.push(MirInstr::Const { dest, value: fill_val, width: *width });
        }
        _ => {
            instrs.push(MirInstr::Const { dest, value: 0, width: 1 });
        }
    }
}

/// Allocate a temporary register index.
fn alloc_temp(mir: &mut MirModule) -> usize {
    let name = Symbol::intern(&format!("__tmp_{}", mir.signals.len()));
    mir.add_signal(MirSignal {
        name,
        width: 64,
        kind: MirSignalKind::Logic,
        initial_value: None,
    })
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::hir::*;

    #[test]
    fn test_lower_module_empty() {
        let hir = HirModule {
            name: Symbol::intern("empty"),
            signals: vec![],
            inputs: vec![],
            outputs: vec![],
            params: vec![],
            stmts: vec![],
            sub_instances: vec![],
            checksum: 0,
        };

        let mir = lower_module(&hir);
        assert_eq!(mir.name, Symbol::intern("empty"));
        assert!(mir.signals.is_empty());
    }

    #[test]
    fn test_lower_module_with_signals() {
        let hir = HirModule {
            name: Symbol::intern("with_sigs"),
            signals: vec![
                HirSignal { name: Symbol::intern("a"), dtype: HirType::BitVec { width: 8 }, width: 8, is_input: true, is_output: false },
                HirSignal { name: Symbol::intern("b"), dtype: HirType::BitVec { width: 8 }, width: 8, is_input: false, is_output: true },
            ],
            inputs: vec![0],
            outputs: vec![1],
            params: vec![],
            stmts: vec![],
            sub_instances: vec![],
            checksum: 0,
        };

        let mir = lower_module(&hir);
        assert_eq!(mir.signals.len(), 2);
        assert_eq!(mir.signals[0].kind, MirSignalKind::Input);
        assert_eq!(mir.signals[1].kind, MirSignalKind::Output);
    }
}
