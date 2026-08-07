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

use std::collections::{HashMap, HashSet};

use crate::ast::types::const_eval_simple;

const MAX_GENERATED_ITEMS: usize = 1_000_000;
use crate::ast::types::const_eval_with_params;
use crate::ast::*;
use crate::diagnostics::diagnostic::{DiagCode, DiagLevel, Diagnostic, SourceSnippet};
use crate::diagnostics::DiagSink;
use crate::intern::Symbol;

use super::loop_unroll::{
    substitute_loop_var_in_expr, substitute_loop_var_in_stmt, substitute_sensitivity_event,
};

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
pub(crate) fn expr_location(expr: &Expr) -> (usize, usize) {
    match expr {
        Expr::Ident { line, col, .. } => (*line, *col),
        Expr::Value(_) | Expr::FillLit(_) | Expr::String(_) | Expr::Null => (0, 0),
        Expr::FuncCall { line, col, .. } => (*line, *col),
        Expr::MethodCall { args, .. } => {
            args.first().map(expr_location).unwrap_or((0, 0))
        }
        Expr::UnaryOp { expr: inner, .. }
        | Expr::Paren(inner)
        | Expr::BitSelect { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::CastWidth { expr: inner, .. }
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
        Expr::MemberAccess { obj, .. } => {
            // Member access tidak menyimpan line/col sendiri — ambil posisi
            // dari objek paling dalam (chain `a.b.c` → posisi `a`).
            expr_location(obj)
        }
        Expr::ScopedIdent { line, col, .. } => (*line, *col),
    }
}

/// Perluas SEMUA generate block di module dengan nilai parameter tertentu.
/// Block yang gagal diekspansi (mis. limit non-constant karena konstanta
/// package tidak tersedia) menghasilkan error elaborasi.
pub fn expand_all_generates(
    module: &mut Module,
    param_vals: &HashMap<Symbol, i64>,
    diag_sink: &DiagSink,
    source_lines: &[String],
    source_file: &str,
) -> Result<(), ElabError> {
    let mut i = 0;
    let mut total_items = 0usize;
    while i < module.items.len() {
        if let ModuleItem::Generate(gen) = &module.items[i] {
            match expand_generate_block(gen, param_vals, diag_sink, source_lines, source_file) {
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
                    // Generate block yang gagal diekspansi (mis. limit generate-for
                    // merujuk param yang tidak bisa di-eval konstan) → lewati blok
                    // dan lanjutkan. Klasifikasikan Warning: modul tetap di-elaborasi
                    // tanpa blok ini, menghindari skip modul berantai. Info baris
                    // global (combined source) dipertahankan di teks pesan.
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
    source_lines: &[String],
    source_file: &str,
) -> Result<Vec<ModuleItem>, ElabError> {
    let mut result = Vec::new();
    for item in &gen.items {
        match item {
            GenerateItem::If {
                cond,
                true_items,
                false_items,
                ..
            } => {
                let (cond_line, cond_col) = expr_location(cond);
                let eval_result = const_eval_with_params(cond, param_vals);
                match eval_result {
                    Ok(val) => {
                        let branch = if val != 0 { true_items } else { false_items };
                        result.extend(expand_item_list(branch, param_vals, diag_sink, source_lines, source_file)?);
                    }
                    Err(e) => {
                        // Kondisi generate-if yang tidak bisa di-evaluasi konstan
                        // (mis. localparam struct yang belum didukung const-eval,
                        // member access, method call) → ambil true branch sebagai
                        // fallback deterministik. Karena elaborasi LANJUT dengan
                        // true branch, klasifikasikan sebagai Warning (bukan
                        // Error) agar modul tidak di-skip dan memicu error E3001
                        // berantai di seluruh hirarki instance.
                        let mut diag = Diagnostic::new(
                            DiagLevel::Warning,
                            DiagCode::NotImplemented,
                            format!("non-constant condition in generate if ({}), taking true branch", e),
                        );
                        // Posisi kondisi generate di-render sebagai snippet
                        // file:line:col (sebelumnya hanya ditulis di teks pesan).
                        if cond_line > 0 && cond_line <= source_lines.len() {
                            let (file, display_line) =
                                crate::diagnostics::resolve_source_location(source_lines, source_file, cond_line);
                            let snippet = SourceSnippet::new(
                                file,
                                display_line,
                                cond_col,
                                &source_lines[cond_line - 1],
                            );
                            diag = diag.with_source_snippet(snippet);
                        }
                        diag_sink.push(diag);
                        result.extend(expand_item_list(true_items, param_vals, diag_sink, source_lines, source_file)?);
                    }
                }
            }
            GenerateItem::For {
                var,
                init,
                cond,
                step,
                body_items,
                label,
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
                        scope_rename_generate_iteration(&mut substituted, label.as_ref(), cur);
                        result.extend(expand_item_list(&substituted, param_vals, diag_sink, source_lines, source_file)?);
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
                        scope_rename_generate_iteration(&mut substituted, label.as_ref(), cur);
                        result.extend(expand_item_list(&substituted, param_vals, diag_sink, source_lines, source_file)?);
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
                    Err(e) => {
                        // Generate case dengan ekspresi non-constant — ambil cabang
                        // pertama sebagai fallback. Turunkan ke warning (bukan error)
                        // karena ini sering terjadi di interface coverage OpenTitan.
                        diag_sink.push(Diagnostic::new(DiagLevel::Warning, DiagCode::NotImplemented, format!("non-constant expression in generate case at line {}:{} ({}), taking first case", case_line, case_col, e)));
                        if let Some(first) = items.first() {
                            result.extend(expand_item_list(&first.body, param_vals, diag_sink, source_lines, source_file)?);
                        } else if let Some(default_items) = default {
                            result.extend(expand_item_list(default_items, param_vals, diag_sink, source_lines, source_file)?);
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
                            result.extend(expand_item_list(&ci.body, param_vals, diag_sink, source_lines, source_file)?);
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
                        result.extend(expand_item_list(default_items, param_vals, diag_sink, source_lines, source_file)?);
                    }
                }
            }
            GenerateItem::Items(items) => {
                result.extend(expand_item_list(items, param_vals, diag_sink, source_lines, source_file)?);
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
    source_lines: &[String],
    source_file: &str,
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
                result.extend(expand_generate_block(gen, &extended, diag_sink, source_lines, source_file)?);
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
            // Substitusi genvar di sensitivity list `@(sig[k])` juga — jika tidak,
            // `k` tertinggal sebagai Ident dan resolve range sensitivity gagal.
            if let Some(sl) = &mut always.sensitivity {
                for event in &mut sl.events {
                    *event = substitute_sensitivity_event(event, var_name, value);
                }
            }
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
            ..
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
            ..
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

// ─── Scope rename untuk generate block ─────────────────────────────────────
// Anti-collision: sinyal lokal di dalam `for genvar k ... begin : name` harus
// dinamai `name[k].sig` per iterasi, agar dua iterasi tidak berbagi sinyal.

/// Kumpulkan nama sinyal LOKAL yang dideklarasikan dalam item generate.
fn collect_scope_locals(items: &[ModuleItem]) -> HashSet<Symbol> {
    let mut locals = HashSet::new();
    for item in items {
        if let ModuleItem::Decl(decl) = item {
            for var in &decl.names {
                locals.insert(var.name);
            }
        }
    }
    locals
}

/// Rename ident lokal di expr: Ident(old) → Ident(new) sesuai `map`.
fn scope_rename_expr(expr: &Expr, map: &HashMap<Symbol, Symbol>) -> Expr {
    match expr {
        Expr::Ident { name, line, col } => match map.get(name) {
            Some(new) => Expr::Ident {
                name: *new,
                line: *line,
                col: *col,
            },
            None => expr.clone(),
        },
        Expr::Value(_) | Expr::String(_) | Expr::Null | Expr::FillLit(_) => expr.clone(),
        Expr::RangeSelect {
            expr: inner,
            msb,
            lsb,
        } => Expr::RangeSelect {
            expr: Box::new(scope_rename_expr(inner, map)),
            msb: Box::new(scope_rename_expr(msb, map)),
            lsb: Box::new(scope_rename_expr(lsb, map)),
        },
        Expr::BitSelect { expr: inner, index } => Expr::BitSelect {
            expr: Box::new(scope_rename_expr(inner, map)),
            index: Box::new(scope_rename_expr(index, map)),
        },
        Expr::PartSelect {
            expr: inner,
            base,
            width,
        } => Expr::PartSelect {
            expr: Box::new(scope_rename_expr(inner, map)),
            base: Box::new(scope_rename_expr(base, map)),
            width: Box::new(scope_rename_expr(width, map)),
        },
        Expr::Concat(exprs) => Expr::Concat(
            exprs
                .iter()
                .map(|e| scope_rename_expr(e, map))
                .collect(),
        ),
        Expr::FuncCall { name, args, line, col } => Expr::FuncCall {
            name: *name,
            args: args
                .iter()
                .map(|a| scope_rename_expr(a, map))
                .collect(),
            line: *line,
            col: *col,
        },
        Expr::Replicate { count, expr: inner } => Expr::Replicate {
            count: Box::new(scope_rename_expr(count, map)),
            expr: Box::new(scope_rename_expr(inner, map)),
        },
        Expr::UnaryOp { op, expr: inner } => Expr::UnaryOp {
            op: op.clone(),
            expr: Box::new(scope_rename_expr(inner, map)),
        },
        Expr::BinaryOp { op, lhs, rhs } => Expr::BinaryOp {
            op: op.clone(),
            lhs: Box::new(scope_rename_expr(lhs, map)),
            rhs: Box::new(scope_rename_expr(rhs, map)),
        },
        Expr::TernaryOp {
            cond,
            true_expr,
            false_expr,
        } => Expr::TernaryOp {
            cond: Box::new(scope_rename_expr(cond, map)),
            true_expr: Box::new(scope_rename_expr(true_expr, map)),
            false_expr: Box::new(scope_rename_expr(false_expr, map)),
        },
        Expr::Paren(inner) => Expr::Paren(Box::new(scope_rename_expr(inner, map))),
        Expr::MethodCall {
            obj,
            method,
            args,
            with_clause,
        } => Expr::MethodCall {
            obj: Box::new(scope_rename_expr(obj, map)),
            method: *method,
            args: args
                .iter()
                .map(|a| scope_rename_expr(a, map))
                .collect(),
            with_clause: with_clause
                .clone()
                .map(|wc| Box::new(scope_rename_expr(&wc, map))),
        },
        Expr::MemberAccess { obj, field } => Expr::MemberAccess {
            obj: Box::new(scope_rename_expr(obj, map)),
            field: *field,
        },
        Expr::Inside {
            expr: inner,
            range_list,
        } => Expr::Inside {
            expr: Box::new(scope_rename_expr(inner, map)),
            range_list: range_list
                .iter()
                .map(|e| scope_rename_expr(e, map))
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
                .map(|ss| Box::new(scope_rename_expr(ss, map))),
            slices: slices
                .iter()
                .map(|e| scope_rename_expr(e, map))
                .collect(),
        },
        Expr::Dist { expr, items } => Expr::Dist {
            expr: Box::new(scope_rename_expr(expr, map)),
            items: items.clone(),
        },
        Expr::Cast { dtype, expr: inner } => Expr::Cast {
            dtype: *dtype,
            expr: Box::new(scope_rename_expr(inner, map)),
        },
        Expr::CastWidth { width, expr: inner } => Expr::CastWidth {
            width: Box::new(scope_rename_expr(width, map)),
            expr: Box::new(scope_rename_expr(inner, map)),
        },
        Expr::ScopedIdent { package, item, line, col } => Expr::ScopedIdent {
            package: *package,
            item: *item,
            line: *line,
            col: *col,
        },
    }
}

fn scope_rename_sensitivity(
    event: &SensitivityEvent,
    map: &HashMap<Symbol, Symbol>,
) -> SensitivityEvent {
    match event {
        SensitivityEvent::PosEdge(e) => SensitivityEvent::PosEdge(scope_rename_expr(e, map)),
        SensitivityEvent::NegEdge(e) => SensitivityEvent::NegEdge(scope_rename_expr(e, map)),
        SensitivityEvent::Level(e) => SensitivityEvent::Level(scope_rename_expr(e, map)),
        SensitivityEvent::Wildcard => SensitivityEvent::Wildcard,
    }
}

fn scope_rename_stmts(stmts: &[Stmt], map: &HashMap<Symbol, Symbol>) -> Vec<Stmt> {
    stmts.iter().map(|s| scope_rename_stmt(s, map)).collect()
}

fn scope_rename_stmt(stmt: &Stmt, map: &HashMap<Symbol, Symbol>) -> Stmt {
    match stmt {
        Stmt::Block { stmts } => Stmt::Block {
            stmts: scope_rename_stmts(stmts, map),
        },
        Stmt::BlockingAssign { lhs, rhs, delay } => Stmt::BlockingAssign {
            lhs: scope_rename_expr(lhs, map),
            rhs: scope_rename_expr(rhs, map),
            delay: delay.clone(),
        },
        Stmt::NonBlockingAssign { lhs, rhs, delay } => Stmt::NonBlockingAssign {
            lhs: scope_rename_expr(lhs, map),
            rhs: scope_rename_expr(rhs, map),
            delay: delay.clone(),
        },
        Stmt::IfElse {
            cond,
            true_branch,
            false_branch,
        } => Stmt::IfElse {
            cond: scope_rename_expr(cond, map),
            true_branch: Box::new(scope_rename_stmt(true_branch, map)),
            false_branch: false_branch
                .as_ref()
                .map(|fb| Box::new(scope_rename_stmt(fb, map))),
        },
        Stmt::Case {
            expr,
            items,
            default,
        } => Stmt::Case {
            expr: scope_rename_expr(expr, map),
            items: items
                .iter()
                .map(|item| crate::ast::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| scope_rename_expr(l, map))
                        .collect(),
                    stmt: Box::new(scope_rename_stmt(&item.stmt, map)),
                })
                .collect(),
            default: default
                .as_ref()
                .map(|d| Box::new(scope_rename_stmt(d, map))),
        },
        Stmt::StmtAssign { lhs, rhs } => Stmt::StmtAssign {
            lhs: scope_rename_expr(lhs, map),
            rhs: scope_rename_expr(rhs, map),
        },
        Stmt::Delay { delay, stmt } => Stmt::Delay {
            delay: scope_rename_expr(delay, map),
            stmt: Box::new(scope_rename_stmt(stmt, map)),
        },
        Stmt::SysCall { name, args } => Stmt::SysCall {
            name: *name,
            args: args
                .iter()
                .map(|a| scope_rename_expr(a, map))
                .collect(),
        },
        Stmt::Expr { expr } => Stmt::Expr {
            expr: scope_rename_expr(expr, map),
        },
        Stmt::CaseX {
            expr,
            items,
            default,
        } => Stmt::CaseX {
            expr: scope_rename_expr(expr, map),
            items: items
                .iter()
                .map(|item| crate::ast::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| scope_rename_expr(l, map))
                        .collect(),
                    stmt: Box::new(scope_rename_stmt(&item.stmt, map)),
                })
                .collect(),
            default: default
                .as_ref()
                .map(|d| Box::new(scope_rename_stmt(d, map))),
        },
        Stmt::CaseZ {
            expr,
            items,
            default,
        } => Stmt::CaseZ {
            expr: scope_rename_expr(expr, map),
            items: items
                .iter()
                .map(|item| crate::ast::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| scope_rename_expr(l, map))
                        .collect(),
                    stmt: Box::new(scope_rename_stmt(&item.stmt, map)),
                })
                .collect(),
            default: default
                .as_ref()
                .map(|d| Box::new(scope_rename_stmt(d, map))),
        },
        Stmt::StmtCase {
            expr,
            items,
            default,
        } => Stmt::StmtCase {
            expr: scope_rename_expr(expr, map),
            items: items
                .iter()
                .map(|item| crate::ast::stmt::CaseItem {
                    labels: item
                        .labels
                        .iter()
                        .map(|l| scope_rename_expr(l, map))
                        .collect(),
                    stmt: Box::new(scope_rename_stmt(&item.stmt, map)),
                })
                .collect(),
            default: default
                .as_ref()
                .map(|d| Box::new(scope_rename_stmt(d, map))),
        },
        Stmt::LoopForever { stmts } => Stmt::LoopForever {
            stmts: scope_rename_stmts(stmts, map),
        },
        Stmt::LoopWhile { cond, stmts } => Stmt::LoopWhile {
            cond: scope_rename_expr(cond, map),
            stmts: scope_rename_stmts(stmts, map),
        },
        Stmt::LoopFor {
            init,
            cond,
            step,
            stmts,
        } => Stmt::LoopFor {
            init: init
                .as_ref()
                .map(|s| Box::new(scope_rename_stmt(s, map))),
            cond: cond.as_ref().map(|c| scope_rename_expr(c, map)),
            step: step
                .as_ref()
                .map(|s| Box::new(scope_rename_stmt(s, map))),
            stmts: scope_rename_stmts(stmts, map),
        },
        Stmt::Repeat { count, stmts } => Stmt::Repeat {
            count: scope_rename_expr(count, map),
            stmts: scope_rename_stmts(stmts, map),
        },
        Stmt::Wait { cond, stmt } => Stmt::Wait {
            cond: scope_rename_expr(cond, map),
            stmt: stmt
                .as_ref()
                .map(|s| Box::new(scope_rename_stmt(s, map))),
        },
        Stmt::Disable { name } => Stmt::Disable { name: *name },
        Stmt::Force { lhs, rhs } => Stmt::Force {
            lhs: scope_rename_expr(lhs, map),
            rhs: scope_rename_expr(rhs, map),
        },
        Stmt::Release { expr } => Stmt::Release {
            expr: scope_rename_expr(expr, map),
        },
        Stmt::Deassign { expr } => Stmt::Deassign {
            expr: scope_rename_expr(expr, map),
        },
        Stmt::Return(expr) => Stmt::Return(
            expr.as_ref()
                .map(|e| Box::new(scope_rename_expr(e, map))),
        ),
        Stmt::Null => Stmt::Null,
        Stmt::SysFinish => Stmt::SysFinish,
        Stmt::EventControl { events, stmt } => Stmt::EventControl {
            events: events
                .iter()
                .map(|e| scope_rename_sensitivity(e, map))
                .collect(),
            stmt: stmt
                .as_ref()
                .map(|s| Box::new(scope_rename_stmt(s, map))),
        },
        Stmt::EventTrigger { name } => Stmt::EventTrigger { name: *name },
        Stmt::ForeachLoop {
            array_var,
            index_vars,
            stmts,
        } => Stmt::ForeachLoop {
            array_var: *array_var,
            index_vars: index_vars.clone(),
            stmts: scope_rename_stmts(stmts, map),
        },
        Stmt::NamedBlock {
            name,
            stmts,
            decls,
        } => Stmt::NamedBlock {
            name: *name,
            stmts: scope_rename_stmts(stmts, map),
            decls: decls.clone(),
        },
        Stmt::RandCase { items } => Stmt::RandCase {
            items: items
                .iter()
                .map(|rc| crate::ast::stmt::RandCaseItem {
                    weight: rc.weight,
                    stmt: Box::new(scope_rename_stmt(&rc.stmt, map)),
                })
                .collect(),
        },
        Stmt::Fork { processes, join_type } => Stmt::Fork {
            processes: processes
                .iter()
                .map(|p| scope_rename_stmt(p, map))
                .collect(),
            join_type: join_type.clone(),
        },
        Stmt::Break => Stmt::Break,
        Stmt::Continue => Stmt::Continue,
        Stmt::DoWhile { cond, stmts } => Stmt::DoWhile {
            cond: scope_rename_expr(cond, map),
            stmts: scope_rename_stmts(stmts, map),
        },
        _ => stmt.clone(),
    }
}

/// Rename sinyal lokal di module item sesuai `map`.
fn scope_rename_module_item(item: &mut ModuleItem, map: &HashMap<Symbol, Symbol>) {
    match item {
        ModuleItem::Always(always) => {
            for stmt in &mut always.stmts {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = scope_rename_stmt(&old, map);
            }
        }
        ModuleItem::Initial(initial) => {
            for stmt in &mut initial.stmts {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = scope_rename_stmt(&old, map);
            }
        }
        ModuleItem::Final(final_block) => {
            for stmt in &mut final_block.stmts {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = scope_rename_stmt(&old, map);
            }
        }
        ModuleItem::Assign(assign) => {
            assign.lhs = scope_rename_expr(&assign.lhs, map);
            assign.rhs = scope_rename_expr(&assign.rhs, map);
        }
        ModuleItem::Instance(inst) => {
            if let Some(range) = &mut inst.range {
                range.msb = scope_rename_expr(&range.msb, map);
                range.lsb = scope_rename_expr(&range.lsb, map);
            }
            for expr in inst.param_assigns.values_mut() {
                let old = std::mem::replace(expr, Expr::Value(crate::ast::expr::Value::Decimal(0)));
                *expr = scope_rename_expr(&old, map);
            }
            for conn in &mut inst.port_conns {
                match conn {
                    PortConnection::Positional(expr) => {
                        let old = std::mem::replace(
                            expr,
                            Expr::Value(crate::ast::expr::Value::Decimal(0)),
                        );
                        *expr = scope_rename_expr(&old, map);
                    }
                    PortConnection::Named { expr, .. } => {
                        let old = std::mem::replace(
                            expr,
                            Expr::Value(crate::ast::expr::Value::Decimal(0)),
                        );
                        *expr = scope_rename_expr(&old, map);
                    }
                }
            }
        }
        ModuleItem::Decl(decl) => {
            for var in &mut decl.names {
                if let Some(new) = map.get(&var.name) {
                    var.name = *new;
                }
                if let Some(er) = &mut var.expr_range {
                    er.msb = scope_rename_expr(&er.msb, map);
                    er.lsb = scope_rename_expr(&er.lsb, map);
                }
                if let Some(init) = &mut var.expr {
                    let old = std::mem::replace(init, Expr::Value(crate::ast::expr::Value::Decimal(0)));
                    *init = scope_rename_expr(&old, map);
                }
            }
        }
        ModuleItem::Func(func) => {
            for stmt in &mut func.stmts {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = scope_rename_stmt(&old, map);
            }
        }
        ModuleItem::Generate(gen) => {
            for gi in &mut gen.items {
                scope_rename_generate_item(gi, map);
            }
        }
        _ => {}
    }
}

/// Rename sinyal lokal di dalam GenerateItem (rekursif) — dipakai scope-rename
/// iterasi generate for BERSARANG: deklarasi lokal `mubi_out` di body outer
/// loop menjadi `label[k].mubi_out`, dan referensinya di dalam nested generate
/// (mis. `mubi_out[i]` di body `for genvar k`) juga harus ikut di-rename.
/// Loop var nested tidak di-rename (hanya sinyal yang di-declararasikan).
fn scope_rename_generate_item(item: &mut GenerateItem, map: &HashMap<Symbol, Symbol>) {
    match item {
        GenerateItem::If {
            cond,
            true_items,
            false_items,
            ..
        } => {
            *cond = scope_rename_expr(cond, map);
            for i in true_items.iter_mut() {
                scope_rename_module_item(i, map);
            }
            for i in false_items.iter_mut() {
                scope_rename_module_item(i, map);
            }
        }
        GenerateItem::For {
            var: _,
            init,
            cond,
            step,
            body_items,
            ..
        } => {
            if let Some(stmt) = init {
                *stmt = scope_rename_stmt(stmt, map);
            }
            if let Some(e) = cond {
                *e = scope_rename_expr(e, map);
            }
            if let Some(stmt) = step {
                *stmt = scope_rename_stmt(stmt, map);
            }
            for i in body_items.iter_mut() {
                scope_rename_module_item(i, map);
            }
        }
        GenerateItem::Case {
            expr,
            items,
            default,
            ..
        } => {
            *expr = scope_rename_expr(expr, map);
            for ci in items.iter_mut() {
                for label in ci.labels.iter_mut() {
                    *label = scope_rename_expr(label, map);
                }
                for i in ci.body.iter_mut() {
                    scope_rename_module_item(i, map);
                }
            }
            if let Some(d) = default {
                for i in d.iter_mut() {
                    scope_rename_module_item(i, map);
                }
            }
        }
        GenerateItem::Items(items) => {
            for i in items.iter_mut() {
                scope_rename_module_item(i, map);
            }
        }
    }
}

/// Terapkan scope-rename untuk SATU iterasi generate for.
/// Sinyal lokal `sig` dinamai `label[cur].sig` (atau `genblk[cur].sig` tanpa
/// label) agar tidak collide antar iterasi.
fn scope_rename_generate_iteration(items: &mut [ModuleItem], label: Option<&Symbol>, cur: i64) {
    let locals = collect_scope_locals(items);
    if locals.is_empty() {
        return;
    }
    let scope_name = match label {
        Some(l) => format!("{}[{}]", l.as_str(), cur),
        None => format!("genblk[{}]", cur),
    };
    let mut map = HashMap::with_capacity(locals.len());
    for l in &locals {
        map.insert(*l, Symbol::intern(&format!("{}.{}", scope_name, l.as_str())));
    }
    for item in items.iter_mut() {
        scope_rename_module_item(item, &map);
    }
}
