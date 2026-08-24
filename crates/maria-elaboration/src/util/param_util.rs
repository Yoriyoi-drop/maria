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

use maria_ast::const_eval_ext::{eval_param_default_full, CVal, SField};
use maria_ast::types::const_eval_with_params;
use maria_ast::types::string_to_i64;
use maria_ast::types::PackageItem;
use maria_ast::*;
use maria_core::intern::Symbol;

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

/// Apakah default param berupa koleksi (array literal Concat multi-elemen /
/// struct literal) yang TIDAK boleh didaftarkan sebagai skalar fallback.
/// `eval_param_default` mengembalikan None untuk keduanya; guard ini
/// mencegah fallback 1 meng-klaim nama yang sebenarnya array/struct (yang
/// didaftarkan lewat jalur flatten array / evaluator struct terpisah).
fn param_default_is_collection(e: &Expr) -> bool {
    matches!(e, Expr::Concat(parts) if parts.len() > 1) || matches!(e, Expr::StructLit { .. })
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
///
/// `existing_vals` berisi konteks skalar penuh: pemanggil menjamin
/// `base_ctx ⊇ pkg_param_ctx ⊇ pkg_const_scalars` (collect_package_param_ctx
/// meng-clone pkg_param_ctx yang sudah meng-flatten konstanta package), jadi
/// `existing_vals` bisa dipakai langsung sebagai `merged` untuk evaluator
/// penuh — TIDAK perlu meng-clone ulang `pkg.scalars` + merge base_ctx per
/// module (bottleneck ~40k+63k insert per module di OpenTitan).
fn eval_param_default(
    e: &Expr,
    existing_vals: &HashMap<Symbol, i64>,
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
            if let Some(p) = pkg {
                return eval_param_default_full(
                    e,
                    existing_vals,
                    p.arrays,
                    p.package_symbols,
                    p.structs,
                );
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
    let _t0 = std::time::Instant::now();
    let mut vals = base_ctx.clone();
    let _t_clone_vals = _t0.elapsed();
    // Konteks skalar penuh = `vals` itu sendiri: pemanggil menjamin
    // base_ctx ⊇ pkg_param_ctx ⊇ pkg_const_scalars (collect_package_param_ctx
    // meng-clone pkg_param_ctx yang meng-flatten konstanta package), jadi
    // tidak perlu map `merged` terpisah (dulu clone pkg.scalars + merge
    // base_ctx per module → bottleneck di OpenTitan).
    // Insert ke `vals`.
    macro_rules! insert_val {
        ($k:expr, $v:expr) => {{
            vals.insert($k, $v);
        }};
    }
    let mut positional_overrides: Vec<i64> = Vec::new();
    for (name, val) in instance_overrides {
        if name.starts_with("__param") {
            let idx: usize = name
                .as_str()
                .trim_start_matches("__param")
                .parse()
                .unwrap_or(0);
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
                    if let Some(v) = eval_param_default(e, &vals, pkg) {
                        insert_val!(param.name, v);
                    } else if !param_default_is_collection(e) {
                        // Fallback global: param gagal dievaluasi (default
                        // `'x`, `$bits(member)` tak ter-resolve, referensi
                        // package DPI tak ada — mis. `VccPokStrNum` di dalam
                        // generate, `TRANSFER_BYTES_WIDTH = $bits(...)`) tetap
                        // didaftarkan dengan nilai 1 agar generate for-limit /
                        // width resolution tidak gagal "not found in parameter
                        // context". Array/struct literal dikecualikan (jalur
                        // array di bawah / evaluator struct yang menangani).
                        insert_val!(param.name, 1);
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
                if let Some(v) = eval_param_default(e, &vals, pkg) {
                    insert_val!(param.name, v);
                } else if !param_default_is_collection(e) {
                    // Fallback global sama seperti body params di atas.
                    insert_val!(param.name, 1);
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
                Some(e) => eval_param_default(e, &vals, pkg).unwrap_or(0),
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

    // Passthrough member keys struct dari override instance (`Info.integrity`
    // dst. — hasil flatten override struct di elaborate_module_with_params_and_type).
    // Loop param di atas hanya menyalin override untuk nama param module;
    // key `param.field` (mengandung `.`) harus ikut agar generate if / width
    // member access di child bisa const-eval (pola `Info.integrity` di
    // otp_ctrl_part_buf — tanpa ini gagal "member access not allowed").
    for (k, v) in instance_overrides {
        if k.as_str().contains('.') && !vals.contains_key(k) {
            insert_val!(*k, *v);
        }
    }

    // ── Member keys struct untuk DEFAULT param (`Info = PartInfoDefault`) ──
    // `eval_param_default` mengubah default struct menjadi skalar 0 sehingga
    // `Info.size`/`Info.integrity` tidak pernah terdaftar. Dengan `pkg.structs`
    // yang berisi index struct package global, salin fields default struct ke
    // `param.field` keys — sama seperti override instance. Tanpa ini, module
    // yang TIDAK di-override (default dipakai) tetap gagal member access di
    // generate if / port width (pola otp_ctrl_part_buf).
    if let Some(p) = pkg {
        let mut struct_defaults: Vec<(Symbol, Expr)> = Vec::new();
        for param in collect_body_params(module) {
            if let Some(e) = &param.default {
                struct_defaults.push((param.name, e.clone()));
            }
        }
        for param in &module.params {
            if let Some(e) = &param.default {
                struct_defaults.push((param.name, e.clone()));
            }
        }
        for (pname, e) in struct_defaults {
            // Hanya default berbentuk ident / scoped ident / bitsel array
            // element yang mereferensikan konstanta struct package.
            let base: Option<String> = match &e {
                Expr::Ident { name, .. } => Some(name.as_str().to_string()),
                Expr::ScopedIdent { item, .. } => Some(item.as_str().to_string()),
                Expr::BitSelect { expr: inner, index } => {
                    if let Expr::Ident { name, .. } = inner.as_ref() {
                        if let Ok(idx) = const_eval_with_params(index, &vals) {
                            Some(format!("{}[{}]", name.as_str(), idx))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };
            let Some(base) = base else { continue };
            let Some(fields) = p.structs.get(&Symbol::intern(base.as_str())) else {
                continue;
            };
            let mut key_created = false;
            for f in fields {
                if let Some(fname) = f.name {
                    if let CVal::Scalar(v) = f.val {
                        let key = format!("{}.{}", pname.as_str(), fname.as_str());
                        let key_sym = Symbol::intern(&key);
                        if !vals.contains_key(&key_sym) {
                            insert_val!(key_sym, v);
                            key_created = true;
                        }
                    }
                }
            }
            if key_created {
                // Pastikan nama param terdaftar juga (skalar fallback 0)
                // bila belum — key `param.field` butuh `param` ada.
                if !vals.contains_key(&pname) {
                    insert_val!(pname, 0);
                }
            }
        }
    }

    if std::env::var("DBG_ELAB").is_ok() {
        eprintln!(
            "[DBG-RESOLVE] module '{}' total={}us clone_vals={}us",
            module.name.as_str(),
            _t0.elapsed().as_micros(),
            _t_clone_vals.as_micros()
        );
    }
    Ok(vals)
}
