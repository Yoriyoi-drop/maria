use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::ast::types::const_eval_simple;
use crate::ast::types::const_eval_with_params;
use crate::ast::types::string_to_i64;
use crate::ir::*;

pub fn is_2state_type(dtype: &DataType) -> bool {
    matches!(dtype, DataType::Bit | DataType::Byte | DataType::Shortint | DataType::Int | DataType::Longint | DataType::Time)
}

pub fn is_signed_type(dtype: &DataType) -> bool {
    matches!(dtype, DataType::Signed(_))
}

pub fn collect_body_params(module: &Module) -> Vec<ParamDecl> {
    let mut params = Vec::new();
    for item in &module.items {
        match item {
            ModuleItem::Param(p) => params.push(p.clone()),
            ModuleItem::Generate(gen) => {
                for gi in &gen.items {
                    if let GenerateItem::Items(items) = gi {
                        for i in items {
                            if let ModuleItem::Param(p) = i {
                                params.push(p.clone());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    params
}

pub fn resolve_param_values_fn(module: &Module, instance_overrides: &HashMap<String, i64>) -> Result<HashMap<String, i64>, String> {
    let mut vals = HashMap::new();
    // First, collect positional overrides indexed by position in module's param list
    let mut positional_overrides: Vec<i64> = Vec::new();
    for (name, val) in instance_overrides {
        if name.starts_with("__param") {
            let idx: usize = name.trim_start_matches("__param").parse().unwrap_or(0);
            if idx >= positional_overrides.len() {
                positional_overrides.resize(idx + 1, 0);
            }
            positional_overrides[idx] = *val;
        }
    }

    // Collect body-level parameter declarations and add them to vals first
    // so they can be referenced by module.params expressions
    // Helper to evaluate a parameter's default value, handling string defaults
    let eval_param_default = |e: &Expr, existing_vals: &HashMap<String, i64>| -> i64 {
        match e {
            Expr::String(s) => string_to_i64(s),
            _ => const_eval_with_params(e, existing_vals).unwrap_or(0),
        }
    };

    // Collect body-level parameter declarations and add them to vals first
    // so they can be referenced by module.params expressions
    for param in collect_body_params(module) {
        if !vals.contains_key(&param.name) {
            match &param.default {
                Some(e) => {
                    let v = eval_param_default(e, &vals);
                    vals.insert(param.name.clone(), v);
                }
                None => {
                    vals.insert(param.name.clone(), 0);
                }
            }
        }
    }

    for (i, param) in module.params.iter().enumerate() {
        if param.is_localparam {
            if let Some(e) = &param.default {
                vals.insert(param.name.clone(), eval_param_default(e, &vals));
            } else {
                vals.insert(param.name.clone(), 0);
            }
            continue;
        }
        let val = if i < positional_overrides.len() {
            positional_overrides[i]
        } else if let Some(override_val) = instance_overrides.get(&param.name) {
            *override_val
        } else {
            match &param.default {
                Some(e) => eval_param_default(e, &vals),
                None => 0,
            }
        };
        vals.insert(param.name.clone(), val);
    }
    Ok(vals)
}

pub fn detect_sync_reset(body: &[IrStmt]) -> Option<ResetInfo> {
    if let Some(IrStmt::If { cond: IrExpr::Signal(sig_id, _), .. }) = body.first() {
        return Some(ResetInfo {
            signal: *sig_id,
            polarity: true,
            r#async: false,
            value: LogicVec::new(1),
        });
    }
    None
}

pub fn expand_all_generates(module: &mut Module, param_vals: &HashMap<String, i64>) -> Result<(), String> {
    let mut i = 0;
    while i < module.items.len() {
        if let ModuleItem::Generate(gen) = &module.items[i] {
            let expanded = expand_generate_block(gen, param_vals)?;
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

pub fn extract_generate_step(step: &Option<Stmt>, param_vals: &HashMap<String, i64>) -> i64 {
    let Some(Stmt::BlockingAssign { rhs, .. }) = step else { return 1 };
    match rhs {
        Expr::BinaryOp { op: BinaryOp::Add, lhs: _, rhs } => {
            const_eval_with_params(rhs, param_vals).unwrap_or(1)
        }
        Expr::BinaryOp { op: BinaryOp::Sub, lhs: _, rhs } => {
            -const_eval_with_params(rhs, param_vals).unwrap_or(1)
        }
        _ => 1
    }
}

pub fn expand_generate_block(gen: &GenerateBlock, param_vals: &HashMap<String, i64>) -> Result<Vec<ModuleItem>, String> {
    let mut result = Vec::new();
    for item in &gen.items {
        match item {
            GenerateItem::If { cond, true_items, false_items } => {
                let val = const_eval_with_params(cond, param_vals)
                    .map_err(|_| format!("non-constant condition in generate if"))?;
                let branch = if val != 0 { true_items } else { false_items };
                for item in branch {
                    result.push(item.clone());
                }
            }
            GenerateItem::For { var, init, cond, step, body_items } => {
                let start_val: i64 = match init {
                    Some(Stmt::BlockingAssign { rhs, .. }) => const_eval_with_params(rhs, param_vals)?,
                    _ => 0,
                };
                let limit: i64 = match cond {
                    Some(Expr::BinaryOp { op: BinaryOp::Lt, rhs, .. }) => {
                        const_eval_with_params(rhs, param_vals)?
                    }
                    Some(Expr::BinaryOp { op: BinaryOp::Le, rhs, .. }) => {
                        const_eval_with_params(rhs, param_vals)? + 1
                    }
                    Some(c) => {
                        if const_eval_with_params(c, param_vals)? != 0 { 1 } else { 0 }
                    }
                    None => 0,
                };
                let step_val = extract_generate_step(step, param_vals);
                if step_val > 0 {
                    let mut cur = start_val;
                    while cur < limit {
                        for mut item in body_items.clone() {
                            substitute_genvar_in_module_item(&mut item, var, cur);
                            result.push(item);
                        }
                        cur += step_val;
                    }
                } else if step_val < 0 {
                    let mut cur = start_val;
                    while cur > limit {
                        for mut item in body_items.clone() {
                            substitute_genvar_in_module_item(&mut item, var, cur);
                            result.push(item);
                        }
                        cur += step_val;
                    }
                }
            }
            GenerateItem::Case { case_type: _case_type, expr, items, default } => {
                let case_val = const_eval_with_params(expr, param_vals)
                    .map_err(|_| format!("non-constant expression in generate case"))?;
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
                    if matched { break; }
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
                &mut assign.lhs, Expr::Value(crate::ast::expr::Value::Decimal(0))
            );
            let old_rhs = std::mem::replace(
                &mut assign.rhs, Expr::Value(crate::ast::expr::Value::Decimal(0))
            );
            assign.lhs = substitute_loop_var_in_expr(&old_lhs, var_name, value);
            assign.rhs = substitute_loop_var_in_expr(&old_rhs, var_name, value);
        }
        ModuleItem::Instance(inst) => {
            if let Some(range) = &mut inst.range {
                let old_msb = std::mem::replace(
                    &mut range.msb, Expr::Value(crate::ast::expr::Value::Decimal(0))
                );
                let old_lsb = std::mem::replace(
                    &mut range.lsb, Expr::Value(crate::ast::expr::Value::Decimal(0))
                );
                range.msb = substitute_loop_var_in_expr(&old_msb, var_name, value);
                range.lsb = substitute_loop_var_in_expr(&old_lsb, var_name, value);
            }
            for (_, expr) in &mut inst.param_assigns {
                let old = std::mem::replace(
                    expr, Expr::Value(crate::ast::expr::Value::Decimal(0))
                );
                *expr = substitute_loop_var_in_expr(&old, var_name, value);
            }
            for conn in &mut inst.port_conns {
                match conn {
                    PortConnection::Positional(expr) => {
                        let old = std::mem::replace(
                            expr, Expr::Value(crate::ast::expr::Value::Decimal(0))
                        );
                        *expr = substitute_loop_var_in_expr(&old, var_name, value);
                    }
                    PortConnection::Named { expr, .. } => {
                        let old = std::mem::replace(
                            expr, Expr::Value(crate::ast::expr::Value::Decimal(0))
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
                    if let (Ok(msb), Ok(lsb)) = (const_eval_simple(&new_msb), const_eval_simple(&new_lsb)) {
                        var.expr_range = None;
                        var.range = Some(Range { msb: msb as usize, lsb: lsb as usize });
                    }
                }
            }
        }
        ModuleItem::Gate(ref mut gate) => {
            for port in &mut gate.ports {
                let old = std::mem::replace(
                    port, Expr::Value(crate::ast::expr::Value::Decimal(0))
                );
                *port = substitute_loop_var_in_expr(&old, var_name, value);
            }
        }
        ModuleItem::Generate(gen) => {
            for gi in &mut gen.items {
                substitute_genvar_in_generate_item(gi, var_name, value);
            }
        }
        ModuleItem::Func(_) | ModuleItem::Typedef(_) | ModuleItem::Import { .. }
        | ModuleItem::Covergroup(_) | ModuleItem::DpiImport(_) | ModuleItem::Param(_) => {}
    }
}

pub fn substitute_genvar_in_generate_item(item: &mut GenerateItem, var_name: &str, value: i64) {
    match item {
        GenerateItem::If { cond, true_items, false_items } => {
            let old_cond = std::mem::replace(cond, Expr::Value(crate::ast::expr::Value::Decimal(0)));
            *cond = substitute_loop_var_in_expr(&old_cond, var_name, value);
            for item in true_items.iter_mut() {
                substitute_genvar_in_module_item(item, var_name, value);
            }
            for item in false_items.iter_mut() {
                substitute_genvar_in_module_item(item, var_name, value);
            }
        }
        GenerateItem::For { var: _, init, cond, step, body_items } => {
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
        GenerateItem::Case { expr, items, default, .. } => {
            let old_expr = std::mem::replace(expr, Expr::Value(crate::ast::expr::Value::Decimal(0)));
            *expr = substitute_loop_var_in_expr(&old_expr, var_name, value);
            for ci in items.iter_mut() {
                for label in ci.labels.iter_mut() {
                    let old = std::mem::replace(label, Expr::Value(crate::ast::expr::Value::Decimal(0)));
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

pub fn try_unroll_for_loop<'a, F>(init: Option<&'a Stmt>, cond: Option<&'a Expr>, step: Option<&'a Stmt>,
                           stmts: &[Stmt], elaborate_body: &F,
                           params: &HashMap<String, i64>)
    -> Result<Option<Vec<IrStmt>>, String>
    where F: Fn(&[Stmt], &str, i64) -> Result<Vec<IrStmt>, String>
{
    // Extract loop variable name and initial value from init statement
    let (var_name, init_val) = match init {
        Some(Stmt::BlockingAssign { lhs: Expr::Ident(name), rhs, .. }) => {
            (name.clone(), const_eval_with_params(rhs, params)?)
        }
        _ => return Ok(None),
    };

    // Extract step function from step statement
    let step_fn: Box<dyn Fn(i64) -> Result<i64, String>> = match step {
        Some(Stmt::BlockingAssign { lhs: Expr::Ident(n), rhs, .. }) if *n == var_name => {
            match rhs {
                Expr::BinaryOp { op: BinaryOp::Add, lhs, rhs } => {
                    if let Expr::Ident(n2) = lhs.as_ref() {
                        if n2 == &var_name {
                            let inc = const_eval_with_params(rhs, params)?;
                            Box::new(move |v| Ok(v + inc))
                        } else { return Ok(None) }
                    } else if let Expr::Ident(n2) = rhs.as_ref() {
                        if n2 == &var_name {
                            let inc = const_eval_with_params(lhs, params)?;
                            Box::new(move |v| Ok(v + inc))
                        } else { return Ok(None) }
                    } else { return Ok(None) }
                }
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };

    // Extract loop limit from condition: var < limit
    let limit = match cond {
        Some(Expr::BinaryOp { op: BinaryOp::Lt, lhs, rhs }) => {
            match lhs.as_ref() {
                Expr::Ident(n) if *n == var_name => const_eval_with_params(rhs, params)?,
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };

    // Unroll the loop
    let mut all_stmts = Vec::new();
    let mut ivar = init_val;
    while ivar < limit {
        let body = elaborate_body(stmts, &var_name, ivar)?;
        all_stmts.extend(body);
        ivar = step_fn(ivar)?;
    }

    Ok(Some(all_stmts))
}

pub fn substitute_loop_var_in_stmts(stmts: &[Stmt], var_name: &str, value: i64) -> Vec<Stmt> {
    stmts.iter().map(|s| substitute_loop_var_in_stmt(s, var_name, value)).collect()
}

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
        Stmt::IfElse { cond, true_branch, false_branch } => Stmt::IfElse {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            true_branch: Box::new(substitute_loop_var_in_stmt(true_branch, var_name, value)),
            false_branch: false_branch.as_ref().map(|fb|
                Box::new(substitute_loop_var_in_stmt(fb, var_name, value))),
        },
        Stmt::Case { expr, items, default } => Stmt::Case {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items.iter().map(|item| crate::ast::stmt::CaseItem {
                labels: item.labels.iter().map(|l| substitute_loop_var_in_expr(l, var_name, value)).collect(),
                stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
            }).collect(),
            default: default.as_ref().map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
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
            args: args.iter().map(|a| substitute_loop_var_in_expr(a, var_name, value)).collect(),
        },
        Stmt::Expr { expr } => Stmt::Expr {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
        },
        Stmt::CaseX { expr, items, default } => Stmt::CaseX {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items.iter().map(|item| crate::ast::stmt::CaseItem {
                labels: item.labels.iter().map(|l| substitute_loop_var_in_expr(l, var_name, value)).collect(),
                stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
            }).collect(),
            default: default.as_ref().map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::CaseZ { expr, items, default } => Stmt::CaseZ {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items.iter().map(|item| crate::ast::stmt::CaseItem {
                labels: item.labels.iter().map(|l| substitute_loop_var_in_expr(l, var_name, value)).collect(),
                stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
            }).collect(),
            default: default.as_ref().map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::StmtCase { expr, items, default } => Stmt::StmtCase {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items.iter().map(|item| crate::ast::stmt::CaseItem {
                labels: item.labels.iter().map(|l| substitute_loop_var_in_expr(l, var_name, value)).collect(),
                stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
            }).collect(),
            default: default.as_ref().map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::LoopForever { stmts } => Stmt::LoopForever {
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::LoopWhile { cond, stmts } => Stmt::LoopWhile {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::LoopFor { init, cond, step, stmts } => Stmt::LoopFor {
            init: init.as_ref().map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
            cond: cond.as_ref().map(|c| substitute_loop_var_in_expr(c, var_name, value)),
            step: step.as_ref().map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::Repeat { count, stmts } => Stmt::Repeat {
            count: substitute_loop_var_in_expr(count, var_name, value),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::Wait { cond, stmt } => Stmt::Wait {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            stmt: stmt.as_ref().map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
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
            expr.as_ref().map(|e| Box::new(substitute_loop_var_in_expr(e, var_name, value))),
        ),
        Stmt::Null => Stmt::Null,
        Stmt::SysFinish => Stmt::SysFinish,
        Stmt::EventControl { events, stmt } => Stmt::EventControl {
            events: events.iter().map(|e| substitute_sensitivity_event(e, var_name, value)).collect(),
            stmt: stmt.as_ref().map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
        },
        Stmt::EventTrigger { name } => Stmt::EventTrigger { name: name.clone() },
        Stmt::ForeachLoop { array_var, index_vars, stmts } => Stmt::ForeachLoop {
            array_var: array_var.clone(),
            index_vars: index_vars.clone(),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::NamedBlock { name, stmts, decls } => Stmt::NamedBlock {
            name: name.clone(),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
            decls: decls.clone(),
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

pub fn substitute_sensitivity_event(event: &SensitivityEvent, var_name: &str, value: i64) -> SensitivityEvent {
    match event {
        SensitivityEvent::PosEdge(e) => SensitivityEvent::PosEdge(substitute_loop_var_in_expr(e, var_name, value)),
        SensitivityEvent::NegEdge(e) => SensitivityEvent::NegEdge(substitute_loop_var_in_expr(e, var_name, value)),
        SensitivityEvent::Level(e) => SensitivityEvent::Level(substitute_loop_var_in_expr(e, var_name, value)),
        SensitivityEvent::Wildcard => SensitivityEvent::Wildcard,
    }
}

pub fn substitute_loop_var_in_expr(expr: &Expr, var_name: &str, value: i64) -> Expr {
    match expr {
        Expr::Ident(name) if name == var_name => Expr::Value(crate::ast::expr::Value::Decimal(value)),
        Expr::Ident(_) => expr.clone(),
        Expr::Value(_) | Expr::String(_) | Expr::Null => expr.clone(),
        Expr::RangeSelect { expr: inner, msb, lsb } => Expr::RangeSelect {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            msb: Box::new(substitute_loop_var_in_expr(msb, var_name, value)),
            lsb: Box::new(substitute_loop_var_in_expr(lsb, var_name, value)),
        },
        Expr::BitSelect { expr: inner, index } => Expr::BitSelect {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            index: Box::new(substitute_loop_var_in_expr(index, var_name, value)),
        },
        Expr::PartSelect { expr: inner, base, width } => Expr::PartSelect {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            base: Box::new(substitute_loop_var_in_expr(base, var_name, value)),
            width: Box::new(substitute_loop_var_in_expr(width, var_name, value)),
        },
        Expr::Concat(exprs) => Expr::Concat(
            exprs.iter().map(|e| substitute_loop_var_in_expr(e, var_name, value)).collect()
        ),
        Expr::FuncCall { name, args } => Expr::FuncCall {
            name: name.clone(),
            args: args.iter().map(|a| substitute_loop_var_in_expr(a, var_name, value)).collect(),
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
        Expr::TernaryOp { cond, true_expr, false_expr } => Expr::TernaryOp {
            cond: Box::new(substitute_loop_var_in_expr(cond, var_name, value)),
            true_expr: Box::new(substitute_loop_var_in_expr(true_expr, var_name, value)),
            false_expr: Box::new(substitute_loop_var_in_expr(false_expr, var_name, value)),
        },
        Expr::Paren(inner) => Expr::Paren(Box::new(substitute_loop_var_in_expr(inner, var_name, value))),
        Expr::MethodCall { obj, method, args } => Expr::MethodCall {
            obj: Box::new(substitute_loop_var_in_expr(obj, var_name, value)),
            method: method.clone(),
            args: args.iter().map(|a| substitute_loop_var_in_expr(a, var_name, value)).collect(),
        },
        Expr::MemberAccess { obj, field } => Expr::MemberAccess {
            obj: Box::new(substitute_loop_var_in_expr(obj, var_name, value)),
            field: field.clone(),
        },
        Expr::FillLit(val) => Expr::FillLit(*val),
        Expr::Inside { expr: inner, range_list } => Expr::Inside {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            range_list: range_list.iter().map(|e| substitute_loop_var_in_expr(e, var_name, value)).collect(),
        },
        Expr::StreamingConcat { op, slices } => Expr::StreamingConcat {
            op: op.clone(),
            slices: slices.iter().map(|e| substitute_loop_var_in_expr(e, var_name, value)).collect(),
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

pub fn infer_comb_sensitivity(body: &[IrStmt]) -> Vec<SignalId> {
    let mut sigs = Vec::new();
    collect_read_signals_stmts(body, &mut sigs);
    sigs.sort();
    sigs.dedup();
    sigs
}

pub fn collect_read_signals_stmts(stmts: &[IrStmt], out: &mut Vec<SignalId>) {
    for stmt in stmts {
        collect_read_signals_stmt(stmt, out);
    }
}

pub fn collect_read_signals_stmt(stmt: &IrStmt, out: &mut Vec<SignalId>) {
    match stmt {
        IrStmt::Block { stmts } => collect_read_signals_stmts(stmts, out),
        IrStmt::BlockingAssign { rhs, .. } | IrStmt::NonBlockingAssign { rhs, .. } => {
            collect_read_signals_expr(rhs, out);
        }
        IrStmt::If { cond, true_branch, false_branch } => {
            collect_read_signals_expr(cond, out);
            collect_read_signals_stmts(true_branch, out);
            collect_read_signals_stmts(false_branch, out);
        }
        IrStmt::Case { expr, items, default, .. } => {
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
        IrStmt::LoopFor { init, cond, step, body } => {
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
        _ => {}
    }
}

pub fn collect_read_signals_expr(expr: &IrExpr, out: &mut Vec<SignalId>) {
    match expr {
        IrExpr::Signal(id, _) | IrExpr::RangeSelect(id, ..) | IrExpr::BitSelect(id, _) | IrExpr::ArrayIndex { sig_id: id, .. } => {
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
    }
}

pub fn resolve_expr_signal(expr: &Expr, signal_map: &HashMap<String, SignalId>) -> Option<SignalId> {
    match expr {
        Expr::Ident(name) => signal_map.get(name).copied(),
        Expr::MethodCall { .. } => None,
        Expr::MemberAccess { .. } => None,
        _ => None,
    }
}

pub fn compute_expr_width(expr: &Expr, signal_map: &HashMap<String, SignalId>,
                      signals: &[SignalInfo], param_vals: &HashMap<String, i64>) -> Result<usize, String> {
    match expr {
        Expr::Ident(name) => {
            if let Some(sig_id) = signal_map.get(name) {
                let info = &signals[*sig_id];
                Ok(info.width * if info.array_depth > 0 { info.array_depth } else { 1 })
            } else if let Some(&val) = param_vals.get(name) {
                let abs = val.unsigned_abs();
                Ok(if val == 0 { 1 } else { 64 - (abs.leading_zeros() as usize) }.max(1))
            } else {
                Err(format!("cannot determine width of '{}'", name))
            }
        }
        Expr::Value(v) => {
            match v {
                Value::Binary { width, .. } => Ok(width.unwrap_or(1)),
                Value::Hex { width, .. } => Ok(width.unwrap_or(1)),
                Value::Octal { width, .. } => Ok(width.unwrap_or(1)),
                Value::Decimal(_) => Ok(32),
                Value::Real(_) => Ok(64),
            }
        }
        Expr::FillLit(_) => Ok(1),
        Expr::FuncCall { name, .. } => {
            if let Some(width) = param_vals.get(name) {
                let abs = width.unsigned_abs();
                Ok(if *width == 0 { 1 } else { 64 - (abs.leading_zeros() as usize) }.max(1))
            } else {
                Err(format!("cannot determine width of function '{}'", name))
            }
        }
        Expr::Paren(inner) => compute_expr_width(inner, signal_map, signals, param_vals),
        Expr::UnaryOp { op, expr: inner } => {
            match op {
                UnaryOp::ReductionAnd | UnaryOp::ReductionNand | UnaryOp::ReductionOr
                | UnaryOp::ReductionNor | UnaryOp::ReductionXor | UnaryOp::ReductionXnor
                | UnaryOp::Not => Ok(1),
                _ => compute_expr_width(inner, signal_map, signals, param_vals),
            }
        }
        Expr::BinaryOp { lhs, rhs, .. } => {
            let lw = compute_expr_width(lhs, signal_map, signals, param_vals)?;
            let rw = compute_expr_width(rhs, signal_map, signals, param_vals)?;
            Ok(lw.max(rw))
        }
        Expr::Concat(items) => {
            let mut total = 0;
            for item in items {
                total += compute_expr_width(item, signal_map, signals, param_vals)?;
            }
            Ok(total)
        }
        Expr::Replicate { count, expr: inner } => {
            let c = const_eval_with_params(count, param_vals).unwrap_or(1) as usize;
            let w = compute_expr_width(inner, signal_map, signals, param_vals)?;
            Ok(c * w)
        }
        Expr::TernaryOp { true_expr, false_expr, .. } => {
            let tw = compute_expr_width(true_expr, signal_map, signals, param_vals)?;
            let fw = compute_expr_width(false_expr, signal_map, signals, param_vals)?;
            Ok(tw.max(fw))
        }
        Expr::RangeSelect { msb, lsb, .. } => {
            if let (Ok(m), Ok(l)) = (const_eval_with_params(msb, param_vals), const_eval_with_params(lsb, param_vals)) {
                Ok((m.abs_diff(l) + 1) as usize)
            } else {
                Err("dynamic range select width not computable at compile time".to_string())
            }
        }
        Expr::BitSelect { .. } => Ok(1),
        Expr::PartSelect { width, .. } => {
            Ok(const_eval_with_params(width, param_vals).unwrap_or(1) as usize)
        }
        Expr::MemberAccess { obj, field } => {
            // Check if obj resolves to a struct signal
            if let Expr::Ident(name) = obj.as_ref() {
                if let Some(&sig_id) = signal_map.get(name) {
                    if !signals[sig_id].struct_fields.is_empty() {
                        if let Some(f) = signals[sig_id].struct_fields.iter().find(|f| f.name == *field) {
                            return Ok(f.width);
                        }
                    }
                }
            }
            compute_expr_width(obj, signal_map, signals, param_vals)
        }
        Expr::Cast { dtype, .. } => {
            match super::parse_type_spec_str(dtype) {
                Some(dt) => match dt {
                    DataType::UserDefined(name) => {
                        param_vals.get(&name).map(|&v| v as usize).ok_or_else(|| format!("unknown type '{}'", name))
                    }
                    _ => Ok(dt.width()),
                },
                None => Err(format!("unknown type '{}' in cast", dtype)),
            }
        }
        Expr::MethodCall { .. } | Expr::StreamingConcat { .. } => {
            Err("width not computable for this expression type".to_string())
        }
        Expr::ScopedIdent { package, item } => {
            Err(format!("cannot determine width of '{}.{}' at compile time", package, item))
        }
        _ => Err("cannot determine width of expression".to_string()),
    }
}

pub fn collect_sensitivity(expr: &Expr, signal_map: &HashMap<String, SignalId>) -> Vec<SignalId> {
    match expr {
        Expr::Ident(name) => {
            signal_map.get(name).map(|&id| vec![id]).unwrap_or_default()
        }
        Expr::BinaryOp { lhs, rhs, .. } => {
            let mut v = collect_sensitivity(lhs, signal_map);
            v.extend(collect_sensitivity(rhs, signal_map));
            v
        }
        Expr::UnaryOp { expr: inner, .. } => collect_sensitivity(inner, signal_map),
        Expr::Concat(exprs) => {
            exprs.iter().flat_map(|e| collect_sensitivity(e, signal_map)).collect()
        }
        Expr::BitSelect { expr: inner, index } => {
            let mut v = collect_sensitivity(inner, signal_map);
            v.extend(collect_sensitivity(index, signal_map));
            v
        }
        Expr::RangeSelect { expr: inner, msb, lsb } => {
            let mut v = collect_sensitivity(inner, signal_map);
            v.extend(collect_sensitivity(msb, signal_map));
            v.extend(collect_sensitivity(lsb, signal_map));
            v
        }
        Expr::PartSelect { expr: inner, base, width } => {
            let mut v = collect_sensitivity(inner, signal_map);
            v.extend(collect_sensitivity(base, signal_map));
            v.extend(collect_sensitivity(width, signal_map));
            v
        }
        Expr::TernaryOp { cond, true_expr, false_expr } => {
            let mut v = collect_sensitivity(cond, signal_map);
            v.extend(collect_sensitivity(true_expr, signal_map));
            v.extend(collect_sensitivity(false_expr, signal_map));
            v
        }
        Expr::MethodCall { obj, .. } => collect_sensitivity(obj, signal_map),
        Expr::MemberAccess { obj, .. } => collect_sensitivity(obj, signal_map),
        _ => vec![],
    }
}

pub fn const_eval_params(expr: &Expr, params: &HashMap<String, i64>) -> Result<i64, String> {
    const_eval_with_params(expr, params)
}

pub fn try_fold_const(expr: &Expr, params: &HashMap<String, i64>) -> Result<Option<IrExpr>, String> {
    match const_eval_with_params(expr, params) {
        Ok(val) => {
            let abs = val.unsigned_abs();
            let min_width = if val >= 0 {
                if val == 0 { 1 } else { 64 - (abs.leading_zeros() as usize) }
            } else {
                64 - (abs.leading_zeros() as usize) + 1
            };
            let width = min_width.max(32);
            Ok(Some(IrExpr::Const(LogicVec::from_u64(val as u64, width))))
        }
        Err(_) => Ok(None),
    }
}

pub fn value_to_logicvec(val: &Value) -> LogicVec {
    match val {
        Value::Decimal(n) => {
            let abs = n.unsigned_abs();
            let min_width = if *n == 0 { 1 } else { 64 - (abs.leading_zeros() as usize) };
            let width = min_width.max(32);
            let mut lv = LogicVec::from_u64(abs, width);
            if *n < 0 {
                // Two's complement
                for b in lv.bits.iter_mut() {
                    *b = match b {
                        LogicVal::Zero => LogicVal::One,
                        LogicVal::One => LogicVal::Zero,
                        _ => LogicVal::X,
                    };
                }
                // Add 1
                let mut carry = true;
                for b in lv.bits.iter_mut() {
                    if carry {
                        match b {
                            LogicVal::Zero => { *b = LogicVal::One; carry = false; }
                            LogicVal::One => { *b = LogicVal::Zero; }
                            _ => {}
                        }
                    }
                }
            }
            lv
        }
        Value::Binary { bits, width, .. } => {
            let w = width.unwrap_or(bits.len());
            let mut vec = LogicVec::new(w);
            for (i, c) in bits.chars().rev().enumerate() {
                if i >= w { break; }
                vec.bits[i] = match c {
                    '0' => LogicVal::Zero,
                    '1' => LogicVal::One,
                    'x' | 'X' => LogicVal::X,
                    'z' | 'Z' => LogicVal::Z,
                    '_' => continue,
                    _ => LogicVal::X,
                };
            }
            vec
        }
        Value::Hex { bits, width, .. } => {
            let w = width.unwrap_or(bits.len() * 4);
            let mut vec = LogicVec::new(w);
            let digits: String = bits.chars().filter(|c| *c != '_').collect();
            for (i, c) in digits.chars().rev().enumerate() {
                let hex_val = c.to_digit(16).unwrap_or(0);
                for j in 0..4 {
                    let bit_idx = i * 4 + j;
                    if bit_idx >= w { break; }
                    vec.bits[bit_idx] = if (hex_val >> j) & 1 == 1 {
                        LogicVal::One
                    } else {
                        LogicVal::Zero
                    };
                }
            }
            vec
        }
        Value::Octal { bits, width, .. } => {
            let w = width.unwrap_or(bits.len() * 3);
            let mut vec = LogicVec::new(w);
            let digits: String = bits.chars().filter(|c| *c != '_').collect();
            for (i, c) in digits.chars().rev().enumerate() {
                let oct_val = c.to_digit(8).unwrap_or(0);
                for j in 0..3 {
                    let bit_idx = i * 3 + j;
                    if bit_idx >= w { break; }
                    vec.bits[bit_idx] = if (oct_val >> j) & 1 == 1 {
                        LogicVal::One
                    } else {
                        LogicVal::Zero
                    };
                }
            }
            vec
        }
        Value::Real(r) => LogicVec::from_u64(r.to_bits(), 64),
    }
}

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

pub fn build_gate_expr(gate_type: &GateType, inputs: &[IrExpr]) -> IrExpr {
    match gate_type {
        GateType::And => fold_binary(BinaryIrOp::BitAnd, inputs),
        GateType::Or => fold_binary(BinaryIrOp::BitOr, inputs),
        GateType::Nand => IrExpr::UnaryOp(UnaryIrOp::BitNot, Box::new(fold_binary(BinaryIrOp::BitAnd, inputs))),
        GateType::Nor => IrExpr::UnaryOp(UnaryIrOp::BitNot, Box::new(fold_binary(BinaryIrOp::BitOr, inputs))),
        GateType::Xor => fold_binary(BinaryIrOp::BitXor, inputs),
        GateType::Xnor => IrExpr::UnaryOp(UnaryIrOp::BitNot, Box::new(fold_binary(BinaryIrOp::BitXor, inputs))),
        GateType::Buf => inputs[0].clone(),
        GateType::Not => IrExpr::UnaryOp(UnaryIrOp::BitNot, Box::new(inputs[0].clone())),
    }
}

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

pub fn substitute_class_types(cd: ClassDecl, param_name: &str, replacement: &DataType) -> ClassDecl {
    let mut new_members = Vec::new();
    for member in cd.members {
        match member {
            ClassMember::Decl(mut decl) => {
                decl.dtype = substitute_data_type(decl.dtype, param_name, replacement);
                new_members.push(ClassMember::Decl(decl));
            }
            ClassMember::Function(mut fd) => {
                fd.return_type = fd.return_type.map(|dt| {
                    Box::new(substitute_data_type(*dt, param_name, replacement))
                });
                new_members.push(ClassMember::Function(fd));
            }
            ClassMember::Task(td) => {
                new_members.push(ClassMember::Task(td));
            }
            ClassMember::Constraint { name, body } => {
                let new_body = body.into_iter().map(|ci| {
                    match ci {
                        ConstraintItem::Expr(e) => {
                            ConstraintItem::Expr(substitute_expr_types(e, param_name, replacement))
                        }
                    }
                }).collect();
                new_members.push(ClassMember::Constraint { name, body: new_body });
            }
            other => new_members.push(other),
        }
    }
    ClassDecl { members: new_members, ..cd }
}

pub fn substitute_data_type(dt: DataType, param_name: &str, replacement: &DataType) -> DataType {
    match dt {
        DataType::UserDefined(ref name) if name == param_name => replacement.clone(),
        DataType::Signed(inner) => DataType::Signed(Box::new(substitute_data_type(*inner, param_name, replacement))),
        DataType::EnumType { base, members } => {
            DataType::EnumType {
                base: base.map(|b| Box::new(substitute_data_type(*b, param_name, replacement))),
                members,
            }
        }
        DataType::StructType { members } => {
            DataType::StructType {
                members: members.into_iter().map(|m| {
                    StructMember {
                        dtype: Box::new(substitute_data_type(*m.dtype, param_name, replacement)),
                        ..m
                    }
                }).collect(),
            }
        }
        DataType::UnionType { members } => {
            DataType::UnionType {
                members: members.into_iter().map(|m| {
                    StructMember {
                        dtype: Box::new(substitute_data_type(*m.dtype, param_name, replacement)),
                        ..m
                    }
                }).collect(),
            }
        }
        other => other,
    }
}

pub fn substitute_expr_types(e: Expr, param_name: &str, replacement: &DataType) -> Expr {
    match e {
        Expr::BinaryOp { lhs, op, rhs } => {
            Expr::BinaryOp {
                lhs: Box::new(substitute_expr_types(*lhs, param_name, replacement)),
                op,
                rhs: Box::new(substitute_expr_types(*rhs, param_name, replacement)),
            }
        }
        Expr::UnaryOp { op, expr } => {
            Expr::UnaryOp { op, expr: Box::new(substitute_expr_types(*expr, param_name, replacement)) }
        }
        Expr::Paren(inner) => {
            Expr::Paren(Box::new(substitute_expr_types(*inner, param_name, replacement)))
        }
        Expr::Concat(items) => {
            Expr::Concat(items.into_iter().map(|e| substitute_expr_types(e, param_name, replacement)).collect())
        }
        Expr::Replicate { count, expr } => {
            Expr::Replicate {
                count: Box::new(substitute_expr_types(*count, param_name, replacement)),
                expr: Box::new(substitute_expr_types(*expr, param_name, replacement)),
            }
        }
        Expr::TernaryOp { cond, true_expr, false_expr } => {
            Expr::TernaryOp {
                cond: Box::new(substitute_expr_types(*cond, param_name, replacement)),
                true_expr: Box::new(substitute_expr_types(*true_expr, param_name, replacement)),
                false_expr: Box::new(substitute_expr_types(*false_expr, param_name, replacement)),
            }
        }
        Expr::FuncCall { name, args } => {
            Expr::FuncCall {
                name,
                args: args.into_iter().map(|a| substitute_expr_types(a, param_name, replacement)).collect(),
            }
        }
        other => other,
    }
}

