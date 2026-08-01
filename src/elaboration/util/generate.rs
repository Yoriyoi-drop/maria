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
//!
//! ──────────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;

use crate::ast::types::const_eval_simple;

const MAX_GENERATED_ITEMS: usize = 1_000_000;
use crate::ast::types::const_eval_with_params;
use crate::ast::*;
use crate::diagnostics::diagnostic::{DiagCode, DiagLevel, Diagnostic};
use crate::diagnostics::DiagSink;
use crate::intern::Symbol;

use super::loop_unroll::{substitute_loop_var_in_expr, substitute_loop_var_in_stmt};

/// Structured error untuk elaboration/generate expansion dengan source location.
#[derive(Debug, Clone)]
pub struct ElabError {
    pub msg: String,
    pub line: usize,
    pub col: usize,
}

impl ElabError {
    pub fn new(msg: impl Into<String>, line: usize, col: usize) -> Self {
        ElabError { msg: msg.into(), line, col }
    }
}

impl std::fmt::Display for ElabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl std::error::Error for ElabError {}

impl From<ElabError> for crate::error::SimError {
    fn from(e: ElabError) -> Self {
        let msg = if e.line > 0 {
            format!("{} (at line {}:{})", e.msg, e.line, e.col)
        } else {
            e.msg
        };
        let diag = crate::diagnostics::diagnostic::Diagnostic::error(
            crate::diagnostics::diagnostic::DiagCode::InvalidSyntax,
            msg,
        )
        .with_code_context();
        crate::error::SimError::Diagnostic(diag)
    }
}

