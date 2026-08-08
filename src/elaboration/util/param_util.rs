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

use crate::ast::const_eval_ext::{eval_param_default_full, SField, Scalars};
use crate::ast::types::const_eval_with_params;
use crate::ast::types::string_to_i64;
use crate::ast::types::PackageItem;
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

/// Konteks package untuk evaluasi PENUH default param: konstanta package
/// ter-evaluasi (qualified `pkg::name` — skalar & array, hasil
/// `eval_package_constants`) dan `package_symbols` (typedef/fungsi package).
/// Dipakai `resolve_param_values_with_ctx` sebagai fallback saat evaluator
/// skalar sederhana gagal — mis. `$bits(typedef)` atau inlining fungsi package
/// seperti `otbn_pkg::SecAddRandWidth`, yang tidak bisa dievaluasi oleh
/// `const_eval_with_params`.
pub struct PkgFullCtx<'a> {
    pub scalars: &'a HashMap<Symbol, i64>,
    pub arrays: &'a HashMap<Symbol, Vec<i64>>,
    pub package_symbols: &'a HashMap<Symbol, HashMap<Symbol, PackageItem>>,
    /// Nilai struct localparam/parameter (assignment pattern) agar
    /// `name.field` bisa di-const-eval (mis. `Info.size`).
    pub structs: &'a HashMap<Symbol, Vec<SField>>,
}

/// Evaluasi default param menjadi `Some(i64)` hanya bila itu konstanta
/// SKALAR. Localparam ARRAY (`localparam logic [63:0] RC [24] = '{...}` —
/// default diparse sebagai `Expr::Concat` multi-elemen) mengembalikan None
/// agar TIDAK masuk param_vals sebagai skalar 0 — kalau masuk, `RC[rnd]`
/// salah di-resolve sebagai bit-select lebar 1 dan `RC[rnd][63:0]` gagal
/// dengan "range select out of bounds: 63:0 on width 1". Array param
/// didaftarkan sebagai signal const array oleh elaborator (lihat
/// elaborate_module_with_params_and_type).
///
/// Jalur cepat `const_eval_with_params` (skalar + $clog2 + vbits + operator),
/// lalu fallback ke evaluator penuh (`$bits(typedef)`, fungsi package, scoped
/// ident) bila konteks package tersedia.
fn eval_param_default(
    e: &Expr,
    existing_vals: &HashMap<Symbol, i64>,
    merged: &Option<Scalars>,
    pkg: Option<&PkgFullCtx>,
) -> Option<i64> {
    match e {
        Expr::Concat(parts) if parts.len() > 1 => None,
        // JANGAN masukkan struct literal ke param_vals sebagai skalar 0.
        // `const_eval_with_params` mengembalikan Ok(0) untuk `Expr::StructLit`
        // (agar array-of-struct lama tetap terdaftar), tapi untuk struct
        // NON-array (mis. `localparam info_t Info = '{size: ..., off: ...};`)
        // nilai 0 itu membuat loop body-param di elaborator melewati `Info`
        // (sudah ada di effective_params) sehingga `struct_vals` tidak pernah
        // diisi dan `Info.size` gagal dengan "member access on non-struct".
        // Serahkan ke evaluator struct (eval_cval_full) di loop body-param.
        Expr::StructLit { .. } => None,
        Expr::String(s) => Some(string_to_i64(s)),
        _ => {
            if let Ok(v) = const_eval_with_params(e, existing_vals) {
                return Some(v);
            }
            if let (Some(m), Some(p)) = (merged, pkg) {
                return eval_param_default_full(e, m, p.arrays, p.package_symbols, p.structs);
            }
            None
        }
    }
}

/// Resolusi nilai parameter module dengan dukungan override instance.
/// positional_overrides: parameter posisional berdasarkan index.
/// named_overrides: parameter berdasarkan nama.
pub fn resolve_param_values_fn(
    module: &Module,
    instance_overrides: &HashMap<Symbol, i64>,
) -> Result<HashMap<Symbol, i64>, String> {
    resolve_param_values_with_ctx(module, instance_overrides, &HashMap::new(), None)
}

/// Resolusi nilai parameter module dengan context awal (package params dari import).
/// `base_ctx` berisi nilai yang sudah diketahui sebelumnya (misal parameter package
/// yang di-import ke module) dan dijadikan fallback saat mengevaluasi default param.
///
/// `pkg` (opsional) membuka evaluasi PENUH: `$bits(typedef)`, inlining fungsi
/// package, dan scoped ident `pkg::item` pada default param. Konteks gabungan
/// (konstanta package + module ctx) dibangun SEKALI per module — bukan per
/// param — agar clone konstanta package besar tidak menjadi bottleneck.
pub fn resolve_param_values_with_ctx(
    module: &Module,
    instance_overrides: &HashMap<Symbol, i64>,
    base_ctx: &HashMap<Symbol, i64>,
    pkg: Option<&PkgFullCtx>,
) -> Result<HashMap<Symbol, i64>, String> {
    let mut vals = base_ctx.clone();
    // Konteks evaluasi penuh: konstanta package (qualified) + module ctx
    // (plain — module menang). Dijaga sinkron dengan `vals` pada setiap insert
    // (module param bisa direferensikan oleh localparam berikutnya).
    let mut merged: Option<Scalars> = pkg.map(|p| {
        let mut m: Scalars = p.scalars.clone();
        for (&k, &v) in base_ctx {
            m.insert(k, v);
        }
        m
    });
    // Insert ke `vals` dan (bila ada) `merged` sekaligus.
    macro_rules! insert_val {
        ($k:expr, $v:expr) => {{
            vals.insert($k, $v);
            if let Some(m) = merged.as_mut() {
                m.insert($k, $v);
            }
        }};
    }
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

    for param in collect_body_params(module) {
        if !vals.contains_key(&param.name) {
            match &param.default {
                Some(e) => {
                    if let Some(v) = eval_param_default(e, &vals, &merged, pkg) {
                        insert_val!(param.name, v);
                    }
                }
                None => {
                    insert_val!(param.name, 0);
                }
            }
        }
    }

    for (i, param) in module.params.iter().enumerate() {
        if param.is_localparam {
            if let Some(e) = &param.default {
                if let Some(v) = eval_param_default(e, &vals, &merged, pkg) {
                    insert_val!(param.name, v);
                }
            } else {
                insert_val!(param.name, 0);
            }
            continue;
        }
        let val = if i < positional_overrides.len() {
            positional_overrides[i]
        } else if let Some(override_val) = instance_overrides.get(&param.name) {
            *override_val
        } else {
            match &param.default {
                Some(e) => eval_param_default(e, &vals, &merged, pkg).unwrap_or(0),
                None => 0,
            }
        };
        insert_val!(param.name, val);
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
                insert_val!(Symbol::intern(&key), v);
            }
        }
    }

    Ok(vals)
}
