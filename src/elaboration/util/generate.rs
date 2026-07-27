//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan util.rs (SRP Refactoring).
//! Tanggung jawab: Generate block expansion & genvar substitution.
//!
//! Fungsi:
//!   - expand_all_generates()         — perluas semua generate block di module
//!   - extract_generate_step()        — ekstrak step value dari generate for
//!   - expand_generate_block()        — perluas satu generate block
//!   - substitute_genvar_in_module_item() — substitusi genvar di module item
//!   - substitute_genvar_in_generate_item() — substitusi genvar di generate item
//! ──────────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;

use crate::ast::types::const_eval_simple;

const MAX_GENERATED_ITEMS: usize = 1_000_000;
use crate::ast::types::const_eval_with_params;
use crate::ast::*;
use crate::intern::Symbol;

use super::loop_unroll::{substitute_loop_var_in_expr, substitute_loop_var_in_stmt};

/// Perluas SEMUA generate block di module dengan nilai parameter tertentu.
pub fn expand_all_generates(
    module: &mut Module,
    param_vals: &HashMap<Symbol, i64>,
) -> Result<(), String> {
    let mut i = 0;
    let mut total_items = 0usize;
    while i < module.items.len() {
        if let ModuleItem::Generate(gen) = &module.items[i] {
            let expanded = expand_generate_block(gen, param_vals)?;
            total_items += expanded.len();
            if total_items > MAX_GENERATED_ITEMS {
                return Err(format!(
                    "generate expansion exceeded limit ({} items)", MAX_GENERATED_ITEMS
                ));
            }
            for item in &expanded {
                if let ModuleItem::Decl(d) = item {
                    module.decls.push(d.clone());
                }
            }
            module.items.splice(i..=i, expanded);
        } else {
            i += 1;
        }
    }
    Ok(())
}

/// Ekstrak nilai step dari statement update generate for loop.
pub fn extract_generate_step(step: &Option<Stmt>, param_vals: &HashMap<Symbol, i64>) -> i64 {
    let Some(Stmt::BlockingAssign { rhs, .. }) = step else {
        return 1;
    };
    match rhs {
        Expr::BinaryOp {
            op: BinaryOp::Add,
            lhs: _,
            rhs,
        } => const_eval_with_params(rhs, param_vals).unwrap_or(1),
        Expr::BinaryOp {
            op: BinaryOp::Sub,
            lhs: _,
            rhs,
        } => -const_eval_with_params(rhs, param_vals).unwrap_or(1),
        _ => 1,
    }
}