/// Extract the best available source location from an expression.
fn expr_location(expr: &Expr) -> (usize, usize) {
    match expr {
        Expr::Ident { line, col, .. } => (*line, *col),
        Expr::Value(_) | Expr::FillLit(_) | Expr::String(_) | Expr::Null => (0, 0),
        Expr::FuncCall { args, .. }
        | Expr::MethodCall { args, .. } => {
            args.first().map(expr_location).unwrap_or((0, 0))
        }
        Expr::UnaryOp { expr: inner, .. }
        | Expr::Paren(inner)
        | Expr::BitSelect { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::Dist { expr: inner, .. } => expr_location(inner),
        Expr::BinaryOp { lhs, rhs, .. } => {
            let (ll, lc) = expr_location(lhs);
            if ll > 0 || lc > 0 { (ll, lc) } else { expr_location(rhs) }
        }
        Expr::RangeSelect { expr: lhs, .. }
        | Expr::PartSelect { expr: lhs, .. } => expr_location(lhs),
        Expr::TernaryOp { cond, .. } => expr_location(cond),
        Expr::Inside { expr: inner, .. } => expr_location(inner),
        Expr::Concat(items) | Expr::StreamingConcat { slices: items, .. } => {
            items.first().map(expr_location).unwrap_or((0, 0))
        }
        Expr::Replicate { expr: inner, .. } => expr_location(inner),
        Expr::ScopedIdent { .. } | Expr::MemberAccess { .. } => (0, 0),
    }
}

/// Perluas SEMUA generate block di module dengan nilai parameter tertentu.
/// Block yang gagal diekspansi (mis. limit non-constant karena konstanta
/// package tidak tersedia) dilewati dengan warning, bukan mematikan elaborasi.
pub fn expand_all_generates(
    module: &mut Module,
    param_vals: &HashMap<Symbol, i64>,
    diag_sink: &DiagSink,
) -> Result<(), ElabError> {
    let mut i = 0;
    let mut total_items = 0usize;
    while i < module.items.len() {
        if let ModuleItem::Generate(gen) = &module.items[i] {
            match expand_generate_block(gen, param_vals, diag_sink) {
                Ok(expanded) => {
                    total_items += expanded.len();
                    if total_items > MAX_GENERATED_ITEMS {
                        return Err(ElabError::new(
                            format!("generate expansion exceeded limit ({} items)", MAX_GENERATED_ITEMS),
                            0, 0
                        ));
                    }
                    for item in &expanded {
                        if let ModuleItem::Decl(d) = item {
                            module.decls.push(d.clone());
                        }
                    }
                    module.items.splice(i..=i, expanded);
                }
                Err(e) => {
                    diag_sink.push(Diagnostic::new(
                        DiagLevel::Warning,
                        DiagCode::NotImplemented,
                        format!(
                            "generate block expansion skipped in '{}': {} (at line {}:{})",
                            module.name.as_str(),
                            e.msg,
                            e.line,
                            e.col
                        ),
                    ));
                    i += 1;
                }
            }
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
    diag_sink: &DiagSink,
) -> Result<Vec<ModuleItem>, ElabError> {
    let mut result = Vec::new();
    for item in &gen.items {
        match item {
            GenerateItem::If {
                cond,
                true_items,
                false_items,
            } => {
                let (cond_line, cond_col) = expr_location(cond);
                let eval_result = const_eval_with_params(cond, param_vals);
                match eval_result {
                    Ok(val) => {
                        let branch = if val != 0 { true_items } else { false_items };
                        result.extend(expand_item_list(branch, param_vals, diag_sink)?);
                    }
                    Err(_) => {
                        diag_sink.push(Diagnostic::new(DiagLevel::Warning, DiagCode::NotImplemented, "non-constant condition in generate if, taking true branch"));
                        result.extend(expand_item_list(true_items, param_vals, diag_sink)?);
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
                if std::env::var("DBG_GEN").is_ok() {
                    eprintln!("DBG-GEN: for var={} init={:?} cond={:?} step={:?}", var.as_str(), init, cond, step);
                }
                let (init_line, init_col) = init.as_ref()
                    .and_then(|s| match s { Stmt::BlockingAssign { rhs, .. } => Some(expr_location(rhs)), _ => None })
                    .unwrap_or((0, 0));
                let start_val: i64 = match init {
                    Some(Stmt::BlockingAssign { rhs, .. }) => {
                        const_eval_with_params(rhs, param_vals)
                            .map_err(|e| ElabError::new(format!("generate for init eval failed: {}", e), init_line, init_col))?
                    }
                    _ => 0,
                };
                let (lim_line, lim_col) = cond.as_ref().map(expr_location).unwrap_or((0, 0));
                let limit: i64 = match cond {
                    Some(Expr::BinaryOp {
                        op: BinaryOp::Lt,
                        rhs,
                        ..
                    }) => {
                        let (rhs_line, rhs_col) = expr_location(rhs);
                        let loc_line = if rhs_line > 0 { rhs_line } else { lim_line };
                        let loc_col = if rhs_col > 0 { rhs_col } else { lim_col };
                        const_eval_with_params(rhs, param_vals)
                            .map_err(|e| ElabError::new(format!("generate for limit eval failed: {}", e), loc_line, loc_col))?
                    },
                    Some(Expr::BinaryOp {
                        op: BinaryOp::Le,
                        rhs,
                        ..
                    }) => {
                        let (rhs_line, rhs_col) = expr_location(rhs);
                        let loc_line = if rhs_line > 0 { rhs_line } else { lim_line };
                        let loc_col = if rhs_col > 0 { rhs_col } else { lim_col };
                        const_eval_with_params(rhs, param_vals)
                            .map_err(|e| ElabError::new(format!("generate for limit eval failed: {}", e), loc_line, loc_col))? + 1
                    },
                    Some(c) => {
                        let (c_line, c_col) = expr_location(c);
                        let loc_line = if c_line > 0 { c_line } else { lim_line };
                        let loc_col = if c_col > 0 { c_col } else { lim_col };
                        if const_eval_with_params(c, param_vals)
                            .map_err(|e| ElabError::new(format!("generate for condition eval failed: {}", e), loc_line, loc_col))? != 0
                        {
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
                            return Err(ElabError::new(
                                format!("generate for loop exceeded {} iterations (possible O(2^N) blowup)", max_iter),
                                lim_line, lim_col
                            ));
                        }
                        iter += 1;
                        let mut substituted: Vec<ModuleItem> = body_items.clone();
                        for item in &mut substituted {
                            substitute_genvar_in_module_item(item, var.as_str(), cur);
                        }
                        result.extend(expand_item_list(&substituted, param_vals, diag_sink)?);
                        cur += step_val;
                    }
                } else if step_val < 0 {
                    let mut cur = start_val;
                    let mut iter = 0usize;
                    while cur > limit {
                        if iter >= max_iter {
                            return Err(ElabError::new(
                                format!("generate for loop exceeded {} iterations (possible O(2^N) blowup)", max_iter),
                                lim_line, lim_col
                            ));
                        }
                        iter += 1;
                        let mut substituted: Vec<ModuleItem> = body_items.clone();
                        for item in &mut substituted {
                            substitute_genvar_in_module_item(item, var.as_str(), cur);
                        }
                        result.extend(expand_item_list(&substituted, param_vals, diag_sink)?);
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
                let (case_line, case_col) = expr_location(expr);
                let case_val = match const_eval_with_params(expr, param_vals) {
                    Ok(v) => v,
                    Err(_) => {
                        diag_sink.push(Diagnostic::new(DiagLevel::Warning, DiagCode::NotImplemented, format!("non-constant expression in generate case at line {}:{}, taking first case", case_line, case_col)));
                        if let Some(first) = items.first() {
                            result.extend(expand_item_list(&first.body, param_vals, diag_sink)?);
                        } else if let Some(default_items) = default {
                            result.extend(expand_item_list(default_items, param_vals, diag_sink)?);
                        }
                        continue;
                    }
                };
                let mut matched = false;
                for ci in items {
                    for label in &ci.labels {
                        let (lab_line, lab_col) = expr_location(label);
                        let label_val = const_eval_with_params(label, param_vals)
                            .map_err(|e| ElabError::new(format!("generate case label eval failed: {}", e), lab_line, lab_col))?;
                        if label_val == case_val {
                            result.extend(expand_item_list(&ci.body, param_vals, diag_sink)?);
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
                        result.extend(expand_item_list(default_items, param_vals, diag_sink)?);
                    }
                }
            }
            GenerateItem::Items(items) => {
                result.extend(expand_item_list(items, param_vals, diag_sink)?);
            }
        }
    }
    Ok(result)
}

/// Perluas daftar module items: evaluasi localparam yang didefinisikan di dalamnya
/// (dipakai oleh generate sibling), lalu perluas generate block bersarang secara
/// rekursif dengan param context yang diperpanjang.
fn expand_item_list(
    items: &[ModuleItem],
    param_vals: &HashMap<Symbol, i64>,
    diag_sink: &DiagSink,
) -> Result<Vec<ModuleItem>, ElabError> {
    let mut extended = param_vals.clone();
    for item in items {
        if let ModuleItem::Param(p) = item {
            if !extended.contains_key(&p.name) {
                if let Some(e) = &p.default {
                    if std::env::var("DBG_GEN").is_ok() {
                        eprintln!("DBG-GEN: param {} default = {:?}", p.name.as_str(), e);
                    }
                    if let Ok(v) = const_eval_with_params(e, &extended) {
                        extended.insert(p.name, v);
                    }
                }
            }
        }
    }
    let mut result = Vec::new();
    for item in items {
        match item {
            ModuleItem::Generate(gen) => {
                result.extend(expand_generate_block(gen, &extended, diag_sink)?);
            }
            other => result.push(other.clone()),
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
            for expr in inst.param_assigns.values_mut() {
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
        ModuleItem::Param(p) => {
            if let Some(default) = &mut p.default {
                let old = std::mem::replace(
                    default,
                    Expr::Value(crate::ast::expr::Value::Decimal(0)),
                );
                *default = substitute_loop_var_in_expr(&old, var_name, value);
            }
            if let Some((msb, lsb)) = &mut p.range {
                let old_msb = std::mem::replace(msb, Expr::Value(crate::ast::expr::Value::Decimal(0)));
                let old_lsb = std::mem::replace(lsb, Expr::Value(crate::ast::expr::Value::Decimal(0)));
                *msb = substitute_loop_var_in_expr(&old_msb, var_name, value);
                *lsb = substitute_loop_var_in_expr(&old_lsb, var_name, value);
            }
        }
        ModuleItem::Func(_)
        | ModuleItem::Typedef(_)
        | ModuleItem::Import { .. }
        | ModuleItem::Covergroup(_)
        | ModuleItem::DpiImport(_)
        | ModuleItem::DpiExport(_)
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
