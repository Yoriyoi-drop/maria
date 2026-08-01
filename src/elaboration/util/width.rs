//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan util.rs (SRP Refactoring).
//! Tanggung jawab: Expression width computation.
//!
//! Fungsi:
//!   - compute_expr_width() — hitung lebar expression (dalam bit)
//!
//! ──────────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;

use crate::ast::types::const_eval_with_params;
use crate::ast::*;
use crate::intern::Symbol;
use crate::ir::*;

/// Hitung lebar (width) expression dalam bit.
/// Mendukung: Ident, Value, FuncCall, Paren, UnaryOp, BinaryOp,
/// Concat, Replicate, TernaryOp, RangeSelect, BitSelect, PartSelect,
/// MemberAccess, Cast.
pub fn compute_expr_width(
    expr: &Expr,
    signal_map: &HashMap<Symbol, SignalId>,
    signals: &[SignalInfo],
    param_vals: &HashMap<Symbol, i64>,
    package_symbols: &HashMap<Symbol, HashMap<Symbol, PackageItem>>,
) -> Result<usize, String> {
    match expr {
        Expr::Ident { name, .. } => {
            if let Some(sig_id) = signal_map.get(name) {
                // SignalInfo.width ALREADY includes unpacked array depth
                // (total_width = elem_width * depth di-set saat elaborasi),
                // jadi jangan kalikan lagi dgn array_depth (double-count).
                let info = &signals[*sig_id];
                Ok(info.width)
            } else if let Some(&val) = param_vals.get(name) {
                let abs = val.unsigned_abs();
                Ok(if val == 0 {
                    1
                } else {
                    64 - (abs.leading_zeros() as usize)
                }
                .max(1))
            } else {
                Err(format!("cannot determine width of '{}'", name))
            }
        }
        Expr::Value(v) => match v {
            Value::Binary { width, .. } => Ok(width.unwrap_or(1)),
            Value::Hex { width, .. } => Ok(width.unwrap_or(1)),
            Value::Octal { width, .. } => Ok(width.unwrap_or(1)),
            Value::Decimal(_) => Ok(32),
            Value::Real(_) => Ok(64),
        },
        Expr::FillLit(_) => Ok(1),
        Expr::FuncCall { name, .. } => {
            if let Some(width) = param_vals.get(name) {
                let abs = width.unsigned_abs();
                Ok(if *width == 0 {
                    1
                } else {
                    64 - (abs.leading_zeros() as usize)
                }
                .max(1))
            } else if let Some((pkg_name, func_name)) = name.split_once("::") {
                let pkg_sym = Symbol::intern(pkg_name);
                let func_sym = Symbol::intern(func_name);
                if let Some(pkg) = package_symbols.get(&pkg_sym) {
                    if let Some(PackageItem::Function(f)) = pkg.get(&func_sym) {
                        let ret_width = if let Some(r) = &f.range {
                            if let (Ok(msb), Ok(lsb)) = (
                                const_eval_with_params(&r.msb, param_vals),
                                const_eval_with_params(&r.lsb, param_vals),
                            ) {
                                (msb.abs_diff(lsb).saturating_add(1)) as usize
                            } else {
                                1
                            }
                        } else {
                            1
                        };
                        Ok(ret_width)
                    } else {
                        Err(format!("cannot determine width of function '{}'", name))
                    }
                } else {
                    Err(format!("cannot determine width of function '{}'", name))
                }
            } else {
                Err(format!("cannot determine width of function '{}'", name))
            }
        }
        Expr::Paren(inner) => {
            compute_expr_width(inner, signal_map, signals, param_vals, package_symbols)
        }
        Expr::UnaryOp { op, expr: inner } => match op {
            UnaryOp::ReductionAnd
            | UnaryOp::ReductionNand
            | UnaryOp::ReductionOr
            | UnaryOp::ReductionNor
            | UnaryOp::ReductionXor
            | UnaryOp::ReductionXnor
            | UnaryOp::Not => Ok(1),
            _ => compute_expr_width(inner, signal_map, signals, param_vals, package_symbols),
        },
        Expr::BinaryOp { lhs, rhs, .. } => {
            let lw = compute_expr_width(lhs, signal_map, signals, param_vals, package_symbols)?;
            let rw = compute_expr_width(rhs, signal_map, signals, param_vals, package_symbols)?;
            Ok(lw.max(rw))
        }
        Expr::Concat(items) => {
            let mut total: usize = 0;
            for item in items {
                total = total.saturating_add(compute_expr_width(
                    item,
                    signal_map,
                    signals,
                    param_vals,
                    package_symbols,
                )?);
            }
            Ok(total)
        }
        Expr::Replicate { count, expr: inner } => {
            let c = const_eval_with_params(count, param_vals).unwrap_or(1) as usize;
            let w = compute_expr_width(inner, signal_map, signals, param_vals, package_symbols)?;
            Ok(c.saturating_mul(w))
        }
        Expr::TernaryOp {
            true_expr,
            false_expr,
            ..
        } => {
            let tw =
                compute_expr_width(true_expr, signal_map, signals, param_vals, package_symbols)?;
            let fw =
                compute_expr_width(false_expr, signal_map, signals, param_vals, package_symbols)?;
            Ok(tw.max(fw))
        }
        Expr::RangeSelect { msb, lsb, .. } => {
            if let (Ok(m), Ok(l)) = (
                const_eval_with_params(msb, param_vals),
                const_eval_with_params(lsb, param_vals),
            ) {
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
            if let Expr::Ident { name, .. } = obj.as_ref() {
                if let Some(&sig_id) = signal_map.get(name) {
                    if !signals[sig_id].struct_fields.is_empty() {
                        if let Some(f) = signals[sig_id]
                            .struct_fields
                            .iter()
                            .find(|f| f.name == *field)
                        {
                            return Ok(f.width);
                        }
                    }
                }
            }
            compute_expr_width(obj, signal_map, signals, param_vals, package_symbols)
        }
        Expr::Cast { dtype, .. } => match crate::elaboration::util::parse_type_spec_str(dtype.as_str()) {
            Some(dt) => match dt {
                DataType::UserDefined(name) => param_vals
                    .get(&name)
                    .map(|&v| v as usize)
                    .ok_or_else(|| format!("unknown type '{}'", name)),
                _ => Ok(dt.width()),
            },
            None => Err(format!("unknown type '{}' in cast", dtype)),
        },
        Expr::MethodCall { .. } | Expr::StreamingConcat { .. } | Expr::Dist { .. } => {
            Err("width not computable for this expression type".to_string())
        }
        Expr::ScopedIdent { package, item } => Err(format!(
            "cannot determine width of '{}.{}' at compile time",
            package, item
        )),
        _ => Err("cannot determine width of expression".to_string()),
    }
}
