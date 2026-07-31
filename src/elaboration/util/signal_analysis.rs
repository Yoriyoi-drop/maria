//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan util.rs (SRP Refactoring).
//! Tanggung jawab: Signal reading analysis & combinational sensitivity.
//!
//! Fungsi:
//!   - infer_comb_sensitivity()     — inferensi sensitivity combinational
//!   - collect_read_signals_stmts() — kumpulkan signal baca dari statements
//!   - collect_read_signals_stmt()  — kumpulkan signal baca dari satu statement
//!   - collect_read_signals_expr()  — kumpulkan signal baca dari expression
//!   - resolve_expr_signal()        — resolusi expression ke signal ID
//!   - collect_sensitivity()        — kumpulkan sensitivity list dari expr
//!   - detect_sync_reset()          — deteksi reset sinkron dari body process
//!
//! ──────────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;

use crate::ast::*;
use crate::intern::Symbol;
use crate::ir::*;

/// Inferensi sensitivity list untuk process combinational dari body IR.
pub fn infer_comb_sensitivity(body: &[IrStmt]) -> Vec<SignalId> {
    let mut sigs = Vec::new();
    collect_read_signals_stmts(body, &mut sigs);
    sigs.sort();
    sigs.dedup();
    sigs
}

/// Kumpulkan semua signal yang dibaca dari daftar IR statements.
pub fn collect_read_signals_stmts(stmts: &[IrStmt], out: &mut Vec<SignalId>) {
    for stmt in stmts {
        collect_read_signals_stmt(stmt, out);
    }
}

