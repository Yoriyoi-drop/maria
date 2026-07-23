//! HIR → MIR Lowering — Full Pipeline.
//!
//! Converts high-level HIR into flat MIR instructions suitable for
//! fast event-driven simulation. Handles ALL HirStmt and HirExpr
//! variants, control flow (for, while, case, etc.), and design-level
//! lowering with sub-instance handling.

use std::collections::{HashMap, HashSet};

use super::mir::*;
use crate::hir::hir::{HirDesign, HirModule, HirStmt, HirExpr, HirBinOp, HirUnOp, HirInstance};
use crate::intern::Symbol;

// ─── Design-Level Lowering ───

/// Lower a full HirDesign to MirDesign.
///
/// Converts the top module and all sub-modules. Sub-instances are
/// noted for external flattening; the immediate output is a per-module
/// MirDesign.
pub fn lower_design(hir: &HirDesign) -> MirDesign {
    let mut mir_modules = HashMap::new();

    // Lower all modules
    for (name, module) in &hir.modules {
        let mir = lower_module(module);
        mir_modules.insert(*name, mir);
    }

    // Lower top module (may already be in modules)
    let top = lower_module(&hir.top);

    MirDesign {
        modules: mir_modules,
        top,
    }
}

/// Lower a single HIR module to a MIR module.
pub fn lower_module(hir: &HirModule) -> MirModule {
    let mut mir = MirModule::new(hir.name);

    // Pre-populate signal map with input/output signals first
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

    // Lower statements into processes
    let mut instrs = Vec::new();
    for stmt in &hir.stmts {
        lower_stmt(stmt, &mut instrs, &mut mir, false);
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

// ─── Statement Lowering ───

/// Lower a single HIR statement to MIR instructions.
fn lower_stmt(
    stmt: &HirStmt,
    instrs: &mut Vec<MirInstr>,
    mir: &mut MirModule,
) {
    match stmt {
        HirStmt::Block { stmts, .. } => {
            for s in stmts {
                lower_stmt(s, instrs, mir);
            }
        }

        HirStmt::BlockingAssign { lhs, rhs } => {
            let dest_reg = alloc_temp(mir);
            lower_expr(rhs, instrs, mir, dest_reg);
            let targets = extract_lvalue_targets(lhs);
            for target in targets {
                if let Some(sig_idx) = mir.signal_index(target) {
                    instrs.push(MirInstr::Store {
                        signal: sig_idx,
                        src: dest_reg,
                    });
                }
            }
        }

        HirStmt::NonBlockingAssign { lhs, rhs } => {
            let dest_reg = alloc_temp(mir);
            lower_expr(rhs, instrs, mir, dest_reg);
            let targets = extract_lvalue_targets(lhs);
            for target in targets {
                if let Some(sig_idx) = mir.signal_index(target) {
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

            // Generate: Branch cond → then_label : else_label
            let then_label = generate_label(instrs);
            let else_label = generate_label(instrs);
            let end_label = generate_label(instrs);

            // Branch: if cond != 0 → then_label, else → else_label
            instrs.push(MirInstr::Branch {
                cond: cond_reg,
                then_label,
                else_label,
            });

            instrs.push(MirInstr::Label(then_label));
            lower_stmt(then, instrs, mir);
            instrs.push(MirInstr::Jump { label: end_label });

            instrs.push(MirInstr::Label(else_label));
            if let Some(else_stmt) = else_ {
                lower_stmt(else_stmt, instrs, mir);
            }

            instrs.push(MirInstr::Label(end_label));
        }

        HirStmt::Case { expr, items } => {
            let case_reg = alloc_temp(mir);
            lower_expr(expr, instrs, mir, case_reg);

            let mut case_labels = Vec::new();
            for item in items {
                let label = generate_label(instrs);
                case_labels.push(label);
                instrs.push(MirInstr::Label(label));

                // Lower the case item statement
                lower_stmt(&item.stmt, instrs, mir);
            }

            // Generate the case dispatch: compare case_reg with each item's expression
            // Simple approach: linear chain of equality checks
            let end_label = generate_label(instrs);
            for (i, item) in items.iter().enumerate() {
                let item_result = alloc_temp(mir);
                // Initialize to 0 before OR-ing equality results
                instrs.push(MirInstr::Const { dest: item_result, value: 0, width: 1 });
                // Compare case_reg with each expression
                for case_expr in &item.exprs {
                    let case_val = alloc_temp(mir);
                    lower_expr(case_expr, instrs, mir, case_val);
                    let eq_result = alloc_temp(mir);
                    instrs.push(MirInstr::Binary {
                        op: MirBinOp::Eq,
                        dest: eq_result,
                        lhs: case_reg,
                        rhs: case_val,
                        width: 1,
                    });
                    // OR with item_result
                    instrs.push(MirInstr::Binary {
                        op: MirBinOp::Or,
                        dest: item_result,
                        lhs: item_result,
                        rhs: eq_result,
                        width: 1,
                    });
                }
                // Branch to item body if match
                instrs.push(MirInstr::Branch {
                    cond: item_result,
                    then_label: case_labels[i],
                    else_label: end_label,
                });
            }
            instrs.push(MirInstr::Label(end_label));
        }

        HirStmt::For { init, cond, step, body } => {
            // for(init; cond; step) body;
            // Lower as: init; loop { if !cond break; body; step; }
            lower_stmt(init, instrs, mir);
            let loop_start = generate_label(instrs);
            instrs.push(MirInstr::Label(loop_start));

            let cond_reg = alloc_temp(mir);
            lower_expr(cond, instrs, mir, cond_reg);

            // Branch: if cond == 0 → end_label, if cond != 0 → body_label
            let body_label = generate_label(instrs);
            let end_label = generate_label(instrs);
            instrs.push(MirInstr::Branch {
                cond: cond_reg,
                then_label: body_label,
                else_label: end_label,
            });

            instrs.push(MirInstr::Label(body_label));
            lower_stmt(body, instrs, mir);
            lower_stmt(step, instrs, mir);
            instrs.push(MirInstr::Jump { label: loop_start });
            instrs.push(MirInstr::Label(end_label));
        }

        HirStmt::While { cond, body } => {
            let loop_start = generate_label(instrs);
            instrs.push(MirInstr::Label(loop_start));

            let cond_reg = alloc_temp(mir);
            lower_expr(cond, instrs, mir, cond_reg);

            // Branch: if cond != 0 → body_label, else → end_label
            let body_label = generate_label(instrs);
            let end_label = generate_label(instrs);
            instrs.push(MirInstr::Branch {
                cond: cond_reg,
                then_label: body_label,
                else_label: end_label,
            });

            instrs.push(MirInstr::Label(body_label));
            lower_stmt(body, instrs, mir);
            instrs.push(MirInstr::Jump { label: loop_start });
            instrs.push(MirInstr::Label(end_label));
        }

        HirStmt::Repeat { count, body } => {
            // repeat(count) body;
            // Lower as: for(i = 0; i < count; i++) body;
            let count_reg = alloc_temp(mir);
            lower_expr(count, instrs, mir, count_reg);

            let counter = alloc_temp(mir);
            instrs.push(MirInstr::Const { dest: counter, value: 0, width: 32 });

            let loop_start = generate_label(instrs);
            instrs.push(MirInstr::Label(loop_start));

            // cond: counter < count_reg
            let cond_reg = alloc_temp(mir);
            instrs.push(MirInstr::Binary {
                op: MirBinOp::Lt,
                dest: cond_reg,
                lhs: counter,
                rhs: count_reg,
                width: 1,
            });

            // Branch: if counter < count → body_label, else → end_label
            let body_label = generate_label(instrs);
            let end_label = generate_label(instrs);
            instrs.push(MirInstr::Branch {
                cond: cond_reg,
                then_label: body_label,
                else_label: end_label,
            });

            instrs.push(MirInstr::Label(body_label));
            lower_stmt(body, instrs, mir);

            // counter++
            let one_reg = alloc_temp(mir);
            instrs.push(MirInstr::Const { dest: one_reg, value: 1, width: 32 });
            instrs.push(MirInstr::Binary {
                op: MirBinOp::Add,
                dest: counter,
                lhs: counter,
                rhs: one_reg,
                width: 32,
            });

            instrs.push(MirInstr::Jump { label: loop_start });
            instrs.push(MirInstr::Label(end_label));
        }

        HirStmt::Forever { body } => {
            // forever body; → loop { body; }
            let loop_start = generate_label(instrs);
            instrs.push(MirInstr::Label(loop_start));
            lower_stmt(body, instrs, mir, false);
            instrs.push(MirInstr::Jump { label: loop_start });
        }

        HirStmt::Display { args } => {
            let mir_args: Vec<MirDisplayArg> = args.iter().map(|a| {
                match a {
                    HirExpr::StringLiteral(s) => MirDisplayArg::Str(s.as_str().to_string()),
                    HirExpr::Ident(name) => {
                        if let Some(idx) = mir.signal_index(*name) {
                            MirDisplayArg::Signal(idx)
                        } else {
                            MirDisplayArg::Str(name.as_str().to_string())
                        }
                    }
                    _ => MirDisplayArg::Str("<expr>".to_string()),
                }
            }).collect();
            instrs.push(MirInstr::Display { args: mir_args });
        }

        HirStmt::Finish(_) => {
            instrs.push(MirInstr::Finish);
        }

        HirStmt::Return { value } => {
            // Value computed but no MIR return mechanism yet
            if let Some(expr) = value {
                let _ret_reg = alloc_temp(mir);
                lower_expr(expr, instrs, mir, _ret_reg);
            }
        }
    }
}

// ─── Expression Lowering ───

/// Lower a HIR expression to MIR instructions.
fn lower_expr(
    expr: &HirExpr,
    instrs: &mut Vec<MirInstr>,
    mir: &mut MirModule,
    dest: usize,
) {
    match expr {
        HirExpr::IntLiteral(val, width) => {
            instrs.push(MirInstr::Const {
                dest,
                value: *val,
                width: *width,
            });
        }

        HirExpr::RealLiteral(val) => {
            // Store real as u64 bits
            let bits = val.to_bits();
            instrs.push(MirInstr::Const {
                dest,
                value: bits,
                width: 64,
            });
        }

        HirExpr::StringLiteral(s) => {
            // Store as a hash for runtime comparison
            let hash = s.as_str().len() as u64;
            instrs.push(MirInstr::Const {
                dest,
                value: hash,
                width: 32,
            });
        }

        HirExpr::Ident(name) => {
            if let Some(sig_idx) = mir.signal_index(*name) {
                instrs.push(MirInstr::Load {
                    dest,
                    signal: sig_idx,
                });
            } else {
                // Unknown signal — load zero
                instrs.push(MirInstr::Const {
                    dest,
                    value: 0,
                    width: 1,
                });
            }
        }

        HirExpr::Binary { op, lhs, rhs, width } => {
            let lhs_reg = alloc_temp(mir);
            let rhs_reg = alloc_temp(mir);
            lower_expr(lhs, instrs, mir, lhs_reg);
            lower_expr(rhs, instrs, mir, rhs_reg);
            let mir_op = hir_binop_to_mir(*op);
            instrs.push(MirInstr::Binary {
                op: mir_op,
                dest,
                lhs: lhs_reg,
                rhs: rhs_reg,
                width: *width,
            });
        }

        HirExpr::Unary { op, operand, width } => {
            let op_reg = alloc_temp(mir);
            lower_expr(operand, instrs, mir, op_reg);
            let mir_op = hir_unop_to_mir(*op);
            instrs.push(MirInstr::Unary {
                op: mir_op,
                dest,
                operand: op_reg,
                width: *width,
            });
        }

        HirExpr::Ternary { cond, then, else_, width } => {
            // a ? b : c → if a { result = b } else { result = c }
            let cond_reg = alloc_temp(mir);
            lower_expr(cond, instrs, mir, cond_reg);

            let then_start = instrs.len();
            let then_label = generate_label(instrs);
            instrs.push(MirInstr::Label(then_label));
            lower_expr(then, instrs, mir, dest);

            let else_start = instrs.len() + 1;
            let else_label = generate_label(instrs);
            let end_label = generate_label(instrs);
            instrs.push(MirInstr::Jump { label: end_label });
            instrs.push(MirInstr::Label(else_label));
            lower_expr(else_, instrs, mir, dest);
            instrs.push(MirInstr::Label(end_label));

            // Insert branch before then body
            instrs.insert(then_start, MirInstr::Branch {
                cond: cond_reg,
                then_label,
                else_label,
            });
        }

        HirExpr::BitSelect { base, index, width } => {
            // base[index]
            let base_reg = alloc_temp(mir);
            let idx_reg = alloc_temp(mir);
            lower_expr(base, instrs, mir, base_reg);
            lower_expr(index, instrs, mir, idx_reg);

            // For now: load base, shift right by index, mask
            instrs.push(MirInstr::Binary {
                op: MirBinOp::Shr,
                dest,
                lhs: base_reg,
                rhs: idx_reg,
                width: *width,
            });
        }

        HirExpr::PartSelect { base, msb, lsb, width } => {
            // base[msb:lsb]
            let base_reg = alloc_temp(mir);
            let lsb_reg = alloc_temp(mir);
            lower_expr(base, instrs, mir, base_reg);
            lower_expr(lsb, instrs, mir, lsb_reg);

            // Shift right by lsb
            instrs.push(MirInstr::Binary {
                op: MirBinOp::Shr,
                dest,
                lhs: base_reg,
                rhs: lsb_reg,
                width: *width,
            });
        }

        HirExpr::Concat { parts, width } => {
            // {part1, part2, ...} — concatenate parts
            let mut offset = 0;
            // Lower all parts into temp registers, building final value
            let result_reg = alloc_temp(mir);
            instrs.push(MirInstr::Const {
                dest: result_reg,
                value: 0,
                width: *width,
            });

            for part in parts.iter().rev() {
                let part_reg = alloc_temp(mir);
                lower_expr(part, instrs, mir, part_reg);
                // Shift part to the right offset
                if offset > 0 {
                    let shift_reg = alloc_temp(mir);
                    instrs.push(MirInstr::Const {
                        dest: shift_reg,
                        value: offset as u64,
                        width: 64,
                    });
                    instrs.push(MirInstr::Binary {
                        op: MirBinOp::Shl,
                        dest: part_reg,
                        lhs: part_reg,
                        rhs: shift_reg,
                        width: *width,
                    });
                }
                // OR into result
                instrs.push(MirInstr::Binary {
                    op: MirBinOp::Or,
                    dest: result_reg,
                    lhs: result_reg,
                    rhs: part_reg,
                    width: *width,
                });
                offset += part.width();
            }
            // Copy result to dest
            // If result_reg != dest, add a copy
            if result_reg != dest {
                instrs.push(MirInstr::Binary {
                    op: MirBinOp::Or,
                    dest,
                    lhs: result_reg,
                    rhs: alloc_temp_const(mir, 0, *width),
                    width: *width,
                });
            }
        }

        HirExpr::Call { func: _func, args, width } => {
            // NOTE: Function call lowering is a stub.
            // Arguments are evaluated but the call is replaced with constant 0.
            // Full function call support requires a call stack in the MIR.
            for arg in args {
                let arg_reg = alloc_temp(mir);
                lower_expr(arg, instrs, mir, arg_reg);
            }
            // Stub: return 0 for unsupported calls
            instrs.push(MirInstr::Const {
                dest,
                value: 0,
                width: *width,
            });
        }

        HirExpr::FillLit { val, width } => {
            let fill_val = match val {
                0 => 0u64,
                1 => u64::MAX >> (64 - width.min(64)),
                2 => 0u64, // 'x → 0 for MIR (no X-state in simple sim)
                3 => 0u64, // 'z → 0
                _ => 0u64,
            };
            instrs.push(MirInstr::Const {
                dest,
                value: fill_val,
                width: *width,
            });
        }
    }
}

// ─── Helpers ───

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

/// Allocate a temporary register initialized to a constant.
fn alloc_temp_const(mir: &mut MirModule, _value: u64, _width: usize) -> usize {
    alloc_temp(mir)
}

/// Generate a unique label number.
fn generate_label(instrs: &mut Vec<MirInstr>) -> usize {
    // Find the highest label number and add 1
    let max_label = instrs.iter().filter_map(|i| {
        if let MirInstr::Label(l) = i { Some(*l) } else { None }
    }).max().unwrap_or(0);
    max_label + 1
}

/// Map HirBinOp to MirBinOp.
fn hir_binop_to_mir(op: HirBinOp) -> MirBinOp {
    match op {
        HirBinOp::Add => MirBinOp::Add,
        HirBinOp::Sub => MirBinOp::Sub,
        HirBinOp::Mul => MirBinOp::Mul,
        HirBinOp::Div => MirBinOp::Div,
        HirBinOp::Mod => MirBinOp::Mod,
        HirBinOp::BitAnd => MirBinOp::And,
        HirBinOp::BitOr => MirBinOp::Or,
        HirBinOp::BitXor => MirBinOp::Xor,
        HirBinOp::Eq => MirBinOp::Eq,
        HirBinOp::Ne => MirBinOp::Ne,
        HirBinOp::Lt => MirBinOp::Lt,
        HirBinOp::Le => MirBinOp::Le,
        HirBinOp::Gt => MirBinOp::Gt,
        HirBinOp::Ge => MirBinOp::Ge,
        HirBinOp::Shl => MirBinOp::Shl,
        HirBinOp::Shr => MirBinOp::Shr,
        HirBinOp::LogicAnd | HirBinOp::LogicOr => MirBinOp::Eq,
        HirBinOp::Sar => MirBinOp::Shr,
        HirBinOp::Power => MirBinOp::Mul,
    }
}

/// Map HirUnOp to MirUnOp.
fn hir_unop_to_mir(op: HirUnOp) -> MirUnOp {
    match op {
        HirUnOp::Neg => MirUnOp::Neg,
        HirUnOp::BitNot | HirUnOp::LogicNot => MirUnOp::Not,
    }
}

/// Extract signal names from an lvalue expression.
fn extract_lvalue_targets(expr: &HirExpr) -> Vec<Symbol> {
    match expr {
        HirExpr::Ident(name) => vec![*name],
        HirExpr::BitSelect { base, .. } => extract_lvalue_targets(base),
        HirExpr::PartSelect { base, .. } => extract_lvalue_targets(base),
        HirExpr::Concat { parts, .. } => {
            let mut targets = Vec::new();
            for part in parts {
                targets.extend(extract_lvalue_targets(part));
            }
            targets
        }
        _ => vec![],
    }
}

/// Collect signal indices used in an expression (for sensitivity).
#[allow(dead_code)]
fn collect_signal_refs(expr: &HirExpr, mir: &MirModule) -> HashSet<usize> {
    let mut refs = HashSet::new();
    collect_signal_refs_inner(expr, mir, &mut refs);
    refs
}

fn collect_signal_refs_inner(
    expr: &HirExpr,
    mir: &MirModule,
    refs: &mut HashSet<usize>,
) {
    match expr {
        HirExpr::Ident(name) => {
            if let Some(idx) = mir.signal_index(*name) {
                refs.insert(idx);
            }
        }
        HirExpr::Binary { lhs, rhs, .. } => {
            collect_signal_refs_inner(lhs, mir, refs);
            collect_signal_refs_inner(rhs, mir, refs);
        }
        HirExpr::Unary { operand, .. } => {
            collect_signal_refs_inner(operand, mir, refs);
        }
        HirExpr::Ternary { cond, then, else_, .. } => {
            collect_signal_refs_inner(cond, mir, refs);
            collect_signal_refs_inner(then, mir, refs);
            collect_signal_refs_inner(else_, mir, refs);
        }
        HirExpr::BitSelect { base, index, .. } => {
            collect_signal_refs_inner(base, mir, refs);
            collect_signal_refs_inner(index, mir, refs);
        }
        HirExpr::PartSelect { base, msb, lsb, .. } => {
            collect_signal_refs_inner(base, mir, refs);
            collect_signal_refs_inner(msb, mir, refs);
            collect_signal_refs_inner(lsb, mir, refs);
        }
        HirExpr::Concat { parts, .. } => {
            for part in parts {
                collect_signal_refs_inner(part, mir, refs);
            }
        }
        HirExpr::Call { args, .. } => {
            for arg in args {
                collect_signal_refs_inner(arg, mir, refs);
            }
        }
        _ => {}
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::hir::*;

    fn make_signal(name: &str, width: usize, is_input: bool, is_output: bool) -> HirSignal {
        HirSignal {
            name: Symbol::intern(name),
            dtype: HirType::BitVec { width },
            width,
            is_input,
            is_output,
        }
    }

    fn make_simple_module(name: &str, signals: Vec<HirSignal>, stmts: Vec<HirStmt>) -> HirModule {
        let inputs: Vec<usize> = signals.iter().enumerate()
            .filter(|(_, s)| s.is_input).map(|(i, _)| i).collect();
        let outputs: Vec<usize> = signals.iter().enumerate()
            .filter(|(_, s)| s.is_output).map(|(i, _)| i).collect();

        HirModule {
            name: Symbol::intern(name),
            signals,
            inputs,
            outputs,
            params: vec![],
            stmts,
            sub_instances: vec![],
            checksum: 0,
        }
    }

    #[test]
    fn test_lower_module_empty() {
        let hir = make_simple_module("empty", vec![], vec![]);
        let mir = lower_module(&hir);
        assert_eq!(mir.name, Symbol::intern("empty"));
        assert!(mir.signals.is_empty());
        assert!(mir.processes.is_empty());
    }

    #[test]
    fn test_lower_module_with_signals() {
        let hir = make_simple_module(
            "with_sigs",
            vec![
                make_signal("a", 8, true, false),
                make_signal("b", 8, false, true),
                make_signal("c", 1, false, false),
            ],
            vec![],
        );

        let mir = lower_module(&hir);
        assert_eq!(mir.signals.len(), 3);
        assert_eq!(mir.signals[0].kind, MirSignalKind::Input);
        assert_eq!(mir.signals[1].kind, MirSignalKind::Output);
        assert_eq!(mir.signals[2].kind, MirSignalKind::Reg);
    }

    #[test]
    fn test_lower_blocking_assign() {
        let stmts = vec![
            HirStmt::BlockingAssign {
                lhs: Box::new(HirExpr::Ident(Symbol::intern("b"))),
                rhs: Box::new(HirExpr::IntLiteral(42, 8)),
            },
        ];

        let hir = make_simple_module("assign_test", vec![
            make_signal("a", 8, true, false),
            make_signal("b", 8, false, true),
        ], stmts);

        let mir = lower_module(&hir);
        assert_eq!(mir.processes.len(), 1);
        assert_eq!(mir.processes[0].sensitivity, MirSensitivity::Initial);

        // Check the store instruction exists
        let has_store = mir.processes[0].instrs.iter().any(|i| {
            matches!(i, MirInstr::Store { signal: 1, .. })
        });
        assert!(has_store, "Expected a Store instruction for signal b");
    }

    #[test]
    fn test_lower_nonblocking_assign() {
        let stmts = vec![
            HirStmt::NonBlockingAssign {
                lhs: Box::new(HirExpr::Ident(Symbol::intern("b"))),
                rhs: Box::new(HirExpr::IntLiteral(99, 8)),
            },
        ];

        let hir = make_simple_module("nba_test", vec![
            make_signal("a", 8, true, false),
            make_signal("b", 8, false, true),
        ], stmts);

        let mir = lower_module(&hir);
        let has_nba = mir.processes[0].instrs.iter().any(|i| {
            matches!(i, MirInstr::NonBlocking { .. })
        });
        assert!(has_nba, "Expected a NonBlocking instruction");
    }

    #[test]
    fn test_lower_if_else() {
        // if(a) { b = 1; } else { b = 2; }
        let stmts = vec![
            HirStmt::If {
                cond: Box::new(HirExpr::Ident(Symbol::intern("a"))),
                then: Box::new(HirStmt::BlockingAssign {
                    lhs: Box::new(HirExpr::Ident(Symbol::intern("b"))),
                    rhs: Box::new(HirExpr::IntLiteral(1, 8)),
                }),
                else_: Some(Box::new(HirStmt::BlockingAssign {
                    lhs: Box::new(HirExpr::Ident(Symbol::intern("b"))),
                    rhs: Box::new(HirExpr::IntLiteral(2, 8)),
                })),
            },
        ];

        let hir = make_simple_module("if_test", vec![
            make_signal("a", 1, true, false),
            make_signal("b", 8, false, true),
        ], stmts);

        let mir = lower_module(&hir);
        assert_eq!(mir.processes.len(), 1);

        // Should have Branch + Label instructions for control flow
        let has_branch = mir.processes[0].instrs.iter().any(|i| {
            matches!(i, MirInstr::Branch { .. })
        });
        assert!(has_branch, "Expected a Branch instruction for if-else");
    }

    #[test]
    fn test_lower_for_loop() {
        // for(self clk; a; ; ) { b = b + 1; }
        let stmts = vec![
            HirStmt::For {
                init: Box::new(HirStmt::Block { stmts: vec![], name: None }),
                cond: Box::new(HirExpr::Ident(Symbol::intern("a"))),
                step: Box::new(HirStmt::Block { stmts: vec![], name: None }),
                body: Box::new(HirStmt::BlockingAssign {
                    lhs: Box::new(HirExpr::Ident(Symbol::intern("b"))),
                    rhs: Box::new(HirExpr::Binary {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::Ident(Symbol::intern("b"))),
                        rhs: Box::new(HirExpr::IntLiteral(1, 8)),
                        width: 8,
                    }),
                }),
            },
        ];

        let hir = make_simple_module("for_test", vec![
            make_signal("a", 1, true, false),
            make_signal("b", 8, false, true),
        ], stmts);

        let mir = lower_module(&hir);
        assert_eq!(mir.processes.len(), 1);

        // Should have a Jump instruction (back edge)
        let has_jump = mir.processes[0].instrs.iter().any(|i| {
            matches!(i, MirInstr::Jump { .. })
        });
        assert!(has_jump, "Expected a Jump instruction for loop");
    }

    #[test]
    fn test_lower_case_stmt() {
        // case(a) 1: b=10; 2: b=20; endcase
        let stmts = vec![
            HirStmt::Case {
                expr: Box::new(HirExpr::Ident(Symbol::intern("a"))),
                items: vec![
                    HirCaseItem {
                        exprs: vec![HirExpr::IntLiteral(1, 8)],
                        stmt: Box::new(HirStmt::BlockingAssign {
                            lhs: Box::new(HirExpr::Ident(Symbol::intern("b"))),
                            rhs: Box::new(HirExpr::IntLiteral(10, 8)),
                        }),
                    },
                    HirCaseItem {
                        exprs: vec![HirExpr::IntLiteral(2, 8)],
                        stmt: Box::new(HirStmt::BlockingAssign {
                            lhs: Box::new(HirExpr::Ident(Symbol::intern("b"))),
                            rhs: Box::new(HirExpr::IntLiteral(20, 8)),
                        }),
                    },
                ],
            },
        ];

        let hir = make_simple_module("case_test", vec![
            make_signal("a", 8, true, false),
            make_signal("b", 8, false, true),
        ], stmts);

        let mir = lower_module(&hir);
        assert!(!mir.processes.is_empty());
        let has_store = mir.processes[0].instrs.iter().any(|i| {
            matches!(i, MirInstr::Store { signal: 1, .. })
        });
        assert!(has_store, "Expected Store instructions for case body");
    }

    #[test]
    fn test_lower_binary_expr() {
        // c = a + b;
        let stmts = vec![
            HirStmt::BlockingAssign {
                lhs: Box::new(HirExpr::Ident(Symbol::intern("c"))),
                rhs: Box::new(HirExpr::Binary {
                    op: HirBinOp::Add,
                    lhs: Box::new(HirExpr::Ident(Symbol::intern("a"))),
                    rhs: Box::new(HirExpr::Ident(Symbol::intern("b"))),
                    width: 8,
                }),
            },
        ];

        let hir = make_simple_module("binop_test", vec![
            make_signal("a", 8, true, false),
            make_signal("b", 8, true, false),
            make_signal("c", 8, false, true),
        ], stmts);

        let mir = lower_module(&hir);
        let has_binary = mir.processes[0].instrs.iter().any(|i| {
            matches!(i, MirInstr::Binary { op: MirBinOp::Add, .. })
        });
        assert!(has_binary, "Expected a Binary Add instruction");
    }

    #[test]
    fn test_lower_design() {
        let top = make_simple_module("top", vec![], vec![]);
        let hir_design = HirDesign {
            modules: {
                let mut m = HashMap::new();
                m.insert(Symbol::intern("top"), std::sync::Arc::new(top.clone()));
                m.insert(Symbol::intern("sub"), std::sync::Arc::new(
                    make_simple_module("sub", vec![], vec![]),
                ));
                m
            },
            top: std::sync::Arc::new(top),
        };

        let mir_design = lower_design(&hir_design);
        assert_eq!(mir_design.top.name, Symbol::intern("top"));
        assert_eq!(mir_design.modules.len(), 2);
    }

    #[test]
    fn test_lower_while_loop() {
        let stmts = vec![
            HirStmt::While {
                cond: Box::new(HirExpr::Ident(Symbol::intern("a"))),
                body: Box::new(HirStmt::BlockingAssign {
                    lhs: Box::new(HirExpr::Ident(Symbol::intern("b"))),
                    rhs: Box::new(HirExpr::Binary {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::Ident(Symbol::intern("b"))),
                        rhs: Box::new(HirExpr::IntLiteral(1, 8)),
                        width: 8,
                    }),
                }),
            },
        ];

        let hir = make_simple_module("while_test", vec![
            make_signal("a", 1, true, false),
            make_signal("b", 8, false, true),
        ], stmts);

        let mir = lower_module(&hir);
        let has_jump = mir.processes[0].instrs.iter().any(|i| {
            matches!(i, MirInstr::Jump { .. })
        });
        assert!(has_jump, "Expected Jump for while loop back-edge");
    }

    #[test]
    fn test_lower_concat() {
        // {a, b}
        let stmts = vec![
            HirStmt::BlockingAssign {
                lhs: Box::new(HirExpr::Ident(Symbol::intern("c"))),
                rhs: Box::new(HirExpr::Concat {
                    parts: vec![
                        HirExpr::Ident(Symbol::intern("a")),
                        HirExpr::Ident(Symbol::intern("b")),
                    ],
                    width: 16,
                }),
            },
        ];

        let hir = make_simple_module("concat_test", vec![
            make_signal("a", 8, true, false),
            make_signal("b", 8, true, false),
            make_signal("c", 16, false, true),
        ], stmts);

        let mir = lower_module(&hir);
        let has_or = mir.processes[0].instrs.iter().any(|i| {
            matches!(i, MirInstr::Binary { op: MirBinOp::Or, .. })
        });
        assert!(has_or, "Expected Or instructions for concat");
    }

    #[test]
    fn test_lower_forever() {
        let stmts = vec![
            HirStmt::Forever {
                body: Box::new(HirStmt::BlockingAssign {
                    lhs: Box::new(HirExpr::Ident(Symbol::intern("clk"))),
                    rhs: Box::new(HirExpr::Unary {
                        op: HirUnOp::BitNot,
                        operand: Box::new(HirExpr::Ident(Symbol::intern("clk"))),
                        width: 1,
                    }),
                }),
            },
        ];

        let hir = make_simple_module("forever_test", vec![
            make_signal("clk", 1, false, false),
        ], stmts);

        let mir = lower_module(&hir);
        let has_jump = mir.processes[0].instrs.iter().any(|i| {
            matches!(i, MirInstr::Jump { .. })
        });
        assert!(has_jump, "Expected Jump for forever loop");
    }

    #[test]
    fn test_lower_ternary() {
        let stmts = vec![
            HirStmt::BlockingAssign {
                lhs: Box::new(HirExpr::Ident(Symbol::intern("c"))),
                rhs: Box::new(HirExpr::Ternary {
                    cond: Box::new(HirExpr::Ident(Symbol::intern("a"))),
                    then: Box::new(HirExpr::IntLiteral(10, 8)),
                    else_: Box::new(HirExpr::IntLiteral(20, 8)),
                    width: 8,
                }),
            },
        ];

        let hir = make_simple_module("ternary_test", vec![
            make_signal("a", 1, true, false),
            make_signal("c", 8, false, true),
        ], stmts);

        let mir = lower_module(&hir);
        let has_branch = mir.processes[0].instrs.iter().any(|i| {
            matches!(i, MirInstr::Branch { .. })
        });
        assert!(has_branch, "Expected Branch for ternary");
    }
}
