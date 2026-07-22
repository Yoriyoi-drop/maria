use std::collections::{HashMap, HashSet};

use super::expr::Expr;
use super::expr::Value;
use super::stmt::Stmt;
use super::types::{DataType, Decl, FunctionDecl, Module, ModuleItem};

/// Detect direct recursive functions (functions that call themselves directly).
fn detect_recursive_functions(funcs: &HashMap<String, FunctionDecl>) -> HashSet<String> {
    let mut recursive = HashSet::new();
    // First pass: detect direct recursion
    for (name, func) in funcs {
        if stmt_has_func_call(name, &func.stmts) {
            recursive.insert(name.clone());
        }
    }
    recursive
}

/// Check if a function body contains calls to a specific function.
fn stmt_has_func_call(func_name: &str, stmts: &[Stmt]) -> bool {
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
fn expr_has_func_call(func_name: &str, expr: &Expr) -> bool {
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

fn func_port_width(func: &FunctionDecl, port_name: &str) -> usize {
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

fn func_return_width(func: &FunctionDecl) -> usize {
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

pub fn inline_func_calls_in_module(module: &mut Module) -> Result<Vec<(String, usize)>, String> {
    let funcs: HashMap<String, FunctionDecl> = module
        .items
        .iter()
        .filter_map(|item| {
            if let ModuleItem::Func(f) = item {
                Some((f.name.clone(), f.clone()))
            } else {
                None
            }
        })
        .collect();

    if funcs.is_empty() {
        return Ok(Vec::new());
    }

    // Detect recursive functions — these must NOT be inlined; they'll be called at runtime
    let recursive_funcs = detect_recursive_functions(&funcs);

    let mut counter = 0usize;
    let prefix = &module.name;
    let mut temp_signals: Vec<(String, usize)> = Vec::new();

    let old_items = std::mem::replace(&mut module.items, Vec::new());
    let mut new_items: Vec<ModuleItem> = Vec::new();
    for item in old_items {
        match item {
            ModuleItem::Always(mut always) => {
                always.stmts = always
                    .stmts
                    .drain(..)
                    .map(|s| {
                        inline_funcs_in_stmt(
                            s,
                            &funcs,
                            prefix,
                            &mut counter,
                            &mut temp_signals,
                            &recursive_funcs,
                        )
                    })
                    .collect();
                new_items.push(ModuleItem::Always(always));
            }
            ModuleItem::Initial(mut initial) => {
                initial.stmts = initial
                    .stmts
                    .drain(..)
                    .map(|s| {
                        inline_funcs_in_stmt(
                            s,
                            &funcs,
                            prefix,
                            &mut counter,
                            &mut temp_signals,
                            &recursive_funcs,
                        )
                    })
                    .collect();
                new_items.push(ModuleItem::Initial(initial));
            }
            ModuleItem::Final(mut final_block) => {
                final_block.stmts = final_block
                    .stmts
                    .drain(..)
                    .map(|s| {
                        inline_funcs_in_stmt(
                            s,
                            &funcs,
                            prefix,
                            &mut counter,
                            &mut temp_signals,
                            &recursive_funcs,
                        )
                    })
                    .collect();
                new_items.push(ModuleItem::Final(final_block));
            }
            ModuleItem::Assign(assign) => {
                let mut preamble = Vec::new();
                let old_rhs = assign.rhs;
                let new_rhs = replace_func_calls_in_expr(
                    old_rhs,
                    &funcs,
                    prefix,
                    &mut counter,
                    &mut preamble,
                    &mut temp_signals,
                    &recursive_funcs,
                );
                if preamble.is_empty() {
                    new_items.push(ModuleItem::Assign(super::types::ContinuousAssign {
                        lhs: assign.lhs,
                        rhs: new_rhs,
                        delay: assign.delay,
                    }));
                } else {
                    preamble.push(Stmt::BlockingAssign {
                        lhs: assign.lhs,
                        rhs: new_rhs,
                        delay: None,
                    });
                    let wc = super::stmt::SensitivityList {
                        events: vec![super::stmt::SensitivityEvent::Wildcard],
                    };
                    new_items.push(ModuleItem::Always(super::stmt::AlwaysBlock {
                        kind: super::stmt::AlwaysKind::AlwaysComb,
                        sensitivity: Some(wc),
                        stmts: preamble,
                    }));
                }
            }
            ModuleItem::Func(f) => {
                if recursive_funcs.contains(&f.name) {
                    // Keep recursive function declarations in module items for runtime evaluation
                    new_items.push(ModuleItem::Func(f));
                }
                // Non-recursive functions are removed (they've been inlined)
            }
            other => {
                new_items.push(other);
            }
        }
    }
    module.items = new_items;

    // Remove non-recursive function declarations from module items
    module.items.retain(|item| {
        if let ModuleItem::Func(f) = item {
            recursive_funcs.contains(&f.name)
        } else {
            true
        }
    });

    Ok(temp_signals)
}

fn inline_funcs_in_stmt(
    stmt: Stmt,
    funcs: &HashMap<String, FunctionDecl>,
    prefix: &str,
    counter: &mut usize,
    temp_signals: &mut Vec<(String, usize)>,
    recursive_funcs: &HashSet<String>,
) -> Stmt {
    match stmt {
        Stmt::Block { stmts } => {
            let new_stmts = stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect();
            Stmt::Block { stmts: new_stmts }
        }
        Stmt::NamedBlock { name, stmts, decls } => {
            let new_stmts = stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect();
            Stmt::NamedBlock {
                name,
                stmts: new_stmts,
                decls,
            }
        }
        Stmt::IfElse {
            cond,
            true_branch,
            false_branch,
        } => {
            let mut preamble = Vec::new();
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_true = inline_funcs_in_stmt(
                *true_branch,
                funcs,
                prefix,
                counter,
                temp_signals,
                recursive_funcs,
            );
            let new_false = false_branch.map(|fb| {
                inline_funcs_in_stmt(*fb, funcs, prefix, counter, temp_signals, recursive_funcs)
            });
            let main = Stmt::IfElse {
                cond: new_cond,
                true_branch: Box::new(new_true),
                false_branch: new_false.map(Box::new),
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::Case {
            expr,
            items,
            default,
        } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_items = items
                .into_iter()
                .map(|item| {
                    let new_labels = item
                        .labels
                        .into_iter()
                        .map(|l| {
                            replace_func_calls_in_expr(
                                l,
                                funcs,
                                prefix,
                                counter,
                                &mut Vec::new(),
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();
                    let new_stmt = inline_funcs_in_stmt(
                        *item.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    super::stmt::CaseItem {
                        labels: new_labels,
                        stmt: Box::new(new_stmt),
                    }
                })
                .collect();
            let new_default = default.map(|d| {
                Box::new(inline_funcs_in_stmt(
                    *d,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let main = Stmt::Case {
                expr: new_expr,
                items: new_items,
                default: new_default,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::CaseX {
            expr,
            items,
            default,
        } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_items = items
                .into_iter()
                .map(|item| {
                    let new_labels = item
                        .labels
                        .into_iter()
                        .map(|l| {
                            replace_func_calls_in_expr(
                                l,
                                funcs,
                                prefix,
                                counter,
                                &mut Vec::new(),
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();
                    let new_stmt = inline_funcs_in_stmt(
                        *item.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    super::stmt::CaseItem {
                        labels: new_labels,
                        stmt: Box::new(new_stmt),
                    }
                })
                .collect();
            let new_default = default.map(|d| {
                Box::new(inline_funcs_in_stmt(
                    *d,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let main = Stmt::CaseX {
                expr: new_expr,
                items: new_items,
                default: new_default,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::CaseZ {
            expr,
            items,
            default,
        } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_items = items
                .into_iter()
                .map(|item| {
                    let new_labels = item
                        .labels
                        .into_iter()
                        .map(|l| {
                            replace_func_calls_in_expr(
                                l,
                                funcs,
                                prefix,
                                counter,
                                &mut Vec::new(),
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();
                    let new_stmt = inline_funcs_in_stmt(
                        *item.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    super::stmt::CaseItem {
                        labels: new_labels,
                        stmt: Box::new(new_stmt),
                    }
                })
                .collect();
            let new_default = default.map(|d| {
                Box::new(inline_funcs_in_stmt(
                    *d,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let main = Stmt::CaseZ {
                expr: new_expr,
                items: new_items,
                default: new_default,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::LoopForever { stmts } => Stmt::LoopForever {
            stmts: stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect(),
        },
        Stmt::LoopWhile { cond, stmts } => {
            let mut preamble = Vec::new();
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_stmts = stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect();
            let main = Stmt::LoopWhile {
                cond: new_cond,
                stmts: new_stmts,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::LoopFor {
            init,
            cond,
            step,
            stmts,
        } => {
            let new_init = init.map(|i| {
                Box::new(inline_funcs_in_stmt(
                    *i,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let mut preamble = Vec::new();
            let new_cond = cond.map(|c| {
                replace_func_calls_in_expr(
                    c,
                    funcs,
                    prefix,
                    counter,
                    &mut preamble,
                    temp_signals,
                    recursive_funcs,
                )
            });
            let new_step = step.map(|s| {
                Box::new(inline_funcs_in_stmt(
                    *s,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let new_stmts = stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect();
            let main = Stmt::LoopFor {
                init: new_init,
                cond: new_cond,
                step: new_step,
                stmts: new_stmts,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::Repeat { count, stmts } => {
            let mut preamble = Vec::new();
            let new_count = replace_func_calls_in_expr(
                count,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_stmts = stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect();
            let main = Stmt::Repeat {
                count: new_count,
                stmts: new_stmts,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::BlockingAssign { lhs, rhs, delay } => {
            let mut preamble = Vec::new();
            let new_rhs = replace_func_calls_in_expr(
                rhs,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let main = Stmt::BlockingAssign {
                lhs,
                rhs: new_rhs,
                delay,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::NonBlockingAssign { lhs, rhs, delay } => {
            let mut preamble = Vec::new();
            let new_rhs = replace_func_calls_in_expr(
                rhs,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let main = Stmt::NonBlockingAssign {
                lhs,
                rhs: new_rhs,
                delay,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::StmtAssign { lhs, rhs } => {
            // Check if LHS is a function/task call (task statement like `my_task(a, b)`)
            if let Expr::FuncCall { name, args } = &lhs {
                if let Some(func) = funcs.get(name) {
                    let c = *counter;
                    *counter += 1;

                    let mut preamble = Vec::new();

                    let new_args: Vec<Expr> = args
                        .iter()
                        .map(|a| {
                            replace_func_calls_in_expr(
                                a.clone(),
                                funcs,
                                prefix,
                                counter,
                                &mut preamble,
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();

                    let mut rename_map: HashMap<String, String> = HashMap::new();

                    for (i, arg) in new_args.into_iter().enumerate() {
                        let port = func.ports.get(i).cloned().unwrap_or_else(|| {
                            super::types::FunctionPort {
                                name: format!("_arg{}", i),
                                range: None,
                                expr_range: None,
                                direction: None,
                            }
                        });
                        let temp_arg_name =
                            format!("__func_{}_{}_{}_{}", prefix, name, c, port.name);
                        let port_width = func_port_width(func, &port.name);
                        temp_signals.push((temp_arg_name.clone(), port_width));
                        rename_map.insert(port.name.clone(), temp_arg_name.clone());
                        preamble.push(Stmt::BlockingAssign {
                            lhs: Expr::Ident(temp_arg_name),
                            rhs: arg,
                            delay: None,
                        });
                    }

                    // Add internal declarations (non-port variables)
                    for decl in &func.decls {
                        for var in &decl.names {
                            if rename_map.contains_key(&var.name) {
                                continue;
                            }
                            let new_name = format!("__func_{}_{}_{}_{}", prefix, name, c, var.name);
                            let dtype_width = match &decl.dtype {
                                super::types::DataType::Bit | super::types::DataType::Logic => 1,
                                super::types::DataType::Byte => 8,
                                super::types::DataType::Shortint => 16,
                                super::types::DataType::Int | super::types::DataType::Integer => 32,
                                super::types::DataType::Longint => 64,
                                super::types::DataType::Time => 64,
                                super::types::DataType::Signed(inner) => match inner.as_ref() {
                                    super::types::DataType::Bit | super::types::DataType::Logic => {
                                        1
                                    }
                                    super::types::DataType::Byte => 8,
                                    super::types::DataType::Shortint => 16,
                                    super::types::DataType::Int
                                    | super::types::DataType::Integer => 32,
                                    super::types::DataType::Longint => 64,
                                    super::types::DataType::Time => 64,
                                    _ => 32,
                                },
                                _ => 1,
                            };
                            let decl_width = match &decl.kind {
                                super::types::DeclKind::Wire
                                | super::types::DeclKind::Reg
                                | super::types::DeclKind::Logic => 1,
                                super::types::DeclKind::Int | super::types::DeclKind::Integer => 32,
                                _ => 1,
                            };
                            let width = if let Some(r) = &var.range {
                                r.width()
                            } else {
                                dtype_width.max(decl_width)
                            };
                            temp_signals.push((new_name.clone(), width));
                            rename_map.insert(var.name.clone(), new_name);
                        }
                    }

                    // Insert renamed body statements
                    for func_stmt in &func.stmts {
                        let mut renamed = rename_in_stmt(func_stmt, &rename_map);
                        renamed = rename_func_decls_in_stmt(renamed, &rename_map);
                        preamble.push(renamed);
                    }

                    // Also process the RHS normally (may contain function calls)
                    let preamble2 = &mut Vec::new();
                    let _new_rhs = replace_func_calls_in_expr(
                        rhs,
                        funcs,
                        prefix,
                        counter,
                        preamble2,
                        temp_signals,
                        recursive_funcs,
                    );
                    preamble.extend(preamble2.drain(..));

                    if preamble.len() == 1 {
                        preamble.into_iter().next().unwrap()
                    } else {
                        Stmt::Block { stmts: preamble }
                    }
                } else {
                    let mut preamble = Vec::new();
                    let new_rhs = replace_func_calls_in_expr(
                        rhs,
                        funcs,
                        prefix,
                        counter,
                        &mut preamble,
                        temp_signals,
                        recursive_funcs,
                    );
                    let main = Stmt::StmtAssign { lhs, rhs: new_rhs };
                    if preamble.is_empty() {
                        main
                    } else {
                        preamble.push(main);
                        Stmt::Block { stmts: preamble }
                    }
                }
            } else {
                let mut preamble = Vec::new();
                let new_rhs = replace_func_calls_in_expr(
                    rhs,
                    funcs,
                    prefix,
                    counter,
                    &mut preamble,
                    temp_signals,
                    recursive_funcs,
                );
                let main = Stmt::StmtAssign { lhs, rhs: new_rhs };
                if preamble.is_empty() {
                    main
                } else {
                    preamble.push(main);
                    Stmt::Block { stmts: preamble }
                }
            }
        }
        Stmt::StmtCase {
            expr,
            items,
            default,
        } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_items = items
                .into_iter()
                .map(|item| {
                    let new_labels = item
                        .labels
                        .into_iter()
                        .map(|l| {
                            replace_func_calls_in_expr(
                                l,
                                funcs,
                                prefix,
                                counter,
                                &mut Vec::new(),
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();
                    let new_stmt = inline_funcs_in_stmt(
                        *item.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    super::stmt::CaseItem {
                        labels: new_labels,
                        stmt: Box::new(new_stmt),
                    }
                })
                .collect();
            let new_default = default.map(|d| {
                Box::new(inline_funcs_in_stmt(
                    *d,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let main = Stmt::StmtCase {
                expr: new_expr,
                items: new_items,
                default: new_default,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::SysCall { name, args } => {
            let mut preamble = Vec::new();
            let new_args = args
                .into_iter()
                .map(|a| {
                    replace_func_calls_in_expr(
                        a,
                        funcs,
                        prefix,
                        counter,
                        &mut preamble,
                        temp_signals,
                        recursive_funcs,
                    )
                })
                .collect();
            let main = Stmt::SysCall {
                name,
                args: new_args,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::SysFinish => Stmt::SysFinish,
        Stmt::Delay { delay, stmt } => {
            let mut preamble = Vec::new();
            let new_delay = replace_func_calls_in_expr(
                delay,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_stmt =
                inline_funcs_in_stmt(*stmt, funcs, prefix, counter, temp_signals, recursive_funcs);
            let main = Stmt::Delay {
                delay: new_delay,
                stmt: Box::new(new_stmt),
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::Disable { name } => Stmt::Disable { name },
        Stmt::Force { lhs, rhs } => {
            let mut preamble = Vec::new();
            let new_rhs = replace_func_calls_in_expr(
                rhs,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let main = Stmt::Force { lhs, rhs: new_rhs };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::Release { expr } => Stmt::Release { expr },
        Stmt::Deassign { expr } => Stmt::Deassign { expr },
        Stmt::Wait { cond, stmt } => {
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut vec![],
                temp_signals,
                recursive_funcs,
            );
            Stmt::Wait {
                cond: new_cond,
                stmt: stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
            }
        }
        Stmt::EventControl { events, stmt } => Stmt::EventControl {
            events: events.clone(),
            stmt: stmt.map(|s| {
                Box::new(inline_funcs_in_stmt(
                    *s,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            }),
        },
        Stmt::EventTrigger { name } => Stmt::EventTrigger { name },
        Stmt::Expr { expr } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let main = Stmt::Expr { expr: new_expr };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::Null => Stmt::Null,
        Stmt::Return(expr) => Stmt::Return(expr),
        Stmt::ForeachLoop {
            array_var,
            index_vars,
            stmts,
        } => {
            let stmts = stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect();
            Stmt::ForeachLoop {
                array_var,
                index_vars,
                stmts,
            }
        }
        Stmt::Break => Stmt::Break,
        Stmt::Continue => Stmt::Continue,
        Stmt::DoWhile { cond, stmts } => {
            let new_stmts = stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect();
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut Vec::new(),
                temp_signals,
                recursive_funcs,
            );
            Stmt::DoWhile {
                cond: new_cond,
                stmts: new_stmts,
            }
        }
        Stmt::Fork {
            processes,
            join_type,
        } => Stmt::Fork {
            processes: processes
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect(),
            join_type,
        },
        Stmt::RandCase { items } => Stmt::RandCase {
            items: items
                .into_iter()
                .map(|rc| crate::ast::stmt::RandCaseItem {
                    weight: rc.weight,
                    stmt: Box::new(inline_funcs_in_stmt(
                        *rc.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    )),
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
                            value: Box::new(inline_funcs_in_stmt(
                                *item.value,
                                funcs,
                                prefix,
                                counter,
                                temp_signals,
                                recursive_funcs,
                            )),
                            weight: item.weight,
                        })
                        .collect(),
                })
                .collect(),
        },
        Stmt::UniqueCase {
            expr,
            items,
            default,
        } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_items = items
                .into_iter()
                .map(|item| {
                    let new_labels = item
                        .labels
                        .into_iter()
                        .map(|l| {
                            replace_func_calls_in_expr(
                                l,
                                funcs,
                                prefix,
                                counter,
                                &mut Vec::new(),
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();
                    let new_stmt = inline_funcs_in_stmt(
                        *item.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    super::stmt::CaseItem {
                        labels: new_labels,
                        stmt: Box::new(new_stmt),
                    }
                })
                .collect();
            let new_default = default.map(|d| {
                Box::new(inline_funcs_in_stmt(
                    *d,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let main = Stmt::UniqueCase {
                expr: new_expr,
                items: new_items,
                default: new_default,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::PriorityCase {
            expr,
            items,
            default,
        } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_items = items
                .into_iter()
                .map(|item| {
                    let new_labels = item
                        .labels
                        .into_iter()
                        .map(|l| {
                            replace_func_calls_in_expr(
                                l,
                                funcs,
                                prefix,
                                counter,
                                &mut Vec::new(),
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();
                    let new_stmt = inline_funcs_in_stmt(
                        *item.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    super::stmt::CaseItem {
                        labels: new_labels,
                        stmt: Box::new(new_stmt),
                    }
                })
                .collect();
            let new_default = default.map(|d| {
                Box::new(inline_funcs_in_stmt(
                    *d,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let main = Stmt::PriorityCase {
                expr: new_expr,
                items: new_items,
                default: new_default,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::CaseInside {
            expr,
            items,
            default,
        } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_items = items
                .into_iter()
                .map(|item| {
                    let new_labels = item
                        .labels
                        .into_iter()
                        .map(|l| {
                            replace_func_calls_in_expr(
                                l,
                                funcs,
                                prefix,
                                counter,
                                &mut Vec::new(),
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();
                    let new_stmt = inline_funcs_in_stmt(
                        *item.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    super::stmt::CaseItem {
                        labels: new_labels,
                        stmt: Box::new(new_stmt),
                    }
                })
                .collect();
            let new_default = default.map(|d| {
                Box::new(inline_funcs_in_stmt(
                    *d,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let main = Stmt::CaseInside {
                expr: new_expr,
                items: new_items,
                default: new_default,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::Assert {
            cond,
            pass_stmt,
            fail_stmt,
            clock_event: _ce,
            disable_iff: _di,
        } => {
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut vec![],
                temp_signals,
                recursive_funcs,
            );
            Stmt::Assert {
                cond: new_cond,
                pass_stmt: pass_stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
                fail_stmt: fail_stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
                clock_event: None,
                disable_iff: None,
            }
        }
        Stmt::Assume {
            cond,
            pass_stmt,
            fail_stmt,
            clock_event: _ce,
            disable_iff: _di,
        } => {
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut vec![],
                temp_signals,
                recursive_funcs,
            );
            Stmt::Assume {
                cond: new_cond,
                pass_stmt: pass_stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
                fail_stmt: fail_stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
                clock_event: None,
                disable_iff: None,
            }
        }
        Stmt::Cover {
            cond,
            pass_stmt,
            clock_event: _ce,
            disable_iff: _di,
        } => {
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut vec![],
                temp_signals,
                recursive_funcs,
            );
            Stmt::Cover {
                cond: new_cond,
                pass_stmt: pass_stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
                clock_event: None,
                disable_iff: None,
            }
        }
        Stmt::Expect {
            cond,
            pass_stmt,
            fail_stmt,
        } => {
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut vec![],
                temp_signals,
                recursive_funcs,
            );
            Stmt::Expect {
                cond: new_cond,
                pass_stmt: pass_stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
                fail_stmt: fail_stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
            }
        }
        Stmt::WaitOrder { events, fail_stmt } => Stmt::WaitOrder {
            events,
            fail_stmt: fail_stmt.map(|s| {
                Box::new(inline_funcs_in_stmt(
                    *s,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            }),
        },
        Stmt::UniqueIf {
            cond,
            true_branch,
            false_branch,
        } => {
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut vec![],
                temp_signals,
                recursive_funcs,
            );
            Stmt::UniqueIf {
                cond: new_cond,
                true_branch: Box::new(inline_funcs_in_stmt(
                    *true_branch,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                )),
                false_branch: false_branch.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
            }
        }
        Stmt::PriorityIf {
            cond,
            true_branch,
            false_branch,
        } => {
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut vec![],
                temp_signals,
                recursive_funcs,
            );
            Stmt::PriorityIf {
                cond: new_cond,
                true_branch: Box::new(inline_funcs_in_stmt(
                    *true_branch,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                )),
                false_branch: false_branch.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
            }
        }
    }
}

fn replace_func_calls_in_expr(
    expr: Expr,
    funcs: &HashMap<String, FunctionDecl>,
    prefix: &str,
    counter: &mut usize,
    preamble: &mut Vec<Stmt>,
    temp_signals: &mut Vec<(String, usize)>,
    recursive_funcs: &HashSet<String>,
) -> Expr {
    match expr {
        Expr::FuncCall { name, args } => {
            if recursive_funcs.contains(&name) {
                // Recursive function call — keep as FuncCall for runtime evaluation
                let new_args: Vec<Expr> = args
                    .into_iter()
                    .map(|a| {
                        replace_func_calls_in_expr(
                            a,
                            funcs,
                            prefix,
                            counter,
                            preamble,
                            temp_signals,
                            recursive_funcs,
                        )
                    })
                    .collect();
                return Expr::FuncCall {
                    name,
                    args: new_args,
                };
            }
            if let Some(func) = funcs.get(&name) {
                let c = *counter;
                *counter += 1;

                let ret_width = func_return_width(func);
                let is_void = ret_width == 0;
                let ret_name = if !is_void {
                    let rn = format!("__func_{}_{}_{}_result", prefix, name, c);
                    temp_signals.push((rn.clone(), ret_width));
                    Some(rn)
                } else {
                    None
                };

                let new_args: Vec<Expr> = args
                    .into_iter()
                    .map(|a| {
                        replace_func_calls_in_expr(
                            a,
                            funcs,
                            prefix,
                            counter,
                            preamble,
                            temp_signals,
                            recursive_funcs,
                        )
                    })
                    .collect();

                let mut rename_map: HashMap<String, String> = HashMap::new();
                if let Some(ref rn) = ret_name {
                    rename_map.insert(name.clone(), rn.clone());
                }

                let orig_args: Vec<Expr> = new_args.clone();

                for (i, arg) in new_args.into_iter().enumerate() {
                    let port =
                        func.ports
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| super::types::FunctionPort {
                                name: format!("_arg{}", i),
                                range: None,
                                expr_range: None,
                                direction: None,
                            });
                    let temp_arg_name = format!("__func_{}_{}_{}_{}", prefix, name, c, port.name);
                    let port_width = func_port_width(func, &port.name);
                    temp_signals.push((temp_arg_name.clone(), port_width));
                    rename_map.insert(port.name.clone(), temp_arg_name.clone());
                    preamble.push(Stmt::BlockingAssign {
                        lhs: Expr::Ident(temp_arg_name),
                        rhs: arg,
                        delay: None,
                    });
                }

                // Add internal function declarations (non-port variables)
                for decl in &func.decls {
                    for var in &decl.names {
                        if rename_map.contains_key(&var.name) {
                            continue;
                        }
                        let new_name = format!("__func_{}_{}_{}_{}", prefix, name, c, var.name);
                        let dtype_width = match &decl.dtype {
                            super::types::DataType::Bit | super::types::DataType::Logic => 1,
                            super::types::DataType::Byte => 8,
                            super::types::DataType::Shortint => 16,
                            super::types::DataType::Int | super::types::DataType::Integer => 32,
                            super::types::DataType::Longint => 64,
                            super::types::DataType::Time => 64,
                            super::types::DataType::Signed(inner) => match inner.as_ref() {
                                super::types::DataType::Bit | super::types::DataType::Logic => 1,
                                super::types::DataType::Byte => 8,
                                super::types::DataType::Shortint => 16,
                                super::types::DataType::Int | super::types::DataType::Integer => 32,
                                super::types::DataType::Longint => 64,
                                super::types::DataType::Time => 64,
                                _ => 32,
                            },
                            _ => 1,
                        };
                        let decl_width = match &decl.kind {
                            super::types::DeclKind::Wire
                            | super::types::DeclKind::Reg
                            | super::types::DeclKind::Logic => 1,
                            super::types::DeclKind::Int | super::types::DeclKind::Integer => 32,
                            _ => 1,
                        };
                        let width = if let Some(r) = &var.range {
                            r.width()
                        } else {
                            dtype_width.max(decl_width)
                        };
                        temp_signals.push((new_name.clone(), width));
                        rename_map.insert(var.name.clone(), new_name);
                    }
                }

                for func_stmt in &func.stmts {
                    let mut renamed = rename_in_stmt(func_stmt, &rename_map);
                    // Convert Return(expr) to assignment to result signal
                    if let Some(ref rn) = ret_name {
                        if let Stmt::Return(Some(expr)) = &renamed {
                            renamed = Stmt::BlockingAssign {
                                lhs: Expr::Ident(rn.clone()),
                                rhs: *expr.clone(),
                                delay: None,
                            };
                        }
                    }
                    renamed = rename_func_decls_in_stmt(renamed, &rename_map);
                    preamble.push(renamed);
                }

                // Write-back output/inout port values to caller's signals
                for (i, orig_arg) in orig_args.into_iter().enumerate() {
                    let port =
                        func.ports
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| super::types::FunctionPort {
                                name: format!("_arg{}", i),
                                range: None,
                                expr_range: None,
                                direction: None,
                            });
                    let temp_arg_name = format!("__func_{}_{}_{}_{}", prefix, name, c, port.name);
                    if let Expr::Ident(_) = &orig_arg {
                        preamble.push(Stmt::BlockingAssign {
                            lhs: orig_arg,
                            rhs: Expr::Ident(temp_arg_name),
                            delay: None,
                        });
                    }
                }

                if let Some(rn) = ret_name {
                    Expr::Ident(rn)
                } else {
                    Expr::Value(Value::Decimal(0))
                }
            } else {
                Expr::FuncCall { name, args }
            }
        }
        Expr::BinaryOp { op, lhs, rhs } => Expr::BinaryOp {
            op,
            lhs: Box::new(replace_func_calls_in_expr(
                *lhs,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            rhs: Box::new(replace_func_calls_in_expr(
                *rhs,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
        },
        Expr::UnaryOp { op, expr: inner } => Expr::UnaryOp {
            op,
            expr: Box::new(replace_func_calls_in_expr(
                *inner,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
        },
        Expr::TernaryOp {
            cond,
            true_expr,
            false_expr,
        } => Expr::TernaryOp {
            cond: Box::new(replace_func_calls_in_expr(
                *cond,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            true_expr: Box::new(replace_func_calls_in_expr(
                *true_expr,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            false_expr: Box::new(replace_func_calls_in_expr(
                *false_expr,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
        },
        Expr::Concat(exprs) => Expr::Concat(
            exprs
                .into_iter()
                .map(|e| {
                    replace_func_calls_in_expr(
                        e,
                        funcs,
                        prefix,
                        counter,
                        preamble,
                        temp_signals,
                        recursive_funcs,
                    )
                })
                .collect(),
        ),
        Expr::Replicate { count, expr: inner } => Expr::Replicate {
            count,
            expr: Box::new(replace_func_calls_in_expr(
                *inner,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
        },
        Expr::Paren(inner) => Expr::Paren(Box::new(replace_func_calls_in_expr(
            *inner,
            funcs,
            prefix,
            counter,
            preamble,
            temp_signals,
            recursive_funcs,
        ))),
        Expr::RangeSelect {
            expr: inner,
            msb,
            lsb,
        } => Expr::RangeSelect {
            expr: Box::new(replace_func_calls_in_expr(
                *inner,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            msb: Box::new(replace_func_calls_in_expr(
                *msb,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            lsb: Box::new(replace_func_calls_in_expr(
                *lsb,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
        },
        Expr::BitSelect { expr: inner, index } => Expr::BitSelect {
            expr: Box::new(replace_func_calls_in_expr(
                *inner,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            index: Box::new(replace_func_calls_in_expr(
                *index,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
        },
        Expr::PartSelect {
            expr: inner,
            base,
            width,
        } => Expr::PartSelect {
            expr: Box::new(replace_func_calls_in_expr(
                *inner,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            base: Box::new(replace_func_calls_in_expr(
                *base,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            width: Box::new(replace_func_calls_in_expr(
                *width,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
        },
        other => other,
    }
}

fn rename_in_stmt(stmt: &Stmt, rename_map: &HashMap<String, String>) -> Stmt {
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

fn rename_func_decls_in_stmt(stmt: Stmt, rename_map: &HashMap<String, String>) -> Stmt {
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

fn rename_in_expr(expr: Expr, rename_map: &HashMap<String, String>) -> Expr {
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