/// Perluas SATU generate block dengan nilai parameter tertentu.
pub fn expand_generate_block(
    gen: &GenerateBlock,
    param_vals: &HashMap<Symbol, i64>,
) -> Result<Vec<ModuleItem>, String> {
    let mut result = Vec::new();
    for item in &gen.items {
        match item {
            GenerateItem::If {
                cond,
                true_items,
                false_items,
            } => {
                let eval_result = const_eval_with_params(cond, param_vals);
                match eval_result {
                    Ok(val) => {
                        let branch = if val != 0 { true_items } else { false_items };
                        for item in branch {
                            result.push(item.clone());
                        }
                    }
                    Err(_) => {
                        eprintln!("  ** WARNING: non-constant condition in generate if, taking true branch");
                        for item in true_items {
                            result.push(item.clone());
                        }
                    }
                }
            }
            GenerateItem::For {
                var,
                init,
                cond,
                step,
                body_items,
            } => {
                let start_val: i64 = match init {
                    Some(Stmt::BlockingAssign { rhs, .. }) => {
                        const_eval_with_params(rhs, param_vals)?
                    }
                    _ => 0,
                };
                let limit: i64 = match cond {
                    Some(Expr::BinaryOp {
                        op: BinaryOp::Lt,
                        rhs,
                        ..
                    }) => const_eval_with_params(rhs, param_vals)?,
                    Some(Expr::BinaryOp {
                        op: BinaryOp::Le,
                        rhs,
                        ..
                    }) => const_eval_with_params(rhs, param_vals)? + 1,
                    Some(c) => {
                        if const_eval_with_params(c, param_vals)? != 0 {
                            1
                        } else {
                            0
                        }
                    }
                    None => 0,
                };
                let step_val = extract_generate_step(step, param_vals);
                let max_iter = MAX_GENERATED_ITEMS / body_items.len().max(1);
                if step_val > 0 {
                    let mut cur = start_val;
                    let mut iter = 0usize;
                    while cur < limit {
                        if iter >= max_iter {
                            return Err(format!(
                                "generate for loop exceeded {} iterations (possible O(2^N) blowup)",
                                max_iter
                            ));
                        }
                        iter += 1;
                        for mut item in body_items.clone() {
                            substitute_genvar_in_module_item(&mut item, var.as_str(), cur);
                            result.push(item);
                        }
                        cur += step_val;
                    }
                } else if step_val < 0 {
                    let mut cur = start_val;
                    let mut iter = 0usize;
                    while cur > limit {
                        if iter >= max_iter {
                            return Err(format!(
                                "generate for loop exceeded {} iterations (possible O(2^N) blowup)",
                                max_iter
                            ));
                        }
                        iter += 1;
                        for mut item in body_items.clone() {
                            substitute_genvar_in_module_item(&mut item, var.as_str(), cur);
                            result.push(item);
                        }
                        cur += step_val;
                    }
                }
            }
            GenerateItem::Case {
                expr,
                items,
                default,
                ..
            } => {
                let case_val = match const_eval_with_params(expr, param_vals) {
                    Ok(v) => v,
                    Err(_) => {
                        eprintln!("  ** WARNING: non-constant expression in generate case, taking first case");
                        if let Some(first) = items.first() {
                            for item in &first.body {
                                result.push(item.clone());
                            }
                        } else if let Some(default_items) = default {
                            for item in default_items {
                                result.push(item.clone());
                            }
                        }
                        continue;
                    }
                };
                let mut matched = false;
                for ci in items {
                    for label in &ci.labels {
                        let label_val = const_eval_with_params(label, param_vals)?;
                        if label_val == case_val {
                            for item in &ci.body {
                                result.push(item.clone());
                            }
                            matched = true;
                            break;
                        }
                    }
                    if matched {
                        break;
                    }
                }
                if !matched {
                    if let Some(default_items) = default {
                        for item in default_items {
                            result.push(item.clone());
                        }
                    }
                }
            }
            GenerateItem::Items(items) => {
                for item in items {
                    result.push(item.clone());
                }
            }
        }
    }
    Ok(result)
}

/// Substitusi genvar di dalam satu ModuleItem.
pub fn substitute_genvar_in_module_item(item: &mut ModuleItem, var_name: &str, value: i64) {
    match item {
        ModuleItem::Always(always) => {
            for stmt in &mut always.stmts {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = substitute_loop_var_in_stmt(&old, var_name, value);
            }
        }
        ModuleItem::Initial(initial) => {
            for stmt in &mut initial.stmts {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = substitute_loop_var_in_stmt(&old, var_name, value);
            }
        }
        ModuleItem::Final(final_block) => {
            for stmt in &mut final_block.stmts {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = substitute_loop_var_in_stmt(&old, var_name, value);
            }
        }
        ModuleItem::Assign(assign) => {
            let old_lhs = std::mem::replace(
                &mut assign.lhs,
                Expr::Value(crate::ast::expr::Value::Decimal(0)),
            );
            let old_rhs = std::mem::replace(
                &mut assign.rhs,
                Expr::Value(crate::ast::expr::Value::Decimal(0)),
            );
            assign.lhs = substitute_loop_var_in_expr(&old_lhs, var_name, value);
            assign.rhs = substitute_loop_var_in_expr(&old_rhs, var_name, value);
        }
        ModuleItem::Instance(inst) => {
            if let Some(range) = &mut inst.range {
                let old_msb = std::mem::replace(
                    &mut range.msb,
                    Expr::Value(crate::ast::expr::Value::Decimal(0)),
                );
                let old_lsb = std::mem::replace(
                    &mut range.lsb,
                    Expr::Value(crate::ast::expr::Value::Decimal(0)),
                );
                range.msb = substitute_loop_var_in_expr(&old_msb, var_name, value);
                range.lsb = substitute_loop_var_in_expr(&old_lsb, var_name, value);
            }
            for (_, expr) in &mut inst.param_assigns {
                let old = std::mem::replace(expr, Expr::Value(crate::ast::expr::Value::Decimal(0)));
                *expr = substitute_loop_var_in_expr(&old, var_name, value);
            }
            for conn in &mut inst.port_conns {
                match conn {
                    PortConnection::Positional(expr) => {
                        let old = std::mem::replace(
                            expr,
                            Expr::Value(crate::ast::expr::Value::Decimal(0)),
                        );
                        *expr = substitute_loop_var_in_expr(&old, var_name, value);
                    }
                    PortConnection::Named { expr, .. } => {
                        let old = std::mem::replace(
                            expr,
                            Expr::Value(crate::ast::expr::Value::Decimal(0)),
                        );
                        *expr = substitute_loop_var_in_expr(&old, var_name, value);
                    }
                }
            }
        }
        ModuleItem::Decl(decl) => {
            for var in &mut decl.names {
                if let Some(er) = &var.expr_range {
                    let old_msb = er.msb.clone();
                    let old_lsb = er.lsb.clone();
                    let new_msb = substitute_loop_var_in_expr(&old_msb, var_name, value);
                    let new_lsb = substitute_loop_var_in_expr(&old_lsb, var_name, value);
                    if let (Ok(msb), Ok(lsb)) =
                        (const_eval_simple(&new_msb), const_eval_simple(&new_lsb))
                    {
                        var.expr_range = None;
                        var.range = Some(Range {
                            msb: msb as usize,
                            lsb: lsb as usize,
                        });
                    }
                }
            }
        }
        ModuleItem::Gate(ref mut gate) => {
            for port in &mut gate.ports {
                let old = std::mem::replace(port, Expr::Value(crate::ast::expr::Value::Decimal(0)));
                *port = substitute_loop_var_in_expr(&old, var_name, value);
            }
        }
        ModuleItem::Generate(gen) => {
            for gi in &mut gen.items {
                substitute_genvar_in_generate_item(gi, var_name, value);
            }
        }
        ModuleItem::Func(_)
        | ModuleItem::Typedef(_)
        | ModuleItem::Import { .. }
        | ModuleItem::Covergroup(_)
        | ModuleItem::DpiImport(_)
        | ModuleItem::DpiExport(_)
        | ModuleItem::Param(_)
        | ModuleItem::Clocking(_)
        | ModuleItem::Specify(_)
        | ModuleItem::VirtualInterface { .. } => {}
    }
}