/// Kumpulkan semua signal yang dibaca dari satu IR statement.
pub fn collect_read_signals_stmt(stmt: &IrStmt, out: &mut Vec<SignalId>) {
    match stmt {
        IrStmt::Block { stmts } => collect_read_signals_stmts(stmts, out),
        IrStmt::BlockingAssign { rhs, .. } | IrStmt::NonBlockingAssign { rhs, .. } => {
            collect_read_signals_expr(rhs, out);
        }
        IrStmt::If {
            cond,
            true_branch,
            false_branch,
        } => {
            collect_read_signals_expr(cond, out);
            collect_read_signals_stmts(true_branch, out);
            collect_read_signals_stmts(false_branch, out);
        }
        IrStmt::Case {
            expr,
            items,
            default,
            ..
        } => {
            collect_read_signals_expr(expr, out);
            for item in items {
                for label in &item.labels {
                    collect_read_signals_expr(label, out);
                }
                collect_read_signals_stmts(&item.body, out);
            }
            collect_read_signals_stmts(default, out);
        }
        IrStmt::Delay { body, .. } => collect_read_signals_stmts(body, out),
        IrStmt::Wait { cond, body } => {
            collect_read_signals_expr(cond, out);
            collect_read_signals_stmts(body, out);
        }
        IrStmt::SysCall { args, .. } => {
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrStmt::LoopWhile { cond, body } => {
            collect_read_signals_expr(cond, out);
            collect_read_signals_stmts(body, out);
        }
        IrStmt::LoopDoWhile { cond, body } => {
            collect_read_signals_expr(cond, out);
            collect_read_signals_stmts(body, out);
        }
        IrStmt::LoopFor {
            init,
            cond,
            step,
            body,
        } => {
            if let Some(s) = init {
                collect_read_signals_stmt(s, out);
            }
            collect_read_signals_expr(cond, out);
            if let Some(s) = step {
                collect_read_signals_stmt(s, out);
            }
            collect_read_signals_stmts(body, out);
        }
        IrStmt::EventControl { sig_id, body, .. } => {
            out.push(*sig_id);
            collect_read_signals_stmts(body, out);
        }
        IrStmt::EventTrigger { sig_id } => {
            out.push(*sig_id);
        }
        IrStmt::MethodCallStmt { obj, args, .. } => {
            collect_read_signals_expr(obj, out);
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrStmt::NamedBlock { stmts, .. } => {
            collect_read_signals_stmts(stmts, out);
        }
        IrStmt::Release { .. } | IrStmt::Deassign { .. } => {}
        IrStmt::Force { rhs, .. } => {
            collect_read_signals_expr(rhs, out);
        }
        IrStmt::Disable { .. } => {}
        IrStmt::RandCase { items } => {
            for (w_expr, body) in items {
                collect_read_signals_expr(w_expr, out);
                collect_read_signals_stmts(body, out);
            }
        }
        _ => {}
    }
}

/// Kumpulkan semua signal yang dibaca dari satu IR expression.
pub fn collect_read_signals_expr(expr: &IrExpr, out: &mut Vec<SignalId>) {
    match expr {
        IrExpr::Signal(id, _)
        | IrExpr::RangeSelect(id, ..)
        | IrExpr::BitSelect(id, _)
        | IrExpr::ArrayIndex { sig_id: id, .. } => {
            out.push(*id);
        }
        IrExpr::Const(_) | IrExpr::String(_) | IrExpr::FillLit(_) => {}
        IrExpr::Concat(exprs) => {
            for e in exprs {
                collect_read_signals_expr(e, out);
            }
        }
        IrExpr::Replicate(_, inner) => {
            collect_read_signals_expr(inner, out);
        }
        IrExpr::UnaryOp(_, inner) => collect_read_signals_expr(inner, out),
        IrExpr::BinaryOp(_, lhs, rhs) => {
            collect_read_signals_expr(lhs, out);
            collect_read_signals_expr(rhs, out);
        }
        IrExpr::Cond(c, t, f) => {
            collect_read_signals_expr(c, out);
            collect_read_signals_expr(t, out);
            collect_read_signals_expr(f, out);
        }
        IrExpr::Signed(inner) => collect_read_signals_expr(inner, out),
        IrExpr::NewCall { args, .. } => {
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrExpr::This => {}
        IrExpr::SysFunc { args, .. } => {
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrExpr::MethodCall { obj, args, .. } => {
            collect_read_signals_expr(obj, out);
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrExpr::MemberAccess { obj, .. } => {
            collect_read_signals_expr(obj, out);
        }
        IrExpr::ExprRangeSelect(inner, _, _) => {
            collect_read_signals_expr(inner, out);
        }
        IrExpr::ExprBitSelect(inner, _) => {
            collect_read_signals_expr(inner, out);
        }
        IrExpr::ExprPartSelect(inner, base_expr, width_expr) => {
            collect_read_signals_expr(inner, out);
            collect_read_signals_expr(base_expr, out);
            collect_read_signals_expr(width_expr, out);
        }
        IrExpr::DpiCall { args, .. } => {
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrExpr::HierRef(_) => {}
        IrExpr::Inside { expr, list } => {
            collect_read_signals_expr(expr, out);
            for item in list {
                collect_read_signals_expr(item, out);
            }
        }
        IrExpr::Cast { expr, .. } => {
            collect_read_signals_expr(expr, out);
        }
        IrExpr::Dist { expr, .. } => {
            collect_read_signals_expr(expr, out);
        }
        IrExpr::StreamingConcat { slices, .. } => {
            for e in slices {
                collect_read_signals_expr(e, out);
            }
        }
        IrExpr::UdpLookup { args, .. } => {
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrExpr::VifBinding { .. } => {}
        IrExpr::VirtualIfaceAccess { .. } => {}
        IrExpr::FuncCall { args, .. } => {
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
    }
}

/// Resolusi expression AST ke signal ID (jika berupa signal sederhana).
pub fn resolve_expr_signal(
    expr: &Expr,
    signal_map: &HashMap<Symbol, SignalId>,
) -> Option<SignalId> {
    match expr {
        Expr::Ident { name, .. } => signal_map.get(name).copied(),
        Expr::MethodCall { .. } => None,
        Expr::MemberAccess { .. } => None,
        _ => None,
    }
}

/// Deteksi reset sinkron dari body IR process.
/// Cari pola: if (signal) sebagai statement pertama.
pub fn detect_sync_reset(body: &[IrStmt]) -> Option<ResetInfo> {
    if let Some(IrStmt::If {
        cond: IrExpr::Signal(sig_id, _),
        ..
    }) = body.first()
    {
        return Some(ResetInfo {
            signal: *sig_id,
            polarity: true,
            r#async: false,
            value: LogicVec::new(1),
        });
    }
    None
}

/// Kumpulkan sensitivity list dari expression AST (untuk always_comb).
pub fn collect_sensitivity(expr: &Expr, signal_map: &HashMap<Symbol, SignalId>) -> Vec<SignalId> {
    match expr {
        Expr::Ident { name, .. } => signal_map.get(name).map(|&id| vec![id]).unwrap_or_default(),
        Expr::BinaryOp { lhs, rhs, .. } => {
            let mut v = collect_sensitivity(lhs, signal_map);
            v.extend(collect_sensitivity(rhs, signal_map));
            v
        }
        Expr::UnaryOp { expr: inner, .. } => collect_sensitivity(inner, signal_map),
        Expr::Concat(exprs) => exprs
            .iter()
            .flat_map(|e| collect_sensitivity(e, signal_map))
            .collect(),
        Expr::BitSelect { expr: inner, index } => {
            let mut v = collect_sensitivity(inner, signal_map);
            v.extend(collect_sensitivity(index, signal_map));
            v
        }
        Expr::RangeSelect {
            expr: inner,
            msb,
            lsb,
        } => {
            let mut v = collect_sensitivity(inner, signal_map);
            v.extend(collect_sensitivity(msb, signal_map));
            v.extend(collect_sensitivity(lsb, signal_map));
            v
        }
        Expr::PartSelect {
            expr: inner,
            base,
            width,
        } => {
            let mut v = collect_sensitivity(inner, signal_map);
            v.extend(collect_sensitivity(base, signal_map));
            v.extend(collect_sensitivity(width, signal_map));
            v
        }
        Expr::TernaryOp {
            cond,
            true_expr,
            false_expr,
        } => {
            let mut v = collect_sensitivity(cond, signal_map);
            v.extend(collect_sensitivity(true_expr, signal_map));
            v.extend(collect_sensitivity(false_expr, signal_map));
            v
        }
        Expr::MethodCall { obj, .. } => collect_sensitivity(obj, signal_map),
        Expr::MemberAccess { obj, .. } => collect_sensitivity(obj, signal_map),
        Expr::Dist { expr, .. } => collect_sensitivity(expr, signal_map),
        _ => vec![],
    }
}
