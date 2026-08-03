use std::collections::HashMap;
use super::Elaborator;
use super::super::util::*;
use crate::ast::types::{const_eval_simple, const_eval_with_params};
use crate::ast::*;
use crate::diagnostics::diagnostic::DiagCode;
use crate::error::SimError;
use crate::intern::Symbol;
use crate::ir::*;

/// Extract SignalId from IrLValue, if it's a simple signal reference.
fn lvalue_signal_id(lv: &IrLValue) -> Option<SignalId> {
    match lv {
        IrLValue::Signal(id, _) => Some(*id),
        IrLValue::RangeSelect(id, _, _) => Some(*id),
        IrLValue::BitSelect(id, _) => Some(*id),
        IrLValue::ArrayIndex { sig_id, .. } => Some(*sig_id),
        IrLValue::ArrayRangeSelect { sig_id, .. } => Some(*sig_id),
        IrLValue::ArrayBitSelect { sig_id, .. } => Some(*sig_id),
        IrLValue::ObjectField { sig_id, .. } => Some(*sig_id),
        IrLValue::Concat(items) => items.first().and_then(lvalue_signal_id),
    }
}

/// Compute approximate width of an IrExpr at elaboration time (best-effort).
fn expr_approx_width(expr: &IrExpr, signals: &[SignalInfo]) -> usize {
    match expr {
        IrExpr::Const(lv) => lv.width,
        IrExpr::FillLit(_) => 1,
        IrExpr::Signal(id, _) => signals.get(*id).map(|s| s.width).unwrap_or(1),
        IrExpr::RangeSelect(_, hi, lo) => hi.saturating_sub(*lo).saturating_add(1),
        IrExpr::BitSelect(_, _) => 1,
        IrExpr::ExprRangeSelect(_, hi, lo) => hi.saturating_sub(*lo).saturating_add(1),
        IrExpr::ExprBitSelect(_, _) => 1,
        IrExpr::ArrayIndex { elem_width, .. } => *elem_width,
        IrExpr::Concat(items) => items.iter().map(|e| expr_approx_width(e, signals)).sum(),
        IrExpr::Replicate(n, inner) => n * expr_approx_width(inner, signals),
        IrExpr::UnaryOp(_, inner) => expr_approx_width(inner, signals),
        IrExpr::BinaryOp(_, a, b) => {
            let wa = expr_approx_width(a, signals);
            let wb = expr_approx_width(b, signals);
            wa.max(wb)
        }
        IrExpr::Cond(_, a, b) => {
            let wa = expr_approx_width(a, signals);
            let wb = expr_approx_width(b, signals);
            wa.max(wb)
        }
        IrExpr::Signed(inner) => expr_approx_width(inner, signals),
        IrExpr::Cast { width, .. } => *width,
        IrExpr::String(s) => s.len() * 8,
        IrExpr::DpiCall { return_width, .. } => *return_width,
        IrExpr::StreamingConcat { slices, .. } => {
            slices.iter().map(|e| expr_approx_width(e, signals)).sum()
        }
        _ => 1,
    }
}

/// Check signedness mismatch between LHS and RHS at elaboration.
fn check_signed_mismatch(
    lhs_signal_id: Option<SignalId>,
    rhs: &IrExpr,
    signals: &[SignalInfo],
) {
    let Some(sid) = lhs_signal_id else { return };
    let Some(lhs_sig) = signals.get(sid) else { return };
    let is_rhs_signed = matches!(rhs, IrExpr::Signed(_));
    if lhs_sig.is_signed && !is_rhs_signed {
        // Only warn when RHS could be determined at compile time
    }
}

impl Elaborator {
    /// Check width mismatch between LHS signal and RHS expression at elaboration.
    fn check_width_mismatch(
        &self,
        lhs_signal_id: Option<SignalId>,
        rhs: &IrExpr,
        signals: &[SignalInfo],
        line: usize,
        col: usize,
    ) {
        let Some(sid) = lhs_signal_id else { return };
        let Some(lhs_sig) = signals.get(sid) else { return };
        let lhs_w = lhs_sig.width;
        let rhs_w = expr_approx_width(rhs, signals);
        if lhs_w != rhs_w && rhs_w > 0 {
            // Jangan warning bila RHS adalah konstanta hasil const-fold yang
            // nilainya muat di lebar LHS (mis. `result = COEFFS[2]` di mana
            // COEFFS[2] = 3 dalam reg [7:0]). Konstanta fold default 32-bit
            // sehingga tanpa cek nilai akan memicu false-positive.
            if let IrExpr::Const(lv) = rhs {
                let cw = lv.width.min(64);
                let cw_mask = if cw >= 64 { u64::MAX } else { (1u64 << cw) - 1 };
                let raw = lv.to_u64() & cw_mask;
                let max_val = if lhs_w >= 64 { u64::MAX } else { (1u64 << lhs_w) - 1 };
                if raw <= max_val {
                    return;
                }
                // Konstanta negatif (two's complement): nilai low bits bila
                // di-sign-extend kembali menghasilkan nilai asli yang sama
                // (mis. -1 = 0xFFFFFFFF muat di reg signed [7:0]).
                if lhs_sig.is_signed && lhs_w > 0 && lhs_w < 64 {
                    let low = raw & max_val;
                    let sign_bit = 1u64 << (lhs_w - 1);
                    if low & sign_bit != 0 {
                        let sign_ext = (low | !max_val) & cw_mask;
                        if sign_ext == raw {
                            return;
                        }
                    }
                }
            }
            self.elab_warn_at(
                DiagCode::WidthMismatchWarning,
                format!("width mismatch in assignment to '{}' (lhs={}, rhs={})", lhs_sig.name, lhs_w, rhs_w),
                line,
                col,
            );
        }
    }

    pub(crate) fn elaborate_stmt_block(
        &self,
        stmts: &[Stmt],
        signal_map: &HashMap<Symbol, SignalId>,
        _known_modules: &[Symbol],
        signals: &[SignalInfo],
    ) -> Result<Vec<IrStmt>, SimError> {
        let mut ir_stmts = Vec::new();
        for stmt in stmts {
            ir_stmts.push(self.elaborate_stmt(stmt, signal_map, _known_modules, signals)?);
        }
        Ok(ir_stmts)
    }

