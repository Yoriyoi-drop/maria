use std::collections::HashMap;

use maria_core::intern::Symbol;
use maria_ir::*;

/// Collect all HierRef symbols from an IR expression tree.
pub fn collect_hier_refs_expr(expr: &IrExpr) -> Vec<Symbol> {
    match expr {
        IrExpr::HierRef(name) => vec![*name],
        IrExpr::BinaryOp(_, lhs, rhs) => {
            let mut v = collect_hier_refs_expr(lhs);
            v.extend(collect_hier_refs_expr(rhs));
            v
        }
        IrExpr::UnaryOp(_, inner) | IrExpr::Cast { expr: inner, .. } | IrExpr::Signed(inner) => {
            collect_hier_refs_expr(inner)
        }
        IrExpr::Concat(exprs) => exprs
            .iter()
            .flat_map(|e| collect_hier_refs_expr(e))
            .collect(),
        IrExpr::ExprBitSelect(inner, _) => collect_hier_refs_expr(inner),
        IrExpr::ExprRangeSelect(inner, _, _) => collect_hier_refs_expr(inner),
        IrExpr::ExprPartSelect(base, idx, w) => {
            let mut v = collect_hier_refs_expr(base);
            v.extend(collect_hier_refs_expr(idx));
            v.extend(collect_hier_refs_expr(w));
            v
        }
        IrExpr::Cond(c, t, f) => {
            let mut v = collect_hier_refs_expr(c);
            v.extend(collect_hier_refs_expr(t));
            v.extend(collect_hier_refs_expr(f));
            v
        }
        IrExpr::MethodCall { obj, args, .. } => {
            let mut v = collect_hier_refs_expr(obj);
            for a in args {
                v.extend(collect_hier_refs_expr(a));
            }
            v
        }
        IrExpr::SysFunc { args, .. } => {
            let mut v = Vec::new();
            for a in args {
                v.extend(collect_hier_refs_expr(a));
            }
            v
        }
        IrExpr::FuncCall { args, .. } => {
            let mut v = Vec::new();
            for a in args {
                v.extend(collect_hier_refs_expr(a));
            }
            v
        }
        IrExpr::UdpLookup { args, .. } => {
            let mut v = Vec::new();
            for a in args {
                v.extend(collect_hier_refs_expr(a));
            }
            v
        }
        IrExpr::MemberAccess { obj, .. } => collect_hier_refs_expr(obj),
        IrExpr::Replicate(_, inner) => collect_hier_refs_expr(inner),
        IrExpr::ArrayIndex { index, .. } => collect_hier_refs_expr(index),
        IrExpr::Inside { expr, list } => {
            let mut v = collect_hier_refs_expr(expr);
            for a in list {
                v.extend(collect_hier_refs_expr(a));
            }
            v
        }
        IrExpr::InsideRange { expr, lo, hi } => {
            let mut v = collect_hier_refs_expr(expr);
            v.extend(collect_hier_refs_expr(lo));
            v.extend(collect_hier_refs_expr(hi));
            v
        }
        IrExpr::StreamingConcat { slices, .. } => {
            let mut v = Vec::new();
            for a in slices {
                v.extend(collect_hier_refs_expr(a));
            }
            v
        }
        IrExpr::Dist { expr, .. } => collect_hier_refs_expr(expr),
        _ => vec![],
    }
}

