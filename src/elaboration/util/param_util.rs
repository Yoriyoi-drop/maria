//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan util.rs (SRP Refactoring).
//! Tanggung jawab: Parameter resolution utilities.
//!
//! Fungsi:
//!   - collect_body_params()    — kumpulkan parameter dari body module
//!   - resolve_param_values_fn() — resolusi nilai parameter dengan override
//!
//! ──────────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;

use crate::ast::types::const_eval_with_params;
use crate::ast::types::string_to_i64;
use crate::ast::*;
use crate::intern::Symbol;

/// Kumpulkan parameter dari body module (termasuk yang ada di dalam generate block).
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

/// Resolusi nilai parameter module dengan dukungan override instance.
/// positional_overrides: parameter posisional berdasarkan index.
/// named_overrides: parameter berdasarkan nama.
pub fn resolve_param_values_fn(
    module: &Module,
    instance_overrides: &HashMap<Symbol, i64>,
) -> Result<HashMap<Symbol, i64>, String> {
    resolve_param_values_with_ctx(module, instance_overrides, &HashMap::new())
}

/// Resolusi nilai parameter module dengan context awal (package params dari import).
/// `base_ctx` berisi nilai yang sudah diketahui sebelumnya (misal parameter package
/// yang di-import ke module) dan dijadikan fallback saat mengevaluasi default param.
pub fn resolve_param_values_with_ctx(
    module: &Module,
    instance_overrides: &HashMap<Symbol, i64>,
    base_ctx: &HashMap<Symbol, i64>,
) -> Result<HashMap<Symbol, i64>, String> {
    let mut vals = base_ctx.clone();
    let mut positional_overrides: Vec<i64> = Vec::new();
    for (name, val) in instance_overrides {
        if name.starts_with("__param") {
            let idx: usize = name.as_str().trim_start_matches("__param").parse().unwrap_or(0);
            if idx >= positional_overrides.len() {
                positional_overrides.resize(idx + 1, 0);
            }
            positional_overrides[idx] = *val;
        }
    }

    /// Evaluasi default param menjadi `Some(i64)` hanya bila itu konstanta
    /// SKALAR. Localparam ARRAY (`localparam logic [63:0] RC [24] = '{...}` —
    /// default diparse sebagai `Expr::Concat` multi-elemen) mengembalikan None
    /// agar TIDAK masuk param_vals sebagai skalar 0 — kalau masuk, `RC[rnd]`
    /// salah di-resolve sebagai bit-select lebar 1 dan `RC[rnd][63:0]` gagal
    /// dengan "range select out of bounds: 63:0 on width 1". Array param
    /// didaftarkan sebagai signal const array oleh elaborator (lihat
    /// elaborate_module_with_params_and_type).
    let eval_param_default = |e: &Expr, existing_vals: &HashMap<Symbol, i64>| -> Option<i64> {
        match e {
            Expr::Concat(parts) if parts.len() > 1 => None,
            Expr::String(s) => Some(string_to_i64(s)),
            _ => const_eval_with_params(e, existing_vals).ok(),
        }
    };

    for param in collect_body_params(module) {
        if !vals.contains_key(&param.name) {
            match &param.default {
                Some(e) => {
                    if let Some(v) = eval_param_default(e, &vals) {
                        vals.insert(param.name, v);
                    }
                }
                None => {
                    vals.insert(param.name, 0);
                }
            }
        }
    }

    for (i, param) in module.params.iter().enumerate() {
        if param.is_localparam {
            if let Some(e) = &param.default {
                if let Some(v) = eval_param_default(e, &vals) {
                    vals.insert(param.name, v);
                }
            } else {
                vals.insert(param.name, 0);
            }
            continue;
        }
        let val = if i < positional_overrides.len() {
            positional_overrides[i]
        } else if let Some(override_val) = instance_overrides.get(&param.name) {
            *override_val
        } else {
            match &param.default {
                Some(e) => eval_param_default(e, &vals).unwrap_or(0),
                None => 0,
            }
        };
        vals.insert(param.name, val);
    }

    // ── Localparam ARRAY: flatten keys `name[i]` / `name[r][c]` ──
    // Agar ekspresi konstanta `RhoOffset[x][y]`, `RC[rnd]`,
    // `SeedInfoPageSel[idx]` bisa di-fold selama generate expansion
    // (design-level pass), keys array di-flatten ke nilai skalar DI SINI —
    // sebelum expand_all_generates memakai param_vals ini. Dulu flatten
    // hanya dilakukan di elaborate_module (setelah generate expansion),
    // sehingga const_eval gagal dan generate-if/for meleset.
    let mut array_srcs: Vec<(Symbol, Vec<Expr>)> = Vec::new();
    for param in collect_body_params(module) {
        if let Some(Expr::Concat(elems)) = &param.default {
            if elems.len() > 1 {
                array_srcs.push((param.name, elems.clone()));
            }
        }
    }
    for param in &module.params {
        if let Some(Expr::Concat(elems)) = &param.default {
            if elems.len() > 1 && !array_srcs.iter().any(|(n, _)| *n == param.name) {
                array_srcs.push((param.name, elems.clone()));
            }
        }
    }
    for (name, elems) in array_srcs {
        let is_2d = elems.iter().all(|e| matches!(e, Expr::Concat(_)));
        let mut flat_elems: Vec<Expr> = Vec::new();
        if is_2d {
            for e in &elems {
                if let Expr::Concat(row) = e {
                    flat_elems.extend(row.iter().cloned());
                }
            }
        } else {
            flat_elems.extend(elems.iter().cloned());
        }
        for (fi, e) in flat_elems.iter().enumerate() {
            if let Ok(v) = const_eval_with_params(e, &vals) {
                let key = if is_2d {
                    let cols = flat_elems.len() / elems.len();
                    format!("{}[{}][{}]", name.as_str(), fi / cols, fi % cols)
                } else {
                    format!("{}[{}]", name.as_str(), fi)
                };
                vals.insert(Symbol::intern(&key), v);
            }
        }
    }

    Ok(vals)
}
