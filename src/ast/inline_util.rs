use std::collections::{HashMap, HashSet};

use super::expr::Expr;
use super::expr::Value;
use super::stmt::Stmt;
use super::types::{DataType, Decl, DeclKind, FunctionDecl, FunctionPort};
use crate::intern::Symbol;
pub(crate) fn detect_recursive_functions(funcs: &HashMap<Symbol, FunctionDecl>) -> HashSet<Symbol> {
    let mut recursive = HashSet::new();
    // First pass: detect direct recursion
    for (name, func) in funcs {
        if stmt_has_func_call(name, &func.stmts) {
            recursive.insert(*name);
        }
    }
    recursive
}

/// Check if a function body contains calls to a specific function.
pub(crate) fn stmt_has_func_call(func_name: &Symbol, stmts: &[Stmt]) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Block { stmts: inner }
            | Stmt::NamedBlock { stmts: inner, .. }
            | Stmt::LoopForever { stmts: inner }
            | Stmt::LoopWhile { stmts: inner, .. }
            | Stmt::LoopFor { stmts: inner, .. }
            | Stmt::Repeat { stmts: inner, .. }
            | Stmt::DoWhile { stmts: inner, .. } => {
                if stmt_has_func_call(func_name, inner) {
                    return true;
                }
            }
            Stmt::IfElse {
                cond,
                true_branch,
                false_branch,
            } => {
                if expr_has_func_call(func_name, cond) {
                    return true;
                }
                if stmt_has_func_call(func_name, &[true_branch.as_ref().clone()]) {
                    return true;
                }
                if let Some(fb) = false_branch {
                    if stmt_has_func_call(func_name, &[fb.as_ref().clone()]) {
                        return true;
                    }
                }
            }
            Stmt::Case {
                expr,
                items,
                default,
            }
            | Stmt::CaseX {
                expr,
                items,
                default,
            }
            | Stmt::CaseZ {
                expr,
                items,
                default,
            }
            | Stmt::StmtCase {
                expr,
                items,
                default,
            }
            | Stmt::UniqueCase {
                expr,
                items,
                default,
            }
            | Stmt::PriorityCase {
                expr,
                items,
                default,
            }
            | Stmt::CaseInside {
                expr,
                items,
                default,
            } => {
                if expr_has_func_call(func_name, expr) {
                    return true;
                }
                for item in items {
                    for l in &item.labels {
                        if expr_has_func_call(func_name, l) {
                            return true;
                        }
                    }
                    if stmt_has_func_call(func_name, &[item.stmt.as_ref().clone()]) {
                        return true;
                    }
                }
                if let Some(d) = default {
                    if stmt_has_func_call(func_name, &[d.as_ref().clone()]) {
                        return true;
                    }
                }
            }
            Stmt::BlockingAssign { rhs, .. } | Stmt::NonBlockingAssign { rhs, .. } => {
                if expr_has_func_call(func_name, rhs) {
                    return true;
                }
            }
            Stmt::StmtAssign { lhs, rhs } => {
                if expr_has_func_call(func_name, lhs) || expr_has_func_call(func_name, rhs) {
                    return true;
                }
            }
            Stmt::Expr { expr } => {
                if expr_has_func_call(func_name, expr) {
                    return true;
                }
            }
            Stmt::Return(expr) => {
                if let Some(e) = expr {
                    if expr_has_func_call(func_name, &e) {
                        return true;
                    }
                }
            }
            Stmt::Wait { cond, stmt: wstmt } => {
                if expr_has_func_call(func_name, cond) {
                    return true;
                }
                if let Some(s) = wstmt {
                    if stmt_has_func_call(func_name, &[*s.clone()]) {
                        return true;
                    }
                }
            }
            Stmt::SysCall { args, .. } => {
                for arg in args {
                    if expr_has_func_call(func_name, arg) {
                        return true;
                    }
                }
            }
            Stmt::Fork { processes, .. } => {
                for p in processes {
                    if stmt_has_func_call(func_name, &[p.clone()]) {
                        return true;
                    }
                }
            }
            Stmt::Force { rhs, .. } => {
                if expr_has_func_call(func_name, rhs) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Check if an expression contains a call to a specific function.
pub(crate) fn expr_has_func_call(func_name: &Symbol, expr: &Expr) -> bool {
    match expr {
        Expr::FuncCall { name, args } => {
            if name == func_name {
                return true;
            }
            args.iter().any(|arg| expr_has_func_call(func_name, arg))
        }
        Expr::BinaryOp { lhs, rhs, .. } => {
            expr_has_func_call(func_name, lhs) || expr_has_func_call(func_name, rhs)
        }
        Expr::UnaryOp { expr: inner, .. } => expr_has_func_call(func_name, inner),
        Expr::TernaryOp {
            cond,
            true_expr,
            false_expr,
        } => {
            expr_has_func_call(func_name, cond)
                || expr_has_func_call(func_name, true_expr)
                || expr_has_func_call(func_name, false_expr)
        }
        Expr::Concat(exprs) => exprs.iter().any(|e| expr_has_func_call(func_name, e)),
        Expr::Replicate { expr: inner, .. } => expr_has_func_call(func_name, inner),
        Expr::Paren(inner) => expr_has_func_call(func_name, inner),
        Expr::RangeSelect {
            expr: inner,
            msb,
            lsb,
        } => {
            expr_has_func_call(func_name, inner)
                || expr_has_func_call(func_name, msb)
                || expr_has_func_call(func_name, lsb)
        }
        Expr::BitSelect { expr: inner, index } => {
            expr_has_func_call(func_name, inner) || expr_has_func_call(func_name, index)
        }
        Expr::PartSelect {
            expr: inner,
            base,
            width,
        } => {
            expr_has_func_call(func_name, inner)
                || expr_has_func_call(func_name, base)
                || expr_has_func_call(func_name, width)
        }
        _ => false,
    }
}

pub(crate) fn func_port_width(func: &FunctionDecl, port_name: Symbol) -> usize {
    if let Some(port) = func.ports.iter().find(|p| p.name == port_name) {
        if let Some(r) = &port.range {
            return r.width();
        }
    }
    for decl in &func.decls {
        for var in &decl.names {
            if var.name == port_name {
                if let Some(r) = &var.range {
                    return r.width();
                }
                return 1;
            }
        }
    }
    // Port has no range and no matching decl — likely user-defined type (struct/enum)
    // Use a safe default width (64) to avoid width mismatch issues during simulation
    let known_builtin = func
        .ports
        .iter()
        .any(|p| p.name == port_name && p.range.is_none());
    if known_builtin {
        1
    } else {
        64
    }
}

pub(crate) fn func_return_width(func: &FunctionDecl) -> usize {
    if let Some(er) = &func.range {
        if let (Ok(msb), Ok(lsb)) = (
            super::types::const_eval_simple(&er.msb),
            super::types::const_eval_simple(&er.lsb),
        ) {
            let msb = msb as usize;
            let lsb = lsb as usize;
            return if msb >= lsb {
                msb - lsb + 1
            } else {
                lsb - msb + 1
            };
        }
    }
    match &func.return_type {
        Some(inner) => match inner.as_ref() {
            DataType::Void => 0,
            DataType::Byte => 8,
            DataType::Shortint => 16,
            DataType::Int | DataType::Integer => 32,
            DataType::Longint => 64,
            DataType::Time => 64,
            DataType::Signed(s) => match s.as_ref() {
                DataType::Bit => 1,
                DataType::Logic => 1,
                DataType::Byte => 8,
                DataType::Shortint => 16,
                DataType::Int | DataType::Integer => 32,
                DataType::Longint => 64,
                DataType::Time => 64,
                _ => 1,
            },
            _ => 1,
        },
        _ => 1,
    }
}

pub(crate) fn rename_in_stmt(stmt: &Stmt, rename_map: &HashMap<Symbol, Symbol>) -> Stmt {
    match stmt.clone() {
        Stmt::Block { stmts } => Stmt::Block {
            stmts: stmts
                .iter()
                .map(|s| rename_in_stmt(s, rename_map))
                .collect(),
        },
        Stmt::NamedBlock { name, stmts, decls } => Stmt::NamedBlock {
            name,
            stmts: stmts
                .iter()
                .map(|s| rename_in_stmt(s, rename_map))
                .collect(),
            decls,
        },
        Stmt::IfElse {
            cond,
            true_branch,
            false_branch,
        } => Stmt::IfElse {
            cond: rename_in_expr(cond, rename_map),
            true_branch: Box::new(rename_in_stmt(&true_branch, rename_map)),
            false_branch: false_branch.map(|fb| Box::new(rename_in_stmt(&fb, rename_map))),
        },
        Stmt::Case {
            expr,
            items,
            default,
        } => Stmt::Case {
            expr: rename_in_expr(expr, rename_map),
            items: items
                .iter()
                .map(|item| super::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| rename_in_expr(l.clone(), rename_map))
                        .collect(),
                    stmt: Box::new(rename_in_stmt(&item.stmt, rename_map)),
                })
                .collect(),
            default: default.map(|d| Box::new(rename_in_stmt(&d, rename_map))),
        },
        Stmt::CaseX {
            expr,
            items,
            default,
        } => Stmt::CaseX {
            expr: rename_in_expr(expr, rename_map),
            items: items
                .iter()
                .map(|item| super::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| rename_in_expr(l.clone(), rename_map))
                        .collect(),
                    stmt: Box::new(rename_in_stmt(&item.stmt, rename_map)),
                })
                .collect(),
            default: default.map(|d| Box::new(rename_in_stmt(&d, rename_map))),
        },
        Stmt::CaseZ {
            expr,
            items,
            default,
        } => Stmt::CaseZ {
            expr: rename_in_expr(expr, rename_map),
            items: items
                .iter()
                .map(|item| super::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| rename_in_expr(l.clone(), rename_map))
                        .collect(),
                    stmt: Box::new(rename_in_stmt(&item.stmt, rename_map)),
                })
                .collect(),
            default: default.map(|d| Box::new(rename_in_stmt(&d, rename_map))),
        },
        Stmt::LoopForever { stmts } => Stmt::LoopForever {
            stmts: stmts
                .iter()
                .map(|s| rename_in_stmt(s, rename_map))
                .collect(),
        },
        Stmt::LoopWhile { cond, stmts } => Stmt::LoopWhile {
            cond: rename_in_expr(cond, rename_map),
            stmts: stmts
                .iter()
                .map(|s| rename_in_stmt(s, rename_map))
                .collect(),
        },
        Stmt::LoopFor {
            init,
            cond,
            step,
            stmts,
        } => Stmt::LoopFor {
            init: init.map(|i| Box::new(rename_in_stmt(&i, rename_map))),
            cond: cond.map(|c| rename_in_expr(c, rename_map)),
            step: step.map(|s| Box::new(rename_in_stmt(&s, rename_map))),
            stmts: stmts
                .iter()
                .map(|s| rename_in_stmt(s, rename_map))
                .collect(),
        },
        Stmt::Repeat { count, stmts } => Stmt::Repeat {
            count: rename_in_expr(count, rename_map),
            stmts: stmts
                .iter()
                .map(|s| rename_in_stmt(s, rename_map))
                .collect(),
        },
        Stmt::BlockingAssign { lhs, rhs, delay } => Stmt::BlockingAssign {
            lhs: rename_in_expr(lhs, rename_map),
            rhs: rename_in_expr(rhs, rename_map),
            delay,
        },
        Stmt::NonBlockingAssign { lhs, rhs, delay } => Stmt::NonBlockingAssign {
            lhs: rename_in_expr(lhs, rename_map),
            rhs: rename_in_expr(rhs, rename_map),
            delay,
        },
        Stmt::StmtAssign { lhs, rhs } => Stmt::StmtAssign {
            lhs: rename_in_expr(lhs, rename_map),
            rhs: rename_in_expr(rhs, rename_map),
        },
        Stmt::StmtCase {
            expr,
            items,
            default,
        } => Stmt::StmtCase {
            expr: rename_in_expr(expr, rename_map),
            items: items
                .iter()
                .map(|item| super::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| rename_in_expr(l.clone(), rename_map))
                        .collect(),
                    stmt: Box::new(rename_in_stmt(&item.stmt, rename_map)),
                })
                .collect(),
            default: default.map(|d| Box::new(rename_in_stmt(&d, rename_map))),
        },
        Stmt::SysCall { name, args } => Stmt::SysCall {
            name,
            args: args
                .into_iter()
                .map(|a| rename_in_expr(a, rename_map))
                .collect(),
        },
        Stmt::SysFinish => Stmt::SysFinish,
        Stmt::Delay { delay, stmt } => Stmt::Delay {
            delay: rename_in_expr(delay, rename_map),
            stmt: Box::new(rename_in_stmt(&stmt, rename_map)),
        },
        Stmt::Disable { name } => Stmt::Disable { name },
        Stmt::Force { lhs, rhs } => Stmt::Force {
            lhs: rename_in_expr(lhs, rename_map),
            rhs: rename_in_expr(rhs, rename_map),
        },
        Stmt::Release { expr } => Stmt::Release {
            expr: rename_in_expr(expr, rename_map),
        },
        Stmt::Deassign { expr } => Stmt::Deassign {
            expr: rename_in_expr(expr, rename_map),
        },
        Stmt::Wait { cond, stmt } => Stmt::Wait {
            cond: rename_in_expr(cond, rename_map),
            stmt: stmt.map(|s| Box::new(rename_in_stmt(&s, rename_map))),
        },
        Stmt::EventControl { events, stmt } => Stmt::EventControl {
            events: events.clone(),
            stmt: stmt.map(|s| Box::new(rename_in_stmt(&s, rename_map))),
        },
        Stmt::EventTrigger { name } => Stmt::EventTrigger { name },
        Stmt::Expr { expr } => Stmt::Expr {
            expr: rename_in_expr(expr, rename_map),
        },
        Stmt::Null => Stmt::Null,
        Stmt::Return(expr) => {
            let renamed_expr = expr.map(|e| Box::new(rename_in_expr(*e, rename_map)));
            Stmt::Return(renamed_expr)
        }
        Stmt::ForeachLoop {
            array_var,
            index_vars,
            stmts,
        } => Stmt::ForeachLoop {
            array_var,
            index_vars,
            stmts: stmts
                .into_iter()
                .map(|s| rename_in_stmt(&s, rename_map))
                .collect(),
        },
        Stmt::Break => Stmt::Break,
        Stmt::Continue => Stmt::Continue,
        Stmt::DoWhile { cond, stmts } => Stmt::DoWhile {
            cond: rename_in_expr(cond, rename_map),
            stmts: stmts
                .into_iter()
                .map(|s| rename_in_stmt(&s, rename_map))
                .collect(),
        },
        Stmt::UniqueCase {
            expr,
            items,
            default,
        } => Stmt::UniqueCase {
            expr: rename_in_expr(expr, rename_map),
            items: items
                .iter()
                .map(|item| super::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| rename_in_expr(l.clone(), rename_map))
                        .collect(),
                    stmt: Box::new(rename_in_stmt(&item.stmt, rename_map)),
                })
                .collect(),
            default: default.map(|d| Box::new(rename_in_stmt(&d, rename_map))),
        },
        Stmt::PriorityCase {
            expr,
            items,
            default,
        } => Stmt::PriorityCase {
            expr: rename_in_expr(expr, rename_map),
            items: items
                .iter()
                .map(|item| super::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| rename_in_expr(l.clone(), rename_map))
                        .collect(),
                    stmt: Box::new(rename_in_stmt(&item.stmt, rename_map)),
                })
                .collect(),
            default: default.map(|d| Box::new(rename_in_stmt(&d, rename_map))),
        },
        Stmt::CaseInside {
            expr,
            items,
            default,
        } => Stmt::CaseInside {
            expr: rename_in_expr(expr, rename_map),
            items: items
                .iter()
                .map(|item| super::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| rename_in_expr(l.clone(), rename_map))
                        .collect(),
                    stmt: Box::new(rename_in_stmt(&item.stmt, rename_map)),
                })
                .collect(),
            default: default.map(|d| Box::new(rename_in_stmt(&d, rename_map))),
        },
        Stmt::Assert {
            cond,
            pass_stmt,
            fail_stmt,
            ..
        } => Stmt::Assert {
            cond: rename_in_expr(cond, rename_map),
            pass_stmt: pass_stmt.map(|s| Box::new(rename_in_stmt(&s, rename_map))),
            fail_stmt: fail_stmt.map(|s| Box::new(rename_in_stmt(&s, rename_map))),
            clock_event: None,
            disable_iff: None,
        },
        Stmt::Assume {
            cond,
            pass_stmt,
            fail_stmt,
            ..
        } => Stmt::Assume {
            cond: rename_in_expr(cond, rename_map),
            pass_stmt: pass_stmt.map(|s| Box::new(rename_in_stmt(&s, rename_map))),
            fail_stmt: fail_stmt.map(|s| Box::new(rename_in_stmt(&s, rename_map))),
            clock_event: None,
            disable_iff: None,
        },
        Stmt::Cover {
            cond, pass_stmt, ..
        } => Stmt::Cover {
            cond: rename_in_expr(cond, rename_map),
            pass_stmt: pass_stmt.map(|s| Box::new(rename_in_stmt(&s, rename_map))),
            clock_event: None,
            disable_iff: None,
        },
        Stmt::Expect {
            cond,
            pass_stmt,
            fail_stmt,
        } => Stmt::Expect {
            cond: rename_in_expr(cond, rename_map),
            pass_stmt: pass_stmt.map(|s| Box::new(rename_in_stmt(&s, rename_map))),
            fail_stmt: fail_stmt.map(|s| Box::new(rename_in_stmt(&s, rename_map))),
        },
        Stmt::WaitOrder { events, fail_stmt } => Stmt::WaitOrder {
            events,
            fail_stmt: fail_stmt.map(|s| Box::new(rename_in_stmt(&s, rename_map))),
        },
        Stmt::UniqueIf {
            cond,
            true_branch,
            false_branch,
        } => Stmt::UniqueIf {
            cond: rename_in_expr(cond, rename_map),
            true_branch: Box::new(rename_in_stmt(&true_branch, rename_map)),
            false_branch: false_branch.map(|s| Box::new(rename_in_stmt(&s, rename_map))),
        },
        Stmt::PriorityIf {
            cond,
            true_branch,
            false_branch,
        } => Stmt::PriorityIf {
            cond: rename_in_expr(cond, rename_map),
            true_branch: Box::new(rename_in_stmt(&true_branch, rename_map)),
            false_branch: false_branch.map(|s| Box::new(rename_in_stmt(&s, rename_map))),
        },
        Stmt::Fork {
            processes,
            join_type,
        } => Stmt::Fork {
            processes: processes
                .into_iter()
                .map(|s| rename_in_stmt(&s, rename_map))
                .collect(),
            join_type,
        },
        Stmt::RandCase { items } => Stmt::RandCase {
            items: items
                .into_iter()
                .map(|rc| crate::ast::stmt::RandCaseItem {
                    weight: rc.weight,
                    stmt: Box::new(rename_in_stmt(&rc.stmt, rename_map)),
                })
                .collect(),
        },
        Stmt::RandSequence { productions } => Stmt::RandSequence {
            productions: productions
                .into_iter()
                .map(|p| crate::ast::stmt::RandSeqProduction {
                    name: p.name,
                    items: p
                        .items
                        .into_iter()
                        .map(|item| crate::ast::stmt::RandSeqItem {
                            value: Box::new(rename_in_stmt(&item.value, rename_map)),
                            weight: item.weight,
                        })
                        .collect(),
                })
                .collect(),
        },
    }
}