/// Substitusi genvar di dalam satu GenerateItem.
pub fn substitute_genvar_in_generate_item(item: &mut GenerateItem, var_name: &str, value: i64) {
    match item {
        GenerateItem::If {
            cond,
            true_items,
            false_items,
        } => {
            let old_cond =
                std::mem::replace(cond, Expr::Value(crate::ast::expr::Value::Decimal(0)));
            *cond = substitute_loop_var_in_expr(&old_cond, var_name, value);
            for item in true_items.iter_mut() {
                substitute_genvar_in_module_item(item, var_name, value);
            }
            for item in false_items.iter_mut() {
                substitute_genvar_in_module_item(item, var_name, value);
            }
        }
        GenerateItem::For {
            var: _,
            init,
            cond,
            step,
            body_items,
        } => {
            if let Some(stmt) = init {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = substitute_loop_var_in_stmt(&old, var_name, value);
            }
            if let Some(expr) = cond {
                let old = std::mem::replace(expr, Expr::Value(crate::ast::expr::Value::Decimal(0)));
                *expr = substitute_loop_var_in_expr(&old, var_name, value);
            }
            if let Some(stmt) = step {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = substitute_loop_var_in_stmt(&old, var_name, value);
            }
            for item in body_items.iter_mut() {
                substitute_genvar_in_module_item(item, var_name, value);
            }
        }
        GenerateItem::Case {
            expr,
            items,
            default,
            ..
        } => {
            let old_expr =
                std::mem::replace(expr, Expr::Value(crate::ast::expr::Value::Decimal(0)));
            *expr = substitute_loop_var_in_expr(&old_expr, var_name, value);
            for ci in items.iter_mut() {
                for label in ci.labels.iter_mut() {
                    let old =
                        std::mem::replace(label, Expr::Value(crate::ast::expr::Value::Decimal(0)));
                    *label = substitute_loop_var_in_expr(&old, var_name, value);
                }
                for item in ci.body.iter_mut() {
                    substitute_genvar_in_module_item(item, var_name, value);
                }
            }
            if let Some(default_items) = default {
                for item in default_items.iter_mut() {
                    substitute_genvar_in_module_item(item, var_name, value);
                }
            }
        }
        GenerateItem::Items(items) => {
            for item in items.iter_mut() {
                substitute_genvar_in_module_item(item, var_name, value);
            }
        }
    }
}