/// Collect all HierRef symbols from an IR statement tree.
pub fn collect_hier_refs_stmt(stmt: &IrStmt) -> Vec<Symbol> {
    match stmt {
        IrStmt::BlockingAssign { lhs, rhs, .. } | IrStmt::NonBlockingAssign { lhs, rhs, .. } => {
            let mut v = collect_hier_refs_expr(rhs);
            v.extend(collect_hier_refs_lvalue(lhs));
            v
        }
        IrStmt::Force { lvalue, rhs } => {
            let mut v = collect_hier_refs_expr(rhs);
            v.extend(collect_hier_refs_lvalue(lvalue));
            v
        }
        IrStmt::Release { lvalue } | IrStmt::Deassign { lvalue } => {
            collect_hier_refs_lvalue(lvalue)
        }
        IrStmt::SysCall { args, .. } => {
            let mut v = Vec::new();
            for a in args {
                v.extend(collect_hier_refs_expr(a));
            }
            v
        }
        IrStmt::If {
            cond,
            true_branch,
            false_branch,
        } => {
            let mut v = collect_hier_refs_expr(cond);
            for s in true_branch {
                v.extend(collect_hier_refs_stmt(s));
            }
            for s in false_branch {
                v.extend(collect_hier_refs_stmt(s));
            }
            v
        }
        IrStmt::LoopFor {
            init,
            cond,
            step,
            body,
        } => {
            let mut v = collect_hier_refs_expr(cond);
            if let Some(i) = init {
                v.extend(collect_hier_refs_stmt(i));
            }
            if let Some(s) = step {
                v.extend(collect_hier_refs_stmt(s));
            }
            for s in body {
                v.extend(collect_hier_refs_stmt(s));
            }
            v
        }
        IrStmt::LoopWhile { cond, body }
        | IrStmt::LoopDoWhile { cond, body }
        | IrStmt::Wait { cond, body } => {
            let mut v = collect_hier_refs_expr(cond);
            for s in body {
                v.extend(collect_hier_refs_stmt(s));
            }
            v
        }
        IrStmt::Repeat { count, body } => {
            let mut v = collect_hier_refs_expr(count);
            for s in body {
                v.extend(collect_hier_refs_stmt(s));
            }
            v
        }
        IrStmt::Case {
            expr, items, default, ..
        } => {
            let mut v = collect_hier_refs_expr(expr);
            for item in items {
                for s in &item.body {
                    v.extend(collect_hier_refs_stmt(s));
                }
            }
            for s in default {
                v.extend(collect_hier_refs_stmt(s));
            }
            v
        }
        IrStmt::Block { stmts } | IrStmt::NamedBlock { stmts, .. } => {
            let mut v = Vec::new();
            for s in stmts {
                v.extend(collect_hier_refs_stmt(s));
            }
            v
        }
        IrStmt::Delay { body, .. } => {
            let mut v = Vec::new();
            for s in body {
                v.extend(collect_hier_refs_stmt(s));
            }
            v
        }
        IrStmt::EventControl { body, iff, .. } => {
            let mut v = Vec::new();
            if let Some(cond) = iff {
                v.extend(collect_hier_refs_expr(cond));
            }
            for s in body {
                v.extend(collect_hier_refs_stmt(s));
            }
            v
        }
        IrStmt::MethodCallStmt { obj, args, .. } => {
            let mut v = collect_hier_refs_expr(obj);
            for a in args {
                v.extend(collect_hier_refs_expr(a));
            }
            v
        }
        IrStmt::Fork { processes, .. } => {
            let mut v = Vec::new();
            for branch in processes {
                for s in branch {
                    v.extend(collect_hier_refs_stmt(s));
                }
            }
            v
        }
        IrStmt::Assert {
            cond,
            pass_stmt,
            fail_stmt,
            ..
        }
        | IrStmt::Assume {
            cond,
            pass_stmt,
            fail_stmt,
            ..
        }
        | IrStmt::Expect {
            cond,
            pass_stmt,
            fail_stmt,
            ..
        } => {
            let mut v = collect_hier_refs_expr(cond);
            for s in pass_stmt {
                v.extend(collect_hier_refs_stmt(s));
            }
            for s in fail_stmt {
                v.extend(collect_hier_refs_stmt(s));
            }
            v
        }
        IrStmt::Cover {
            cond, pass_stmt, ..
        } => {
            let mut v = collect_hier_refs_expr(cond);
            for s in pass_stmt {
                v.extend(collect_hier_refs_stmt(s));
            }
            v
        }
        IrStmt::Foreach {
            array_var, body, ..
        } => {
            let mut v = collect_hier_refs_expr(array_var);
            for s in body {
                v.extend(collect_hier_refs_stmt(s));
            }
            v
        }
        IrStmt::RandCase { items } => {
            let mut v = Vec::new();
            for (cond, body) in items {
                v.extend(collect_hier_refs_expr(cond));
                for s in body {
                    v.extend(collect_hier_refs_stmt(s));
                }
            }
            v
        }
        _ => vec![],
    }
}

/// Collect HierRef symbols from an lvalue.
fn collect_hier_refs_lvalue(lvalue: &IrLValue) -> Vec<Symbol> {
    match lvalue {
        IrLValue::HierRef(name) => vec![*name],
        IrLValue::HierRefIndex { name, index, .. } => {
            let mut v = vec![*name];
            v.extend(collect_hier_refs_expr(index));
            v
        }
        IrLValue::ArrayIndex { index, .. }
        | IrLValue::ArrayRangeSelect { index, .. } => collect_hier_refs_expr(index),
        IrLValue::ArrayBitSelect {
            index, bit, ..
        } => {
            let mut v = collect_hier_refs_expr(index);
            v.extend(collect_hier_refs_expr(bit));
            v
        }
        IrLValue::ExprPartSelect { base, .. } => collect_hier_refs_expr(base),
        _ => vec![],
    }
}

/// Post-flatten sensitivity fixup: scan all combinational processes for HierRef
/// symbols and inject corresponding signal IDs from hier_signal_map into their
/// sensitivity lists.
///
/// This fixes the case where `assign y = uut.x` has empty sensitivity because
/// `collect_sensitivity` (AST-level) cannot resolve hierarchical names that only
/// exist in `hier_signal_map` (created by flatten_instances after module elaboration).
pub fn fix_hier_sensitivity(
    processes: &mut Vec<Process>,
    hier_signal_map: &HashMap<Symbol, SignalId>,
) {
    for proc in processes.iter_mut() {
        if let Process::Combinational {
            sensitivity,
            body,
            ..
        } = proc
        {
            let mut hier_refs = Vec::new();
            for stmt in body.iter() {
                hier_refs.extend(collect_hier_refs_stmt(stmt));
            }
            // Dedup using symbol equality (Symbol doesn't impl Ord)
            hier_refs.dedup_by(|a, b| a == b);
            for name in &hier_refs {
                if let Some(&sig_id) = hier_signal_map.get(name) {
                    let already_present = sensitivity.iter().any(|s| s.sig_id == sig_id);
                    if !already_present {
                        sensitivity.push(SignalSensitivity::whole(sig_id));
                    }
                }
            }
        }
    }
}
