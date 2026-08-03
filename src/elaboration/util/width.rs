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
                        } else if let Some(rt) = &f.return_type {
                            // Return type berupa typedef package (mis.
                            // `function automatic mubi4_t mubi4_or_hi(...)`)
                            // — resolve width dari typedef.
                            resolve_dtype_width(rt, package_symbols)
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
        Expr::BitSelect { expr: inner, .. } => {
            // `sig[idx]`: single-bit select UNTUK skalar/array 1D. Untuk packed
            // array multidimensi (`logic [1:0][3:0] mubis`), `mubis[0]` memilih
            // SATU ELEMENT (lebar = width/packed_dims[0]), bukan 1 bit.
            if let Expr::Ident { name, .. } = inner.as_ref() {
                if let Some(&sig_id) = signal_map.get(name) {
                    let sig = &signals[sig_id];
                    if sig.packed_dims.len() > 1 && sig.packed_dims[0] > 0 {
                        return Ok((sig.width / sig.packed_dims[0]).max(1));
                    }
                    if sig.array_depth > 1 {
                        return Ok(sig.elem_width.max(1));
                    }
                }
            }
            Ok(1)
        }
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
                DataType::UserDefined(name) => {
                    // 1) Parameter/konstanta: nilai = width (mis. `Width'(...)`).
                    if let Some(&v) = param_vals.get(&name) {
                        return Ok(v as usize);
                    }
                    // 2) Package param di-import (nama polos) — mis.
                    //    `MuBi4Width'(x)` dari `import prim_mubi_pkg::*`.
                    if let Some(v) = resolve_pkg_param_width(&name, package_symbols) {
                        return Ok(v as usize);
                    }
                    // 3) Typedef package (mis. `mubi4_t'(...)`) — resolve width
                    //    dari typedef di semua package.
                    Ok(resolve_dtype_width(&dt, package_symbols))
                }
                _ => Ok(dt.width()),
            },
            // dtype bukan base type — bisa jadi parameter/typedef package
            // (mis. `MuBi4Width'(x)`, `k'(x)`). Perlakukan sebagai identifier.
            None => {
                let name = Symbol::intern(dtype.as_str());
                if let Some(&v) = param_vals.get(&name) {
                    return Ok(v as usize);
                }
                if let Some(v) = resolve_pkg_param_width(&name, package_symbols) {
                    return Ok(v as usize);
                }
                Ok(resolve_dtype_width(&DataType::UserDefined(name), package_symbols))
            }
        },
        Expr::MethodCall { .. } | Expr::StreamingConcat { .. } | Expr::Dist { .. } => {
            Err("width not computable for this expression type".to_string())
        }
        Expr::ScopedIdent { package, item } => {
            // Enum member / konstanta package yang di-flatten ke param_vals
            // sebagai qualified `pkg::item` (build_pkg_param_ctx). Contoh nyata
            // OpenTitan: `prim_mubi_pkg::MuBi4False` sebagai argumen port.
            let qualified = Symbol::intern(&format!("{}::{}", package.as_str(), item.as_str()));
            if let Some(&val) = param_vals.get(&qualified) {
                let abs = val.unsigned_abs();
                return Ok(if val == 0 {
                    1
                } else {
                    64 - (abs.leading_zeros() as usize)
                }
                .max(1));
            }
            // Package parameter dengan default: `pkg::PARAM`.
            if let Some(pkg_items) = package_symbols.get(package) {
                if let Some(PackageItem::Param(p)) = pkg_items.get(item) {
                    if let Some(expr) = &p.default {
                        if let Ok(val) = const_eval_with_params(expr, param_vals) {
                            let abs = val.unsigned_abs();
                            return Ok(if val == 0 {
                                1
                            } else {
                                64 - (abs.leading_zeros() as usize)
                            }
                            .max(1));
                        }
                    }
                }
            }
            Err(format!(
                "cannot determine width of '{}.{}' at compile time",
                package, item
            ))
        }
        _ => Err("cannot determine width of expression".to_string()),
    }
}

/// Hitung lebar DataType dengan resolve typedef package / enum base.
/// Dipakai untuk return type function (`mubi4_t`) dan cast ke typedef
/// (`mubi4_t'(...)`). UserDefined di-resolve lewat `package_symbols`
/// (bukan default 64), EnumType via base-nya.
/// Cari parameter package dengan nama polos (hasil `import pkg::*`) dan
/// evaluasi default-nya. Dipakai untuk cast `MuBi4Width'(...)` di mana
/// `MuBi4Width` adalah `parameter` di package, bukan typedef.
fn resolve_pkg_param_width(
    name: &Symbol,
    package_symbols: &HashMap<Symbol, HashMap<Symbol, PackageItem>>,
) -> Option<i64> {
    for items in package_symbols.values() {
        if let Some(PackageItem::Param(p)) = items.get(name) {
            if let Some(expr) = &p.default {
                if let Ok(v) = const_eval_with_params(expr, &HashMap::new()) {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn resolve_dtype_width(
    dt: &DataType,
    package_symbols: &HashMap<Symbol, HashMap<Symbol, PackageItem>>,
) -> usize {
    match dt {
        DataType::UserDefined(name) => {
            for items in package_symbols.values() {
                if let Some(PackageItem::Typedef(td)) = items.get(name) {
                    return resolve_dtype_width(&td.dtype, package_symbols);
                }
            }
            64
        }
        DataType::EnumType { base, .. } => {
            if let Some(b) = base {
                resolve_dtype_width(b, package_symbols)
            } else {
                32
            }
        }
        _ => dt.width(),
    }
}
