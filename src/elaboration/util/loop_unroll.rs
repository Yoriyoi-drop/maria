//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan util.rs (SRP Refactoring).
//! Tanggung jawab: Loop unrolling & variable substitution (genvar, loop vars).
//!
//! Fungsi:
//!   - try_unroll_for_loop()          — unroll for loop dengan konstanta
//!   - substitute_loop_var_in_stmts() — substitusi loop var di statements
//!   - substitute_loop_var_in_stmt()  — substitusi loop var di satu statement
//!   - substitute_loop_var_in_expr()  — substitusi loop var di expression
//!   - substitute_sensitivity_event() — substitusi loop var di sensitivity event
//! ──────────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;

use crate::ast::types::const_eval_with_params;
use crate::ast::*;
use crate::intern::Symbol;
use crate::ir::*;

/// Coba unroll for loop dengan nilai parameter yang diketahui.
/// Mengembalikan Some(vec) jika berhasil di-unroll, None jika tidak bisa.
pub fn try_unroll_for_loop<'a, F>(
    init: Option<&'a Stmt>,
    cond: Option<&'a Expr>,
    step: Option<&'a Stmt>,
    stmts: &[Stmt],
    elaborate_body: &F,
    params: &HashMap<Symbol, i64>,
) -> Result<Option<Vec<IrStmt>>, String>
where
    F: Fn(&[Stmt], &str, i64) -> Result<Vec<IrStmt>, String>,
{
    let (var_name, init_val) = match init {
        Some(Stmt::BlockingAssign {
            lhs: Expr::Ident(name),
            rhs,
            ..
        }) => (*name, const_eval_with_params(rhs, params)?),
        _ => return Ok(None),
    };

    let step_fn: Box<dyn Fn(i64) -> Result<i64, String>> = match step {
        Some(Stmt::BlockingAssign {
            lhs: Expr::Ident(n),
            rhs,
            ..
        }) if *n == var_name => match rhs {
            Expr::BinaryOp {
                op: BinaryOp::Add,
                lhs,
                rhs,
            } => {
                if let Expr::Ident(n2) = lhs.as_ref() {
                    if n2 == &var_name {
                        let inc = const_eval_with_params(rhs, params)?;
                        Box::new(move |v| Ok(v + inc))
                    } else {
                        return Ok(None);
                    }
                } else if let Expr::Ident(n2) = rhs.as_ref() {
                    if n2 == &var_name {
                        let inc = const_eval_with_params(lhs, params)?;
                        Box::new(move |v| Ok(v + inc))
                    } else {
                        return Ok(None);
                    }
                } else {
                    return Ok(None);
                }
            }
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };

    let limit = match cond {
        Some(Expr::BinaryOp {
            op: BinaryOp::Lt,
            lhs,
            rhs,
        }) => match lhs.as_ref() {
            Expr::Ident(n) if *n == var_name => const_eval_with_params(rhs, params)?,
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };

    let mut all_stmts = Vec::new();
    let mut ivar = init_val;
    while ivar < limit {
        let body = elaborate_body(stmts, var_name.as_str(), ivar)?;
        all_stmts.extend(body);
        ivar = step_fn(ivar)?;
    }

    Ok(Some(all_stmts))
}

/// Substitusi loop variable di semua statements.
pub fn substitute_loop_var_in_stmts(stmts: &[Stmt], var_name: &str, value: i64) -> Vec<Stmt> {
    stmts
        .iter()
        .map(|s| substitute_loop_var_in_stmt(s, var_name, value))
        .collect()
}

/// Substitusi loop variable di satu statement.
pub fn substitute_loop_var_in_stmt(stmt: &Stmt, var_name: &str, value: i64) -> Stmt {
    match stmt {
        Stmt::Block { stmts } => Stmt::Block {
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::BlockingAssign { lhs, rhs, delay } => Stmt::BlockingAssign {
            lhs: substitute_loop_var_in_expr(lhs, var_name, value),
            rhs: substitute_loop_var_in_expr(rhs, var_name, value),
            delay: delay.clone(),
        },
        Stmt::NonBlockingAssign { lhs, rhs, delay } => Stmt::NonBlockingAssign {
            lhs: substitute_loop_var_in_expr(lhs, var_name, value),
            rhs: substitute_loop_var_in_expr(rhs, var_name, value),
            delay: delay.clone(),
        },
        Stmt::IfElse {
            cond,
            true_branch,
            false_branch,
        } => Stmt::IfElse {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            true_branch: Box::new(substitute_loop_var_in_stmt(true_branch, var_name, value)),
            false_branch: false_branch
                .as_ref()
                .map(|fb| Box::new(substitute_loop_var_in_stmt(fb, var_name, value))),
        },
        Stmt::Case {
            expr,
            items,
            default,
        } => Stmt::Case {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items
                .iter()
                .map(|item| crate::ast::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| substitute_loop_var_in_expr(l, var_name, value))
                        .collect(),
                    stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
                })
                .collect(),
            default: default
                .as_ref()
                .map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::StmtAssign { lhs, rhs } => Stmt::StmtAssign {
            lhs: substitute_loop_var_in_expr(lhs, var_name, value),
            rhs: substitute_loop_var_in_expr(rhs, var_name, value),
        },
        Stmt::Delay { delay, stmt } => Stmt::Delay {
            delay: substitute_loop_var_in_expr(delay, var_name, value),
            stmt: Box::new(substitute_loop_var_in_stmt(stmt, var_name, value)),
        },
        Stmt::SysCall { name, args } => Stmt::SysCall {
            name: name.clone(),
            args: args
                .iter()
                .map(|a| substitute_loop_var_in_expr(a, var_name, value))
                .collect(),
        },
        Stmt::Expr { expr } => Stmt::Expr {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
        },
        Stmt::CaseX {
            expr,
            items,
            default,
        } => Stmt::CaseX {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items
                .iter()
                .map(|item| crate::ast::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| substitute_loop_var_in_expr(l, var_name, value))
                        .collect(),
                    stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
                })
                .collect(),
            default: default
                .as_ref()
                .map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::CaseZ {
            expr,
            items,
            default,
        } => Stmt::CaseZ {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items
                .iter()
                .map(|item| crate::ast::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| substitute_loop_var_in_expr(l, var_name, value))
                        .collect(),
                    stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
                })
                .collect(),
            default: default
                .as_ref()
                .map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::StmtCase {
            expr,
            items,
            default,
        } => Stmt::StmtCase {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items
                .iter()
                .map(|item| crate::ast::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| substitute_loop_var_in_expr(l, var_name, value))
                        .collect(),
                    stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
                })
                .collect(),
            default: default
                .as_ref()
                .map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::LoopForever { stmts } => Stmt::LoopForever {
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::LoopWhile { cond, stmts } => Stmt::LoopWhile {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::LoopFor {
            init,
            cond,
            step,
            stmts,
        } => Stmt::LoopFor {
            init: init
                .as_ref()
                .map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
            cond: cond
                .as_ref()
                .map(|c| substitute_loop_var_in_expr(c, var_name, value)),
            step: step
                .as_ref()
                .map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::Repeat { count, stmts } => Stmt::Repeat {
            count: substitute_loop_var_in_expr(count, var_name, value),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::Wait { cond, stmt } => Stmt::Wait {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            stmt: stmt
                .as_ref()
                .map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
        },
        Stmt::Disable { name } => Stmt::Disable { name: name.clone() },
        Stmt::Force { lhs, rhs } => Stmt::Force {
            lhs: substitute_loop_var_in_expr(lhs, var_name, value),
            rhs: substitute_loop_var_in_expr(rhs, var_name, value),
        },
        Stmt::Release { expr } => Stmt::Release {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
        },
        Stmt::Deassign { expr } => Stmt::Deassign {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
        },
        Stmt::Return(expr) => Stmt::Return(
            expr.as_ref()
                .map(|e| Box::new(substitute_loop_var_in_expr(e, var_name, value))),
        ),
        Stmt::Null => Stmt::Null,
        Stmt::SysFinish => Stmt::SysFinish,
        Stmt::EventControl { events, stmt } => Stmt::EventControl {
            events: events
                .iter()
                .map(|e| substitute_sensitivity_event(e, var_name, value))
                .collect(),
            stmt: stmt
                .as_ref()
                .map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
        },
        Stmt::EventTrigger { name } => Stmt::EventTrigger { name: name.clone() },
        Stmt::ForeachLoop {
            array_var,
            index_vars,
            stmts,
        } => Stmt::ForeachLoop {
            array_var: array_var.clone(),
            index_vars: index_vars.clone(),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::NamedBlock { name, stmts, decls } => Stmt::NamedBlock {
            name: name.clone(),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
            decls: decls.clone(),
        },
        Stmt::RandCase { items } => Stmt::RandCase {
            items: items
                .iter()
                .map(|rc| RandCaseItem {
                    weight: rc.weight,
                    stmt: Box::new(substitute_loop_var_in_stmt(&rc.stmt, var_name, value)),
                })
                .collect(),
        },
        Stmt::Break => Stmt::Break,
        Stmt::Continue => Stmt::Continue,
        Stmt::DoWhile { cond, stmts } => Stmt::DoWhile {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        _ => stmt.clone(),
    }
}

/// Substitusi sensitivity event (posedge/negedge/level) dengan nilai loop var.
pub fn substitute_sensitivity_event(
    event: &SensitivityEvent,
    var_name: &str,
    value: i64,
) -> SensitivityEvent {
    match event {
        SensitivityEvent::PosEdge(e) => {
            SensitivityEvent::PosEdge(substitute_loop_var_in_expr(e, var_name, value))
        }
        SensitivityEvent::NegEdge(e) => {
            SensitivityEvent::NegEdge(substitute_loop_var_in_expr(e, var_name, value))
        }
        SensitivityEvent::Level(e) => {
            SensitivityEvent::Level(substitute_loop_var_in_expr(e, var_name, value))
        }
        SensitivityEvent::Wildcard => SensitivityEvent::Wildcard,
    }
}

/// Substitusi loop variable di expression AST.
pub fn substitute_loop_var_in_expr(expr: &Expr, var_name: &str, value: i64) -> Expr {
    match expr {
        Expr::Ident(name) if name == var_name => {
            Expr::Value(crate::ast::expr::Value::Decimal(value))
        }
        Expr::Ident(_) => expr.clone(),
        Expr::Value(_) | Expr::String(_) | Expr::Null => expr.clone(),
        Expr::RangeSelect {
            expr: inner,
            msb,
            lsb,
        } => Expr::RangeSelect {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            msb: Box::new(substitute_loop_var_in_expr(msb, var_name, value)),
            lsb: Box::new(substitute_loop_var_in_expr(lsb, var_name, value)),
        },
        Expr::BitSelect { expr: inner, index } => Expr::BitSelect {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            index: Box::new(substitute_loop_var_in_expr(index, var_name, value)),
        },
        Expr::PartSelect {
            expr: inner,
            base,
            width,
        } => Expr::PartSelect {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            base: Box::new(substitute_loop_var_in_expr(base, var_name, value)),
            width: Box::new(substitute_loop_var_in_expr(width, var_name, value)),
        },
        Expr::Concat(exprs) => Expr::Concat(
            exprs
                .iter()
                .map(|e| substitute_loop_var_in_expr(e, var_name, value))
                .collect(),
        ),
        Expr::FuncCall { name, args } => Expr::FuncCall {
            name: name.clone(),
            args: args
                .iter()
                .map(|a| substitute_loop_var_in_expr(a, var_name, value))
                .collect(),
        },
        Expr::Replicate { count, expr: inner } => Expr::Replicate {
            count: Box::new(substitute_loop_var_in_expr(count, var_name, value)),
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
        },
        Expr::UnaryOp { op, expr: inner } => Expr::UnaryOp {
            op: op.clone(),
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
        },
        Expr::BinaryOp { op, lhs, rhs } => Expr::BinaryOp {
            op: op.clone(),
            lhs: Box::new(substitute_loop_var_in_expr(lhs, var_name, value)),
            rhs: Box::new(substitute_loop_var_in_expr(rhs, var_name, value)),
        },
        Expr::TernaryOp {
            cond,
            true_expr,
            false_expr,
        } => Expr::TernaryOp {
            cond: Box::new(substitute_loop_var_in_expr(cond, var_name, value)),
            true_expr: Box::new(substitute_loop_var_in_expr(true_expr, var_name, value)),
            false_expr: Box::new(substitute_loop_var_in_expr(false_expr, var_name, value)),
        },
        Expr::Paren(inner) => Expr::Paren(Box::new(substitute_loop_var_in_expr(
            inner, var_name, value,
        ))),
        Expr::MethodCall {
            obj,
            method,
            args,
            with_clause,
        } => Expr::MethodCall {
            obj: Box::new(substitute_loop_var_in_expr(obj, var_name, value)),
            method: method.clone(),
            args: args
                .iter()
                .map(|a| substitute_loop_var_in_expr(a, var_name, value))
                .collect(),
            with_clause: with_clause
                .clone()
                .map(|wc| Box::new(substitute_loop_var_in_expr(&wc, var_name, value))),
        },
        Expr::MemberAccess { obj, field } => Expr::MemberAccess {
            obj: Box::new(substitute_loop_var_in_expr(obj, var_name, value)),
            field: field.clone(),
        },
        Expr::FillLit(val) => Expr::FillLit(*val),
        Expr::Inside {
            expr: inner,
            range_list,
        } => Expr::Inside {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            range_list: range_list
                .iter()
                .map(|e| substitute_loop_var_in_expr(e, var_name, value))
                .collect(),
        },
        Expr::StreamingConcat {
            op,
            slice_size,
            slices,
        } => Expr::StreamingConcat {
            op: op.clone(),
            slice_size: slice_size
                .as_ref()
                .map(|ss| Box::new(substitute_loop_var_in_expr(ss, var_name, value))),
            slices: slices
                .iter()
                .map(|e| substitute_loop_var_in_expr(e, var_name, value))
                .collect(),
        },
        Expr::Dist { expr, items } => Expr::Dist {
            expr: Box::new(substitute_loop_var_in_expr(expr, var_name, value)),
            items: items.clone(),
        },
        Expr::Cast { dtype, expr: inner } => Expr::Cast {
            dtype: dtype.clone(),
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
        },
        Expr::ScopedIdent { package, item } => Expr::ScopedIdent {
            package: package.clone(),
            item: item.clone(),
        },
    }
}