pub(crate) fn rename_func_decls_in_stmt(stmt: Stmt, rename_map: &HashMap<Symbol, Symbol>) -> Stmt {
    match stmt {
        Stmt::NamedBlock { name, stmts, decls } => {
            let new_decls: Vec<Decl> = decls
                .into_iter()
                .map(|mut d| {
                    for var in &mut d.names {
                        if let Some(new_name) = rename_map.get(&var.name) {
                            var.name = new_name.clone();
                        }
                    }
                    d
                })
                .collect();
            let new_stmts = stmts
                .into_iter()
                .map(|s| rename_func_decls_in_stmt(s, rename_map))
                .collect();
            Stmt::NamedBlock {
                name,
                stmts: new_stmts,
                decls: new_decls,
            }
        }
        Stmt::Block { stmts } => Stmt::Block {
            stmts: stmts
                .into_iter()
                .map(|s| rename_func_decls_in_stmt(s, rename_map))
                .collect(),
        },
        other => other,
    }
}

pub(crate) fn rename_in_expr(expr: Expr, rename_map: &HashMap<Symbol, Symbol>) -> Expr {
    match expr {
        Expr::Ident(name) => rename_map
            .get(&name)
            .map_or(Expr::Ident(name), |n| Expr::Ident(n.clone())),
        Expr::BinaryOp { op, lhs, rhs } => Expr::BinaryOp {
            op,
            lhs: Box::new(rename_in_expr(*lhs, rename_map)),
            rhs: Box::new(rename_in_expr(*rhs, rename_map)),
        },
        Expr::UnaryOp { op, expr: inner } => Expr::UnaryOp {
            op,
            expr: Box::new(rename_in_expr(*inner, rename_map)),
        },
        Expr::TernaryOp {
            cond,
            true_expr,
            false_expr,
        } => Expr::TernaryOp {
            cond: Box::new(rename_in_expr(*cond, rename_map)),
            true_expr: Box::new(rename_in_expr(*true_expr, rename_map)),
            false_expr: Box::new(rename_in_expr(*false_expr, rename_map)),
        },
        Expr::Concat(exprs) => Expr::Concat(
            exprs
                .into_iter()
                .map(|e| rename_in_expr(e, rename_map))
                .collect(),
        ),
        Expr::Replicate { count, expr: inner } => Expr::Replicate {
            count: Box::new(rename_in_expr(*count, rename_map)),
            expr: Box::new(rename_in_expr(*inner, rename_map)),
        },
        Expr::Paren(inner) => Expr::Paren(Box::new(rename_in_expr(*inner, rename_map))),
        Expr::RangeSelect {
            expr: inner,
            msb,
            lsb,
        } => Expr::RangeSelect {
            expr: Box::new(rename_in_expr(*inner, rename_map)),
            msb: Box::new(rename_in_expr(*msb, rename_map)),
            lsb: Box::new(rename_in_expr(*lsb, rename_map)),
        },
        Expr::BitSelect { expr: inner, index } => Expr::BitSelect {
            expr: Box::new(rename_in_expr(*inner, rename_map)),
            index: Box::new(rename_in_expr(*index, rename_map)),
        },
        Expr::PartSelect {
            expr: inner,
            base,
            width,
        } => Expr::PartSelect {
            expr: Box::new(rename_in_expr(*inner, rename_map)),
            base: Box::new(rename_in_expr(*base, rename_map)),
            width: Box::new(rename_in_expr(*width, rename_map)),
        },
        Expr::FuncCall { name, args } => Expr::FuncCall {
            name: rename_map.get(&name).cloned().unwrap_or(name),
            args: args
                .into_iter()
                .map(|a| rename_in_expr(a, rename_map))
                .collect(),
        },
        other => other,
    }
}