    fn elaborate_stmt(
        &self,
        stmt: &Stmt,
        signal_map: &HashMap<Symbol, SignalId>,
        known_modules: &[Symbol],
        signals: &[SignalInfo],
    ) -> Result<IrStmt, SimError> {
        match stmt {
            Stmt::Block { stmts } => {
                let body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::Block { stmts: body })
            }
            Stmt::BlockingAssign { lhs, rhs, .. } => {
                let ir_lhs = self.elaborate_lvalue(lhs, signal_map, signals)?;
                // Check if LHS is a virtual interface signal
                let is_vif_lhs = match &ir_lhs {
                    IrLValue::Signal(sid, _) => signals
                        .get(*sid)
                        .map(|s| s.iface_type.is_some())
                        .unwrap_or(false),
                    _ => false,
                };
                let mut ir_rhs = if is_vif_lhs {
                    // For vif binding, RHS might be an instance name (not a signal)
                    match rhs {
                        Expr::Ident { name, .. } if !signal_map.contains_key(name) => {
                            // Vif binding: store instance name for runtime resolution
                            IrExpr::VifBinding {
                                instance_name: *name,
                            }
                        }
                        _ => self.elaborate_expr(rhs, signal_map, signals)?,
                    }
                } else {
                    self.elaborate_expr(rhs, signal_map, signals)?
                };
                // Fill in class name for new() calls from LHS signal info
                if let IrExpr::NewCall {
                    ref mut class_name, ..
                } = ir_rhs
                {
                    if class_name.is_empty() {
                        if let IrLValue::Signal(sid, _) = ir_lhs {
                            if let Some(sig) = signals.get(sid) {
                                if let Some(cn) = &sig.class_name {
                                    *class_name = *cn;
                                }
                            }
                        }
                    }
                }
                let lhs_sid = lvalue_signal_id(&ir_lhs);
                let (lhs_line, lhs_col) = expr_location(lhs);
                self.check_width_mismatch(lhs_sid, &ir_rhs, signals, lhs_line, lhs_col);
                check_signed_mismatch(lhs_sid, &ir_rhs, signals);
                Ok(IrStmt::BlockingAssign {
                    lhs: ir_lhs,
                    rhs: ir_rhs,
                    delay: None,
                })
            }
            Stmt::NonBlockingAssign { lhs, rhs, .. } => {
                let ir_lhs = self.elaborate_lvalue(lhs, signal_map, signals)?;
                let ir_is_vif = match &ir_lhs {
                    IrLValue::Signal(sid, _) => signals
                        .get(*sid)
                        .map(|s| s.iface_type.is_some())
                        .unwrap_or(false),
                    _ => false,
                };
                let mut ir_rhs = if ir_is_vif {
                    match rhs {
                        Expr::Ident { name, .. } if !signal_map.contains_key(name) => IrExpr::VifBinding {
                            instance_name: *name,
                        },
                        _ => self.elaborate_expr(rhs, signal_map, signals)?,
                    }
                } else {
                    self.elaborate_expr(rhs, signal_map, signals)?
                };
                if let IrExpr::NewCall {
                    ref mut class_name, ..
                } = ir_rhs
                {
                    if class_name.is_empty() {
                        if let IrLValue::Signal(sid, _) = ir_lhs {
                            if let Some(sig) = signals.get(sid) {
                                if let Some(cn) = &sig.class_name {
                                    *class_name = *cn;
                                }
                            }
                        }
                    }
                }
                let lhs_sid = lvalue_signal_id(&ir_lhs);
                let (lhs_line, lhs_col) = expr_location(lhs);
                self.check_width_mismatch(lhs_sid, &ir_rhs, signals, lhs_line, lhs_col);
                check_signed_mismatch(lhs_sid, &ir_rhs, signals);
                Ok(IrStmt::NonBlockingAssign {
                    lhs: ir_lhs,
                    rhs: ir_rhs,
                    delay: None,
                })
            }
            Stmt::IfElse {
                cond,
                true_branch,
                false_branch,
            } => {
                // Constant-fold condition — if known at compile time, eliminate dead branch
                if let Ok(val) = const_eval_with_params(cond, &self.param_vals) {
                    if val != 0 {
                        // Condition is always true — keep only true branch
                        Ok(self.elaborate_stmt(true_branch, signal_map, known_modules, signals)?)
                    } else {
                        // Condition is always false — keep only false branch
                        match false_branch {
                            Some(fb) => self.elaborate_stmt(fb, signal_map, known_modules, signals),
                            None => Ok(IrStmt::Block { stmts: vec![] }),
                        }
                    }
                } else {
                    let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                    let true_stmt = vec![self.elaborate_stmt(
                        true_branch,
                        signal_map,
                        known_modules,
                        signals,
                    )?];
                    let false_stmt = match false_branch {
                        Some(fb) => {
                            vec![self.elaborate_stmt(fb, signal_map, known_modules, signals)?]
                        }
                        None => vec![],
                    };
                    Ok(IrStmt::If {
                        cond: ir_cond,
                        true_branch: true_stmt,
                        false_branch: false_stmt,
                    })
                }
            }
            Stmt::Case {
                expr,
                items,
                default,
            } => {
                // Try constant-fold the case expression
                if let Ok(case_val) = const_eval_with_params(expr, &self.param_vals) {
                    // Case expression is compile-time constant — find matching branch
                    let mut matched_body: Option<&Stmt> = None;
                    for item in items {
                        for label in &item.labels {
                            let label_val = const_eval_with_params(label, &self.param_vals);
                            if let Ok(lv) = label_val {
                                if lv == case_val {
                                    matched_body = Some(&item.stmt);
                                    break;
                                }
                            } else if let Expr::Value(v) = label {
                                let lv = match v {
                                    Value::Decimal(d) => *d,
                                    Value::Hex { bits, .. } => i64::from_str_radix(
                                        bits.trim_start_matches("0x").trim_start_matches("0X"),
                                        16,
                                    )
                                    .unwrap_or(0),
                                    Value::Binary { bits, .. } => i64::from_str_radix(
                                        bits.trim_start_matches("0b").trim_start_matches("0B"),
                                        2,
                                    )
                                    .unwrap_or(0),
                                    Value::Octal { bits, .. } => i64::from_str_radix(
                                        bits.trim_start_matches("0o").trim_start_matches("0O"),
                                        8,
                                    )
                                    .unwrap_or(0),
                                    Value::Real(_) => 0,
                                };
                                if lv == case_val {
                                    matched_body = Some(&item.stmt);
                                    break;
                                }
                            }
                        }
                        if matched_body.is_some() {
                            break;
                        }
                    }
                    match matched_body {
                        Some(body) => self.elaborate_stmt(body, signal_map, known_modules, signals),
                        None => {
                            if let Some(def) = default {
                                self.elaborate_stmt(def, signal_map, known_modules, signals)
                            } else {
                                Ok(IrStmt::Block { stmts: vec![] })
                            }
                        }
                    }
                } else {
                    let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                    let mut ir_items = Vec::new();
                    for item in items {
                        let mut labels = Vec::new();
                        for label in &item.labels {
                            labels.push(self.elaborate_expr(label, signal_map, signals)?);
                        }
                        let body = match &*item.stmt {
                            Stmt::Block { stmts } => self.elaborate_stmt_block(
                                stmts,
                                signal_map,
                                known_modules,
                                signals,
                            )?,
                            other => self.elaborate_stmt_block(
                                std::slice::from_ref(other),
                                signal_map,
                                known_modules,
                                signals,
                            )?,
                        };
                        ir_items.push(IrCaseItem { labels, body });
                    }
                    let ir_default = match default {
                        Some(d) => {
                            vec![self.elaborate_stmt(d, signal_map, known_modules, signals)?]
                        }
                        None => vec![],
                    };
                    Ok(IrStmt::Case {
                        case_type: CaseType::Normal,
                        expr: ir_expr,
                        items: ir_items,
                        default: ir_default,
                    })
                }
            }
            Stmt::StmtAssign { lhs, rhs } => {
                let ir_lhs = self.elaborate_lvalue(lhs, signal_map, signals)?;
                let ir_rhs = self.elaborate_expr(rhs, signal_map, signals)?;
                Ok(IrStmt::BlockingAssign {
                    lhs: ir_lhs,
                    rhs: ir_rhs,
                    delay: None,
                })
            }
            Stmt::Expr { expr } => {
                match expr {
                    Expr::MethodCall {
                        obj,
                        method,
                        args,
                        with_clause,
                    } => {
                        let ir_obj = self.elaborate_expr(obj, signal_map, signals)?;
                        let ir_args: Vec<IrExpr> = args
                            .iter()
                            .map(|a| self.elaborate_expr(a, signal_map, signals))
                            .collect::<Result<_, _>>()?;
                        let ir_with = match with_clause {
                            Some(wc) => {
                                Some(Box::new(self.elaborate_expr(wc, signal_map, signals)?))
                            }
                            None => None,
                        };
                        Ok(IrStmt::MethodCallStmt {
                            obj: ir_obj,
                            method: *method,
                            args: ir_args,
                            with_clause: ir_with,
                        })
                    }
                    Expr::FuncCall { name, .. } if name.starts_with("$") => {
                        let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                        Ok(IrStmt::SysCall {
                            name: Symbol::intern(""),
                            args: vec![ir_expr],
                        })
                    }
                    Expr::FuncCall { name, .. } if name.ends_with("::new") => {
                        let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                        Ok(IrStmt::SysCall {
                            name: Symbol::intern(""),
                            args: vec![ir_expr],
                        })
                    }
                    Expr::FuncCall { name, .. } => {
                        // Check if this is a DPI function call used as a statement
                        let is_dpi = self.design.modules.iter().flat_map(|m| m.items.iter()).any(
                            |item| matches!(item, ModuleItem::DpiImport(d) if d.name == *name),
                        );
                        if is_dpi {
                            let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                            Ok(IrStmt::SysCall {
                                name: Symbol::intern("__dpi_stmt"),
                                args: vec![ir_expr],
                            })
                        } else {
                            // Side-effect-free expression statement — eliminate it
                            Ok(IrStmt::Block { stmts: vec![] })
                        }
                    }
                    _ => {
                        // Side-effect-free expression statement — eliminate it
                        Ok(IrStmt::Block { stmts: vec![] })
                    }
                }
            }
            Stmt::SysCall { name, args } => {
                let ir_args: Vec<IrExpr> = args
                    .iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect::<Result<_, _>>()?;
                Ok(IrStmt::SysCall {
                    name: *name,
                    args: ir_args,
                })
            }
            Stmt::SysFinish => Ok(IrStmt::SysFinish),
            Stmt::Null => Ok(IrStmt::Null),
            Stmt::Return(_) => Ok(IrStmt::Null),
            Stmt::EventControl { events, stmt } => {
                if events.is_empty() {
                    return Ok(IrStmt::Null);
                }
                let body = match stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                // @(*) — wildcard: treat as immediate (block), bukan blocking event.
                if events.iter().any(|e| matches!(e, SensitivityEvent::Wildcard)) {
                    return Ok(IrStmt::Block { stmts: body });
                }
                // Dukung multi-event `@(a or b)` / `@(posedge a or posedge b)`:
                // kumpulkan SEMUA (sig_id, edge), bukan hanya event pertama.
                let mut sigs = Vec::with_capacity(events.len());
                for event in events {
                    match event {
                        SensitivityEvent::PosEdge(expr) => {
                            let sig_id = resolve_expr_signal(expr, signal_map).ok_or_else(
                                || self.elab_diag(DiagCode::ModuleNotFound, "cannot resolve signal in @(...)".to_string()),
                            )?;
                            sigs.push((sig_id, Some(ClockEdge::PosEdge(sig_id))));
                        }
                        SensitivityEvent::NegEdge(expr) => {
                            let sig_id = resolve_expr_signal(expr, signal_map).ok_or_else(
                                || self.elab_diag(DiagCode::ModuleNotFound, "cannot resolve signal in @(...)".to_string()),
                            )?;
                            sigs.push((sig_id, Some(ClockEdge::NegEdge(sig_id))));
                        }
                        SensitivityEvent::Level(expr) => {
                            let sig_id = resolve_expr_signal(expr, signal_map).ok_or_else(
                                || self.elab_diag(DiagCode::ModuleNotFound, "cannot resolve signal in @(...)".to_string()),
                            )?;
                            sigs.push((sig_id, None));
                        }
                        SensitivityEvent::Wildcard => unreachable!("handled above"),
                    }
                }
                Ok(IrStmt::EventControl { sigs, body })
            }
            Stmt::EventTrigger { name } => {
                if let Some(sig_id) = signal_map.get(name) {
                    Ok(IrStmt::EventTrigger { sig_id: *sig_id })
                } else {
                    Ok(IrStmt::Null)
                }
            }
            Stmt::Force { lhs, rhs } => {
                let ir_lhs = self.elaborate_lvalue(lhs, signal_map, signals)?;
                let ir_rhs = self.elaborate_expr(rhs, signal_map, signals)?;
                Ok(IrStmt::Force {
                    lvalue: ir_lhs,
                    rhs: ir_rhs,
                })
            }
            Stmt::Release { expr } => {
                let ir_lhs = self.elaborate_lvalue(expr, signal_map, signals)?;
                Ok(IrStmt::Release { lvalue: ir_lhs })
            }
            Stmt::Deassign { expr } => {
                let ir_lhs = self.elaborate_lvalue(expr, signal_map, signals)?;
                Ok(IrStmt::Deassign { lvalue: ir_lhs })
            }
            Stmt::CaseX {
                expr,
                items,
                default,
            } => {
                let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                let mut ir_items = Vec::new();
                for item in items {
                    let mut labels = Vec::new();
                    for label in &item.labels {
                        labels.push(self.elaborate_expr(label, signal_map, signals)?);
                    }
                    let body = match &*item.stmt {
                        Stmt::Block { stmts } => {
                            self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?
                        }
                        other => self.elaborate_stmt_block(
                            std::slice::from_ref(other),
                            signal_map,
                            known_modules,
                            signals,
                        )?,
                    };
                    ir_items.push(IrCaseItem { labels, body });
                }
                let ir_default = match default {
                    Some(d) => vec![self.elaborate_stmt(d, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Case {
                    case_type: CaseType::CaseX,
                    expr: ir_expr,
                    items: ir_items,
                    default: ir_default,
                })
            }
            Stmt::CaseZ {
                expr,
                items,
                default,
            } => {
                let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                let mut ir_items = Vec::new();
                for item in items {
                    let mut labels = Vec::new();
                    for label in &item.labels {
                        labels.push(self.elaborate_expr(label, signal_map, signals)?);
                    }
                    let body = match &*item.stmt {
                        Stmt::Block { stmts } => {
                            self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?
                        }
                        other => self.elaborate_stmt_block(
                            std::slice::from_ref(other),
                            signal_map,
                            known_modules,
                            signals,
                        )?,
                    };
                    ir_items.push(IrCaseItem { labels, body });
                }
                let ir_default = match default {
                    Some(d) => vec![self.elaborate_stmt(d, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Case {
                    case_type: CaseType::CaseZ,
                    expr: ir_expr,
                    items: ir_items,
                    default: ir_default,
                })
            }
            Stmt::NamedBlock { name, stmts, decls } => {
                let body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::NamedBlock {
                    name: *name,
                    stmts: body,
                    decls: decls.clone(),
                })
            }
            Stmt::Delay { delay, stmt } => {
                let d = const_eval_params(delay, &self.param_vals)? as u64;
                let body = vec![self.elaborate_stmt(stmt, signal_map, known_modules, signals)?];
                Ok(IrStmt::Delay { delay: d, body })
            }
            Stmt::Wait { cond, stmt } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let body = match stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Wait {
                    cond: ir_cond,
                    body,
                })
            }
            Stmt::LoopFor {
                init,
                cond,
                step,
                stmts,
            } => {
                // Try to unroll constant-bounded for loops at elaboration time
                let unroll_result = try_unroll_for_loop(
                    init.as_deref(),
                    cond.as_ref(),
                    step.as_deref(),
                    stmts,
                    &|stmts, var_name, iter_val| {
                        let subst_stmts = substitute_loop_var_in_stmts(stmts, var_name, iter_val);
                        if std::env::var("DBG_LOOP").is_ok() {
                            eprintln!("[DBG-LOOP] iter {}={} body={:?}", var_name, iter_val, subst_stmts);
                        }
                        self.elaborate_stmt_block(&subst_stmts, signal_map, known_modules, signals)
                            .map_err(|e| e.to_string())
                    },
                    &self.param_vals,
                );
                if std::env::var("DBG_LOOP").is_ok() {
                    match &unroll_result {
                        Ok(Some(u)) => eprintln!("[DBG-LOOP] unrolled {} stmts", u.len()),
                        Ok(None) => eprintln!("[DBG-LOOP] unroll None (fallback runtime)"),
                        Err(e) => eprintln!("[DBG-LOOP] unroll Err: {}", e),
                    }
                }
                if let Ok(Some(unrolled)) = unroll_result {
                    return Ok(IrStmt::Block { stmts: unrolled });
                }
                // Fallback: generate runtime LoopFor
                let ir_init = match init {
                    Some(s) => Some(Box::new(self.elaborate_stmt(
                        s,
                        signal_map,
                        known_modules,
                        signals,
                    )?)),
                    None => None,
                };
                let ir_cond = if let Some(c) = cond {
                    self.elaborate_expr(c, signal_map, signals)?
                } else {
                    IrExpr::Const(LogicVec::from_u64(1, 1))
                };
                let ir_step = match step {
                    Some(s) => Some(Box::new(self.elaborate_stmt(
                        s,
                        signal_map,
                        known_modules,
                        signals,
                    )?)),
                    None => None,
                };
                let ir_body =
                    self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::LoopFor {
                    init: ir_init,
                    cond: ir_cond,
                    step: ir_step,
                    body: ir_body,
                })
            }
            Stmt::LoopWhile { cond, stmts } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let ir_body =
                    self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::LoopWhile {
                    cond: ir_cond,
                    body: ir_body,
                })
            }
            Stmt::DoWhile { cond, stmts } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let ir_body =
                    self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::LoopDoWhile {
                    cond: ir_cond,
                    body: ir_body,
                })
            }
            Stmt::LoopForever { stmts } => {
                let ir_body =
                    self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::LoopWhile {
                    cond: IrExpr::Const(LogicVec::from_u64(1, 1)),
                    body: ir_body,
                })
            }
            Stmt::ForeachLoop {
                array_var,
                index_vars,
                stmts,
            } => {
                let sig_id = signal_map.get(array_var).ok_or_else(|| {
                    self.elab_diag(DiagCode::ModuleNotFound, format!("array '{}' not found for foreach", array_var))
                })?;
                let sig_info = signals.get(*sig_id).ok_or_else(|| {
                    self.elab_diag(DiagCode::ModuleNotFound, format!("signal info not found for '{}'", array_var))
                })?;
                if sig_info.is_dynamic || sig_info.is_queue {
                    let ir_body =
                        self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                    let iv = index_vars
                        .first()
                        .cloned()
                        .unwrap_or_else(|| Symbol::intern("i"));
                    Ok(IrStmt::Foreach {
                        array_var: IrExpr::Signal(*sig_id, sig_info.width),
                        index_var: iv,
                        body: ir_body,
                    })
                } else {
                    let n = sig_info.array_depth;
                    if n == 0 {
                        return Err(self.elab_diag(DiagCode::TypeMismatch, format!(
                            "'{}' is not an array, cannot use foreach",
                            array_var
                        )));
                    }
                    let mut all_stmts = Vec::new();
                    let iv = index_vars
                        .first()
                        .cloned()
                        .unwrap_or_else(|| Symbol::intern("i"));
                    for i in 0..n {
                        let subst_stmts = substitute_loop_var_in_stmts(stmts, iv.as_str(), i as i64);
                        all_stmts.extend(self.elaborate_stmt_block(
                            &subst_stmts,
                            signal_map,
                            known_modules,
                            signals,
                        )?);
                    }
                    Ok(IrStmt::Block { stmts: all_stmts })
                }
            }
            Stmt::StmtCase {
                expr,
                items,
                default,
            } => {
                let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                let mut ir_items = Vec::new();
                for item in items {
                    let mut labels = Vec::new();
                    for label in &item.labels {
                        labels.push(self.elaborate_expr(label, signal_map, signals)?);
                    }
                    let body = match &*item.stmt {
                        Stmt::Block { stmts } => {
                            self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?
                        }
                        other => self.elaborate_stmt_block(
                            std::slice::from_ref(other),
                            signal_map,
                            known_modules,
                            signals,
                        )?,
                    };
                    ir_items.push(IrCaseItem { labels, body });
                }
                let ir_default = match default {
                    Some(d) => vec![self.elaborate_stmt(d, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Case {
                    case_type: CaseType::Normal,
                    expr: ir_expr,
                    items: ir_items,
                    default: ir_default,
                })
            }
            Stmt::Break => Ok(IrStmt::Break),
            Stmt::Continue => Ok(IrStmt::Continue),
            Stmt::Disable { name } => Ok(IrStmt::Disable { name: *name }),
            Stmt::Repeat { count, stmts } => {
                if let Ok(n) = const_eval_params(count, &self.param_vals) {
                    let mut all = Vec::new();
                    for _ in 0..n {
                        all.extend(self.elaborate_stmt_block(
                            stmts,
                            signal_map,
                            known_modules,
                            signals,
                        )?);
                    }
                    Ok(IrStmt::Block { stmts: all })
                } else {
                    let ir_count = self.elaborate_expr(count, signal_map, signals)?;
                    let ir_body =
                        self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                    Ok(IrStmt::Repeat {
                        count: ir_count,
                        body: ir_body,
                    })
                }
            }
            // New variants: elaborate as comparable constructs
            Stmt::UniqueCase {
                expr,
                items,
                default,
            }
            | Stmt::PriorityCase {
                expr,
                items,
                default,
            } => self.elaborate_stmt(
                &Stmt::Case {
                    expr: expr.clone(),
                    items: items.clone(),
                    default: default.clone(),
                },
                signal_map,
                known_modules,
                signals,
            ),
            Stmt::CaseInside {
                expr,
                items,
                default,
            } => self.elaborate_stmt(
                &Stmt::Case {
                    expr: expr.clone(),
                    items: items.clone(),
                    default: default.clone(),
                },
                signal_map,
                known_modules,
                signals,
            ),
            Stmt::UniqueIf {
                cond,
                true_branch,
                false_branch,
            } => self.elaborate_stmt(
                &Stmt::IfElse {
                    cond: cond.clone(),
                    true_branch: true_branch.clone(),
                    false_branch: false_branch.clone(),
                },
                signal_map,
                known_modules,
                signals,
            ),
            Stmt::PriorityIf {
                cond,
                true_branch,
                false_branch,
            } => self.elaborate_stmt(
                &Stmt::IfElse {
                    cond: cond.clone(),
                    true_branch: true_branch.clone(),
                    false_branch: false_branch.clone(),
                },
                signal_map,
                known_modules,
                signals,
            ),
            Stmt::Assert {
                cond,
                pass_stmt,
                fail_stmt,
                clock_event,
                disable_iff,
            } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let pass = match pass_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                let fail = match fail_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                let ir_disable = match disable_iff {
                    Some(e) => Some(Box::new(self.elaborate_expr(e, signal_map, signals)?)),
                    None => None,
                };
                Ok(IrStmt::Assert {
                    cond: ir_cond,
                    pass_stmt: pass,
                    fail_stmt: fail,
                    clock_event: clock_event.clone(),
                    disable_iff: ir_disable,
                    sequence: None,
                })
            }
            Stmt::Assume {
                cond,
                pass_stmt,
                fail_stmt,
                clock_event,
                disable_iff,
            } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let pass = match pass_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                let fail = match fail_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                let ir_disable = match disable_iff {
                    Some(e) => Some(Box::new(self.elaborate_expr(e, signal_map, signals)?)),
                    None => None,
                };
                Ok(IrStmt::Assume {
                    cond: ir_cond,
                    pass_stmt: pass,
                    fail_stmt: fail,
                    clock_event: clock_event.clone(),
                    disable_iff: ir_disable,
                    sequence: None,
                })
            }
            Stmt::Cover {
                cond,
                pass_stmt,
                clock_event,
                disable_iff,
            } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let pass = match pass_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                let ir_disable = match disable_iff {
                    Some(e) => Some(Box::new(self.elaborate_expr(e, signal_map, signals)?)),
                    None => None,
                };
                Ok(IrStmt::Cover {
                    cond: ir_cond,
                    pass_stmt: pass,
                    clock_event: clock_event.clone(),
                    disable_iff: ir_disable,
                    sequence: None,
                })
            }
            Stmt::Expect { .. } => Ok(IrStmt::Null),
            Stmt::WaitOrder { events, fail_stmt } => {
                let mut sig_ids = Vec::new();
                for name in events {
                    if let Some(idx) = signal_map.get(name) {
                        sig_ids.push(*idx);
                    } else {
                        return Err(self.elab_diag(DiagCode::ModuleNotFound, format!(
                            "wait_order: signal '{}' not found",
                            name
                        )));
                    }
                }
                let failure = match fail_stmt {
                    Some(s) => {
                        vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?]
                    }
                    None => vec![],
                };
                Ok(IrStmt::WaitOrder {
                    events: sig_ids,
                    failure_stmts: failure,
                })
            }
            Stmt::Fork {
                processes,
                join_type,
            } => {
                let mut ir_processes = Vec::new();
                for proc_stmt in processes {
                    let ir = self.elaborate_stmt(proc_stmt, signal_map, known_modules, signals)?;
                    ir_processes.push(vec![ir]);
                }
                let ir_join = match join_type {
                    JoinType::Join => IrJoinType::Join,
                    JoinType::JoinAny => IrJoinType::JoinAny,
                    JoinType::JoinNone => IrJoinType::JoinNone,
                };
                Ok(IrStmt::Fork {
                    processes: ir_processes,
                    join_type: ir_join,
                })
            }
            Stmt::RandCase { items } => {
                let new_items: Result<Vec<(IrExpr, Vec<IrStmt>)>, SimError> = items
                    .iter()
                    .map(|rc| {
                        let weight_expr = IrExpr::Const(LogicVec::from_u64(rc.weight, 32));
                        let body = self.elaborate_stmt_block(
                            &[*rc.stmt.clone()],
                            signal_map,
                            known_modules,
                            signals,
                        )?;
                        Ok((weight_expr, body))
                    })
                    .collect();
                Ok(IrStmt::RandCase { items: new_items? })
            }
            Stmt::RandSequence { productions } => {
                let mut ir_productions = Vec::new();
                for prod in productions {
                    let mut ir_items = Vec::new();
                    for item in &prod.items {
                        let weight_expr = if let Some(w) = item.weight {
                            IrExpr::Const(LogicVec::from_u64(w, 32))
                        } else {
                            IrExpr::Const(LogicVec::from_u64(1, 32))
                        };
                        let body = self.elaborate_stmt_block(
                            &[(*item.value).clone()],
                            signal_map,
                            known_modules,
                            signals,
                        )?;
                        ir_items.push((weight_expr, body));
                    }
                    ir_productions.push((prod.name, ir_items));
                }
                Ok(IrStmt::RandSequence {
                    productions: ir_productions,
                })
            }
        }
    }

    pub(crate) fn elaborate_lvalue(
        &self,
        expr: &Expr,
        signal_map: &HashMap<Symbol, SignalId>,
        signals: &[SignalInfo],
    ) -> Result<IrLValue, SimError> {
        match expr {
            Expr::Ident { name, line, col } => {
                let sig_id = signal_map
                    .get(name)
                    .ok_or_else(|| self.elab_diag_at(DiagCode::UndefinedSignal, format!("signal '{}' not found", name), *line, *col))?;
                Ok(IrLValue::Signal(*sig_id, 0))
            }
            Expr::RangeSelect {
                expr: inner,
                msb,
                lsb,
            } => {
                let inner_lv = self.elaborate_lvalue(inner, signal_map, signals)?;
                let msb_c = const_eval_params(msb, &self.param_vals)? as usize;
                let lsb_c = const_eval_params(lsb, &self.param_vals)? as usize;
                match inner_lv {
                    IrLValue::Signal(sid, _) => Ok(IrLValue::RangeSelect(sid, msb_c, lsb_c)),
                    IrLValue::RangeSelect(sid, outer_msb, outer_lsb) => {
                        let outer_start = if outer_msb > outer_lsb {
                            outer_lsb
                        } else {
                            outer_msb
                        };
                        let inner_start = outer_start + if msb_c > lsb_c { lsb_c } else { msb_c };
                        let inner_end = outer_start + if msb_c > lsb_c { msb_c } else { lsb_c };
                        Ok(IrLValue::RangeSelect(sid, inner_end, inner_start))
                    }
                    IrLValue::ArrayIndex {
                        sig_id,
                        index,
                        elem_width,
                    } => Ok(IrLValue::ArrayRangeSelect {
                        sig_id,
                        index,
                        elem_width,
                        msb: msb_c,
                        lsb: lsb_c,
                    }),
                    // Range select di atas bit select (`sig[a][b][msb:lsb]` —
                    // packed multidimensi setelah unroll). Akumulasi offset bit.
                    IrLValue::BitSelect(sid, base_bit) => {
                        let start = base_bit + msb_c.min(lsb_c);
                        let end = base_bit + msb_c.max(lsb_c);
                        Ok(IrLValue::RangeSelect(sid, end, start))
                    }
                    _ => Err(self.elab_diag_at(
                        DiagCode::NotImplemented,
                        "nested range select not supported",
                        expr_location(expr).0,
                        expr_location(expr).1,
                    )),
                }
            }
            Expr::BitSelect {
                expr: inner,
                index: bs_index,
            } => {
                let inner_lv = self.elaborate_lvalue(inner, signal_map, signals)?;
                match inner_lv {
                    IrLValue::Signal(sid, _) => {
                        let sig = &signals[sid];
                        // Check for multi-dim packed array: packed_dims.len() > 1
                        if sig.packed_dims.len() > 1 {
                            let outer_elem_width = sig.width / sig.packed_dims[0];
                            if let Ok(idx) = const_eval_params(bs_index, &self.param_vals) {
                                let idx = idx as usize;
                                let lsb = idx * outer_elem_width;
                                let msb = lsb + outer_elem_width - 1;
                                Ok(IrLValue::RangeSelect(sid, msb, lsb))
                            } else {
                                let index_expr =
                                    self.elaborate_expr(bs_index, signal_map, signals)?;
                                Ok(IrLValue::ArrayIndex {
                                    sig_id: sid,
                                    index: Box::new(index_expr),
                                    elem_width: outer_elem_width,
                                })
                            }
                        } else if sig.array_depth > 1 || sig.is_dynamic || sig.is_queue {
                            let index_expr = self.elaborate_expr(bs_index, signal_map, signals)?;
                            Ok(IrLValue::ArrayIndex {
                                sig_id: sid,
                                index: Box::new(index_expr),
                                elem_width: sig.elem_width,
                            })
                        } else if let Ok(idx) = const_eval_params(bs_index, &self.param_vals) {
                            Ok(IrLValue::BitSelect(sid, idx as usize))
                        } else {
                            // Dynamic index on a flat signal — treat as array index
                            let index_expr = self.elaborate_expr(bs_index, signal_map, signals)?;
                            Ok(IrLValue::ArrayIndex {
                                sig_id: sid,
                                index: Box::new(index_expr),
                                elem_width: sig.elem_width,
                            })
                        }
                    }
                    IrLValue::RangeSelect(sid, outer_msb, outer_lsb) => {
                        if let Ok(idx) = const_eval_params(bs_index, &self.param_vals) {
                            let base = if outer_msb > outer_lsb {
                                outer_lsb
                            } else {
                                outer_msb
                            };
                            Ok(IrLValue::BitSelect(sid, base + idx as usize))
                        } else {
                            let index_expr = self.elaborate_expr(bs_index, signal_map, signals)?;
                            Ok(IrLValue::ArrayIndex {
                                sig_id: sid,
                                index: Box::new(index_expr),
                                elem_width: outer_msb.max(outer_lsb) - outer_msb.min(outer_lsb) + 1,
                            })
                        }
                    }
                    IrLValue::ArrayIndex {
                        sig_id,
                        index,
                        elem_width,
                    } => {
                        if let Ok(idx) = const_eval_params(bs_index, &self.param_vals) {
                            Ok(IrLValue::ArrayBitSelect {
                                sig_id,
                                index,
                                elem_width,
                                bit: idx as usize,
                            })
                        } else {
                            Err(self.elab_diag_at(
                                DiagCode::NotImplemented,
                                "dynamic bit-select on array element not supported",
                                expr_location(expr).0,
                                expr_location(expr).1,
                            ))
                        }
                    }
                    // Bit select di atas bit select (`sig[a][b]` — packed
                    // multidimensi setelah unroll, mis. `result[0][0][0]`).
                    // Akumulasi offset bit.
                    IrLValue::BitSelect(sid, inner_bit) => {
                        if let Ok(idx) = const_eval_params(bs_index, &self.param_vals) {
                            Ok(IrLValue::BitSelect(sid, inner_bit + idx as usize))
                        } else {
                            let index_expr =
                                self.elaborate_expr(bs_index, signal_map, signals)?;
                            Ok(IrLValue::ArrayBitSelect {
                                sig_id: sid,
                                index: Box::new(index_expr),
                                elem_width: 1,
                                bit: inner_bit,
                            })
                        }
                    }
                    _ => Err(self.elab_diag_at(
                        DiagCode::NotImplemented,
                        "nested bit select not supported",
                        expr_location(expr).0,
                        expr_location(expr).1,
                    )),
                }
            }
            Expr::PartSelect {
                expr: inner,
                base,
                width,
            } => {
                let inner_lv = self.elaborate_lvalue(inner, signal_map, signals)?;
                let base_r = const_eval_params(base, &self.param_vals);
                let width_r = const_eval_params(width, &self.param_vals);
                let (base_c, width_c) = match (base_r, width_r) {
                    (Ok(b), Ok(w)) => (b as usize, w as usize),
                    _ => return Err(self.elab_diag_at(
                        DiagCode::NotImplemented,
                        "dynamic part-select not supported",
                        expr_location(expr).0,
                        expr_location(expr).1,
                    )),
                };
                match inner_lv {
                    IrLValue::Signal(sid, _) => {
                        if width_c > 0 {
                            Ok(IrLValue::RangeSelect(sid, base_c + width_c - 1, base_c))
                        } else {
                            Ok(IrLValue::RangeSelect(sid, base_c, base_c))
                        }
                    }
                    IrLValue::RangeSelect(sid, outer_msb, outer_lsb) => {
                        let outer_base = if outer_msb > outer_lsb {
                            outer_lsb
                        } else {
                            outer_msb
                        };
                        let new_base = outer_base + base_c;
                        if width_c > 0 {
                            Ok(IrLValue::RangeSelect(sid, new_base + width_c - 1, new_base))
                        } else {
                            Ok(IrLValue::RangeSelect(sid, new_base, new_base))
                        }
                    }
                    IrLValue::ArrayIndex {
                        sig_id,
                        index,
                        elem_width,
                    } => {
                        if width_c > 0 {
                            Ok(IrLValue::ArrayRangeSelect {
                                sig_id,
                                index,
                                elem_width,
                                msb: base_c + width_c - 1,
                                lsb: base_c,
                            })
                        } else {
                            Ok(IrLValue::ArrayRangeSelect {
                                sig_id,
                                index,
                                elem_width,
                                msb: base_c,
                                lsb: base_c,
                            })
                        }
                    }
                    _ => Err(self.elab_diag_at(
                        DiagCode::NotImplemented,
                        "nested part-select in lvalue not supported",
                        expr_location(expr).0,
                        expr_location(expr).1,
                    )),
                }
            }
            Expr::Concat(exprs) => {
                let parts: Result<Vec<IrLValue>, SimError> = exprs
                    .iter()
                    .map(|e| self.elaborate_lvalue(e, signal_map, signals))
                    .collect();
                Ok(IrLValue::Concat(parts?))
            }
            Expr::MethodCall { .. } => Err(self.elab_diag_at(
                DiagCode::NotImplemented,
                "method calls cannot be used as lvalues",
                expr_location(expr).0,
                expr_location(expr).1,
            )),
            Expr::MemberAccess { obj, field } => {
                // Try struct/union field write
                let hier_name = Self::build_hier_name(obj, field.as_str());
                if let Some(&sig_id) = signal_map.get(hier_name.as_str()) {
                    return Ok(IrLValue::Signal(sig_id, 0));
                }
                // Nested member access lvalue (`hw2reg.val.d = x`): kumpulkan
                // chain field (dari luar ke dalam), resolve base signal, lalu
                // akumulasi offset berjenjang via struct_fields + typedef_field_map.
                if let Some((base_name, chain)) = Self::collect_member_chain(obj, *field) {
                    if let Some(&base_sid) = signal_map.get(base_name.as_str()) {
                        let base_info = &signals[base_sid];
                        if !base_info.struct_fields.is_empty() {
                            let mut offset = 0usize;
                            let mut width = 1usize;
                            let mut cur_fields: Option<Vec<StructFieldInfo>> =
                                Some(base_info.struct_fields.clone());
                            let mut last_field: Option<StructFieldInfo> = None;
                            let mut ok = true;
                            for (i, step) in chain.iter().enumerate() {
                                match step {
                                    ChainStep::Index(idx) => {
                                        // Elemen ke-idx dari field struct array
                                        // (atau base signal bila di posisi awal).
                                        let elem_width = if let Some(f) = &last_field {
                                            f.type_name
                                                .as_ref()
                                                .and_then(|tn| {
                                                    self.lookup_struct_fields(tn.as_str())
                                                })
                                                .map(|fs| {
                                                    fs.iter()
                                                        .map(|sf| sf.width)
                                                        .sum::<usize>()
                                                        .max(1)
                                                })
                                                .unwrap_or(1)
                                        } else {
                                            base_info.elem_width.max(1)
                                        };
                                        offset = offset.saturating_add(
                                            (*idx as usize).saturating_mul(elem_width),
                                        );
                                        // Fields elemen struct (untuk field berikutnya).
                                        if let Some(f) = &last_field {
                                            if let Some(tn) = &f.type_name {
                                                cur_fields =
                                                    self.lookup_struct_fields(tn.as_str());
                                            }
                                        }
                                        last_field = None;
                                    }
                                    ChainStep::Field(fname) => {
                                        let fields = match &cur_fields {
                                            Some(fs) => fs.clone(),
                                            None => {
                                                ok = false;
                                                break;
                                            }
                                        };
                                        let found = fields.iter().find(|f| f.name == *fname);
                                        match found {
                                            Some(f) => {
                                                offset += f.offset;
                                                width = f.width;
                                                last_field = Some(f.clone());
                                                if i + 1 < chain.len() {
                                                    let mut nxt = f
                                                        .type_name
                                                        .as_ref()
                                                        .and_then(|tn| {
                                                            self.lookup_struct_fields(tn.as_str())
                                                        });
                                                    if nxt.is_none() {
                                                        nxt = Some(f.sub_fields.clone());
                                                    }
                                                    cur_fields = nxt;
                                                }
                                            }
                                            None => {
                                                ok = false;
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            if ok {
                                let lsb = offset;
                                let msb = offset + width - 1;
                                return Ok(IrLValue::RangeSelect(base_sid, msb, lsb));
                            }
                        }
                    }
                }
                match self.elaborate_expr(obj, signal_map, signals) {
                    Ok(IrExpr::Signal(sig_id, _)) => {
                        let sig_info = &signals[sig_id];
                        if !sig_info.struct_fields.is_empty() {
                            if let Some(f) =
                                sig_info.struct_fields.iter().find(|f| f.name == *field)
                            {
                                let lsb = f.offset;
                                let msb = f.offset + f.width - 1;
                                return Ok(IrLValue::RangeSelect(sig_id, lsb, msb));
                            }
                            return Err(self.elab_diag_at(DiagCode::ModuleNotFound, format!(
                                "field '{}' not found in struct type",
                                field
                            ), expr_location(expr).0, expr_location(expr).1));
                        }
                        // Class object handle: obj = signal berisi obj id → field.
                        if sig_info.class_name.is_some() {
                            return Ok(IrLValue::ObjectField {
                                sig_id,
                                field: *field,
                            });
                        }
                        Err(self.elab_diag_at(DiagCode::ModuleNotFound, format!("member access on signal '{:?}' that has no struct fields (cannot use as lvalue)", obj), expr_location(expr).0, expr_location(expr).1))
                    }
                    _ => Err(self.elab_diag_at(
                        DiagCode::NotImplemented,
                        "member access cannot be used as lvalues",
                        expr_location(expr).0,
                        expr_location(expr).1,
                    )),
                }
            }
            _ => Err(self.elab_diag_at(
                DiagCode::InvalidSyntax,
                format!("invalid lvalue expression: {:?}", expr),
                expr_location(expr).0,
                expr_location(expr).1,
            )),
        }
    }

    /// Kumpulkan chain member access `a.b.c` → (nama signal base, urutan step
    /// dari luar ke dalam). Contoh: `hw2reg.val.d` → ("hw2reg", [val, d]);
    /// `reg2hw.key_share0[0].qe` → ("reg2hw", [key_share0, Index(0), qe]).
    /// Index konstanta (genvar sudah di-substitute saat generate expansion).
    fn collect_member_chain(obj: &Expr, leaf_field: Symbol) -> Option<(String, Vec<ChainStep>)> {
        let mut chain = vec![ChainStep::Field(leaf_field)];
        let mut cur = obj;
        loop {
            match cur {
                Expr::MemberAccess {
                    obj: inner,
                    field,
                } => {
                    chain.push(ChainStep::Field(*field));
                    cur = inner;
                }
                Expr::BitSelect { expr: inner, index } => {
                    if let Ok(idx) = const_eval_simple(index) {
                        chain.push(ChainStep::Index(idx));
                        cur = inner;
                    } else {
                        return None;
                    }
                }
                Expr::Ident { name, .. } => {
                    let base = name.as_str().to_string();
                    chain.reverse(); // [base_step, ..., leaf_step]
                    return Some((base, chain));
                }
                _ => return None,
            }
        }
    }
}

/// Langkah dalam chain member access — field struct atau index array konstanta.
#[derive(Debug, Clone)]
enum ChainStep {
    Field(Symbol),
    Index(i64),
}
