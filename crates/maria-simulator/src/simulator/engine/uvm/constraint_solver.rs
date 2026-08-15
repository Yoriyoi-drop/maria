use maria_ast::expr::{BinaryOp, DistItem, DistWeight, Expr, UnaryOp, Value};
use maria_ast::types::ConstraintItem;
use maria_ast::types::is_signed_type;
use maria_core::diagnostics::DiagCode;
use maria_core::error::SimError;
use maria_ir::*;
use crate::simulator::engine::SimulationEngine;
use crate::simulator::util::map_ast_binary_op;
use crate::simulator::value::{eval_binary, eval_binary_signed};
use maria_core::Symbol;
use std::collections::{HashMap, HashSet};

/// Result from a constraint solve attempt
#[derive(Debug)]
pub(crate) enum SolveResult {
    Satisfied,
    Unsatisfiable,
}

/// Inline constraint `randomize() with {...}` — F17. Bisa berupa AST
/// (jalur method class: dievaluasi via `evaluate_ast_expr` dengan
/// `current_this` sehingga field class diakses langsung) atau IR (jalur
/// modul: sudah di-elaborate ke signal).
#[derive(Debug, Clone, Copy)]
pub(crate) enum InlineConstraint<'a> {
    Ast(&'a Expr),
    Ir(&'a IrExpr),
}

/// Analyzed domains for rand variables
#[derive(Debug, Clone, Default)]
struct VarDomain {
    /// Fixed value (from equality constraint like `a == 5`)
    fixed: Option<u64>,
    /// Lower bound (inclusive)
    min: Option<u64>,
    /// Upper bound (inclusive)
    max: Option<u64>,
    /// Inside values/ranges
    inside: Vec<InsideRange>,
    /// Excluded values (from `a != val`)
    exclude: HashSet<u64>,
}

#[derive(Debug, Clone)]
struct InsideRange {
    lo: u64,
    hi: u64,
}

/// Jumlah nilai dalam interval [lo, hi] pada domain u64 lebar `width`.
/// Interval WRAP (lo > hi, field signed) berarti [lo, max] ∪ [0, hi]
/// (dua's complement) — contoh [-5:5] = [0xFFFFFFFB, 5] = 11 nilai.
fn interval_count(lo: u64, hi: u64, width: usize) -> u64 {
    let max_allowed = (1u64 << width.min(63)).saturating_sub(1);
    if lo <= hi {
        hi - lo + 1
    } else {
        (max_allowed - lo + 1) + (hi + 1)
    }
}

/// Ambil nilai ke-`offset` (0-based) dari interval [lo, hi] (bisa WRAP).
fn interval_value(lo: u64, hi: u64, width: usize, offset: u64) -> u64 {
    let max_allowed = (1u64 << width.min(63)).saturating_sub(1);
    if lo <= hi {
        lo + offset
    } else {
        let first = max_allowed - lo + 1;
        if offset < first {
            lo + offset
        } else {
            offset - first
        }
    }
}

impl VarDomain {
    fn generate(&self, seed: &mut u64, width: usize) -> LogicVec {
        if let Some(fixed) = self.fixed {
            return LogicVec::from_u64(fixed, width);
        }

        // If inside constraints define specific ranges, pick from those.
        // Rentang bisa WRAP (lo > hi, field signed): [lo, max] ∪ [0, hi].
        if !self.inside.is_empty() {
            let total_values: u64 = self
                .inside
                .iter()
                .map(|r| interval_count(r.lo, r.hi, width))
                .sum();
            if total_values < u64::MAX && total_values > 0 {
                *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let pick = *seed % total_values;
                let mut prev_accum = 0u64;
                for range in &self.inside {
                    let range_width = interval_count(range.lo, range.hi, width);
                    let accum = prev_accum + range_width;
                    if pick < accum {
                        let offset = pick - prev_accum;
                        return LogicVec::from_u64(interval_value(range.lo, range.hi, width, offset), width);
                    }
                    prev_accum = accum;
                }
            }
        }

        // Use bounds (bisa WRAP untuk field signed: [lo, max] ∪ [0, hi])
        let lo = self.min.unwrap_or(0);
        let max_allowed = (1u64 << width.min(63)).saturating_sub(1);
        let hi = self.max.unwrap_or(max_allowed).min(max_allowed);
        let total = interval_count(lo, hi, width);
        if total > 0 && total < u64::MAX {
            *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let val = interval_value(lo, hi, width, *seed % total);

            // Ensure exclusion
            if self.exclude.contains(&val) && total > self.exclude.len() as u64 {
                *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let val2 = interval_value(lo, hi, width, *seed % total);
                if !self.exclude.contains(&val2) {
                    return LogicVec::from_u64(val2, width);
                }
            }
            return LogicVec::from_u64(val, width);
        }

        // Fall back to full range
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        LogicVec::from_u64(*seed >> (64 - width.min(32)), width)
    }
}

impl SimulationEngine {
    /// Evaluasi daftar item constraint secara rekursif (F12): ekspresi divalidasi
    /// via `evaluate_ast_expr`; `if/else` mengevaluasi kondisinya lalu menerapkan
    /// HANYA cabang yang terpenuhi; `solve-before` dilewati (urutan sudah
    /// diekstrak). Dipakai solver + rejection fallback — hasil `false` bila ada
    /// item yang tidak terpenuhi.
    pub(crate) fn eval_constraint_body(&mut self, items: &[ConstraintItem]) -> Result<bool, SimError> {
        for item in items {
            match item {
                ConstraintItem::Expr(e) => {
                    if !self.eval_constraint_expr(e, false)? {
                        return Ok(false);
                    }
                }
                ConstraintItem::Soft(e) => {
                    // LANG-31: constraint soft (best-effort) — dievaluasi
                    // sebagai preferensi, tapi pelanggaran DITOLERANSI:
                    // boleh dilanggar bila bertentangan dengan hard constraint
                    // (IEEE 1800-2017 §18.5.14).
                    let _ = self.eval_constraint_expr(e, true)?;
                }
                ConstraintItem::If { cond, then, els } => {
                    let c = self.evaluate_ast_expr(cond)?.to_bool().unwrap_or(false);
                    let branch = if c { then.as_slice() } else { els.as_slice() };
                    if !self.eval_constraint_body(branch)? {
                        return Ok(false);
                    }
                }
                ConstraintItem::SolveBefore { .. } => {}
            }
        }
        Ok(true)
    }

    /// Evaluasi SATU ekspresi constraint (F12): `x dist {...}` sebagai
    /// membership (nilai harus ada di himpunan distribusi), ekspresi lain
    /// sebagai Boolean. `tolerate=true` (LANG-31 soft): kegagalan dikembalikan
    /// sebagai `true` — pelanggaran soft ditoleransi; hard (`tolerate=false`)
    /// tetap wajib terpenuhi.
    pub(crate) fn eval_constraint_expr(&mut self, e: &Expr, tolerate: bool) -> Result<bool, SimError> {
        // `x dist {...}` SEBAGAI CONSTRAINT = membership: nilai `x` harus
        // berada di himpunan distribusi (nilai & range), BUKAN mengambil nilai
        // acak dari distribusi (IrExpr::Dist dipakai saat dist menjadi RHS
        // assignment).
        if let Expr::Dist { expr: inner, items: dis } = e {
            let val = self.evaluate_ast_expr(inner)?;
            let mut ok = false;
            for di in dis {
                match di {
                    DistItem::Value(v, _) => {
                        let vv = self.evaluate_ast_expr(v)?;
                        let w = val.width.max(vv.width);
                        if val.resize(w).case_eq(&vv.resize(w))
                            == LogicVec::from_u64(1, 1)
                        {
                            ok = true;
                            break;
                        }
                    }
                    DistItem::Range(lo, hi, _) => {
                        // ROUND 36: field signed → bandingkan di domain i64
                        // (to_i64 sign-extend dari lebar); unsigned → u64.
                        let signed = self.constraint_expr_signed(inner)?
                            && self.constraint_expr_signed(lo)?
                            && self.constraint_expr_signed(hi)?;
                        if signed {
                            let (a, b, v) = (
                                self.evaluate_ast_expr(lo)?.to_i64(),
                                self.evaluate_ast_expr(hi)?.to_i64(),
                                val.to_i64(),
                            );
                            if v >= a.min(b) && v <= a.max(b) {
                                ok = true;
                                break;
                            }
                        } else {
                            let a = self.evaluate_ast_expr(lo)?.to_u64();
                            let b = self.evaluate_ast_expr(hi)?.to_u64();
                            let v = val.to_u64();
                            if v >= a.min(b) && v <= a.max(b) {
                                ok = true;
                                break;
                            }
                        }
                    }
                }
            }
            if ok {
                return Ok(true);
            }
            return Ok(tolerate);
        }
        // ROUND 36: relational + div/mod pada constraint memakai signedness
        // operand — field class signed (dtype) dan literal desimal unsized
        // signed (LRM §11.8.2). evaluate_ast_expr memakai eval_binary
        // (unsigned) tanpa info tipe field → `rand int x; constraint
        // { x < 0; }` tak pernah terpenuhi (0xXXXXXXXX < 0 = false).
        if let Expr::BinaryOp { op, lhs, rhs } = e {
            if matches!(
                op,
                BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
                    | BinaryOp::Div
                    | BinaryOp::Mod
            ) {
                let lv = self.evaluate_ast_expr(lhs)?;
                let rv = self.evaluate_ast_expr(rhs)?;
                let ls = self.constraint_expr_signed(lhs)?;
                let rs = self.constraint_expr_signed(rhs)?;
                let ir_op = map_ast_binary_op(op)?;
                let r = if ls && rs {
                    eval_binary_signed(ir_op, &lv, &rv)
                } else {
                    eval_binary(ir_op, &lv, &rv)
                };
                return Ok(r.to_bool().unwrap_or(false) || tolerate);
            }
        }
        // ROUND 36: `inside` dengan operand SIGNED → compare di domain i64
        // (rentang [-5:5] = bits [0xFFFFFFFB, 5] wrap — urutan u64 min/max
        // salah utk signed: min(0xFFFFFFFB, 5) = 5). Nilai tunggal = bit
        // equality (sama utk signed/unsigned).
        if let Expr::Inside { expr: inner, range_list } = e {
            let val = self.evaluate_ast_expr(inner)?;
            let val_signed = self.constraint_expr_signed(inner)?;
            for item in range_list {
                if let Expr::RangeSelect { expr: base, msb, lsb, .. } = item {
                    if matches!(base.as_ref(), Expr::Value(Value::Decimal(0))) {
                        let a = self.evaluate_ast_expr(msb)?;
                        let b = self.evaluate_ast_expr(lsb)?;
                        if val_signed {
                            let (v, lo, hi) = (
                                val.to_i64(),
                                a.to_i64().min(b.to_i64()),
                                a.to_i64().max(b.to_i64()),
                            );
                            if v >= lo && v <= hi {
                                return Ok(true);
                            }
                        } else {
                            let (v, lo, hi) = (
                                val.to_u64(),
                                a.to_u64().min(b.to_u64()),
                                a.to_u64().max(b.to_u64()),
                            );
                            if v >= lo && v <= hi {
                                return Ok(true);
                            }
                        }
                        continue;
                    }
                }
                let item_val = self.evaluate_ast_expr(item)?;
                let w = val.width.max(item_val.width);
                if val.resize(w).case_eq(&item_val.resize(w))
                    == LogicVec::from_u64(1, 1)
                {
                    return Ok(true);
                }
            }
            return Ok(tolerate);
        }
        let r = self.evaluate_ast_expr(e)?;
        if r.to_bool().unwrap_or(false) {
            Ok(true)
        } else {
            Ok(tolerate)
        }
    }

    /// Signedness operand ekspresi CONSTRAINT (jalur AST): field class signed
    /// via dtype (current_this), literal desimal unsized signed (LRM §6.8.1),
    /// ekspresi majemuk rekursif (any-unsigned §11.8.2). `Ident` di luar
    /// konteks method / field tanpa dtype → unsigned (konservatif).
    fn constraint_expr_signed(&self, expr: &Expr) -> Result<bool, SimError> {
        Ok(match expr {
            Expr::Value(Value::Decimal(_)) => true,
            Expr::Value(Value::Binary { is_signed, .. })
            | Expr::Value(Value::Hex { is_signed, .. })
            | Expr::Value(Value::Octal { is_signed, .. }) => *is_signed,
            Expr::UnaryOp { expr: inner, .. } => self.constraint_expr_signed(inner)?,
            Expr::BinaryOp { lhs, rhs, .. } => {
                self.constraint_expr_signed(lhs)? && self.constraint_expr_signed(rhs)?
            }
            Expr::Ident { name, .. } => {
                let Some(obj_id) = self.current_this else {
                    return Ok(false);
                };
                let Some(obj) = self.state.objects.get(obj_id) else {
                    return Ok(false);
                };
                if obj.class_name == Symbol::EMPTY {
                    return Ok(false);
                }
                let Some(cd) = self.design.classes.get(&obj.class_name) else {
                    return Ok(false);
                };
                cd.fields
                    .iter()
                    .find(|f| f.name == *name)
                    .and_then(|f| f.dtype.as_ref())
                    .map(|d| is_signed_type(d))
                    .unwrap_or(false)
            }
            _ => false,
        })
    }

    /// Enhanced constraint solver: analyze constraints, compute domains, guided generation,
    /// with bounded backtracking. Falls back to rejection sampling with more attempts.
    pub(crate) fn solve_constraints(
        &mut self,
        obj_id: ObjId,
        class_name: &str,
        inline_constraint: Option<InlineConstraint<'_>>,
    ) -> Result<SolveResult, SimError> {
        let class_def = self.design.classes.get(&Symbol::intern(class_name))
            .ok_or_else(|| SimError::with_diag(
                DiagCode::NullHandle,
                format!("class '{}' not found", class_name),
            ))?
            .clone();

        if class_def.rand_fields.is_empty() {
            return Ok(SolveResult::Satisfied);
        }

        let old_this = self.current_this;
        self.current_this = Some(obj_id);

        // Step 1: Extract solve-before ordering
        let before_order = extract_before_order(&class_def);

        // Step 2: Order rand fields
        let ordered_fields = order_fields(&class_def.rand_fields, &before_order);

        // Step 3: Analyze constraints to build domains
        let mut domains: HashMap<Symbol, VarDomain> = HashMap::new();
        for fname in &class_def.rand_fields {
            domains.insert(*fname, VarDomain::default());
        }
        // ROUND 36: signature field (signed/width) utk narrowing domain signed.
        let mut field_sigs: HashMap<Symbol, FieldSig> = HashMap::new();
        for f in &class_def.fields {
            let signed = f.dtype.as_ref().map(|d| is_signed_type(d)).unwrap_or(false);
            field_sigs.insert(f.name, FieldSig { signed, width: f.width });
        }

        // Analyze class constraints: ekstrak domain HANYA dari item ekspresi
        // level-atas yang sederhana. Item `if/else` (F12) bersifat kondisional
        // — tidak diekstrak ke domain, divalidasi penuh di eval_constraint_body.
        // LANG-33: block yang di-disable via constraint_mode(0) di-skip.
        // LANG-32: block STATIC dicek global per-class (semua instance).
        let class_sym = Symbol::intern(class_name);
        for (block_name, is_static, body) in &class_def.constraints {
            if !self.constraint_block_enabled(obj_id, class_sym, *block_name, *is_static) {
                continue;
            }
            for item in body {
                if let ConstraintItem::Expr(expr) = item {
                    let _ = analyze_constraint_for_domains(expr, &mut domains, &field_sigs);
                }
            }
        }

        // Analyze inline constraints (with_clause) — F17: AST inline
        // (jalur method class) ikut diekstrak domain-nya, sehingga field lebar
        // tetap terpandu guided generation (bukan rejection murni).
        if let Some(InlineConstraint::Ast(expr)) = inline_constraint {
            let _ = analyze_constraint_for_domains(expr, &mut domains, &field_sigs);
        }
        let mut inline_constraints: Vec<InlineConstraint<'_>> = Vec::new();
        if let Some(wc) = inline_constraint {
            inline_constraints.push(wc);
        }

        // Step 4: Guided generation with bounded backtracking
        let max_attempts = 10_000u32;
        let mut seed = self.current_time;

        for attempt_n in 0..max_attempts {
            // Generate values for rand fields in order
            for fname in &ordered_fields {
                let domain = domains.get(fname).cloned().unwrap_or_default();
                let field_info = class_def.fields.iter().find(|f| &f.name == fname);
                let width = field_info.map(|f| f.width).unwrap_or(1);

                let val = domain.generate(&mut seed, width);
                if let Some(obj) = self.state.objects.get_mut(obj_id) {
                    obj.fields.insert(*fname, val);
                }
            }

            // Evaluate class constraints: evaluasi PENUH semua body (termasuk
            // item if/else F12). Domain sederhana sudah dijamin generasi
            // terpandu; evaluasi ulang penuh = correctness check.
            // LANG-33: block nonaktif (constraint_mode(0)) di-skip.
            // LANG-32: block STATIC dicek global per-class.
            let mut all_satisfied = true;
            for (block_name, is_static, body) in &class_def.constraints {
                if !self.constraint_block_enabled(obj_id, class_sym, *block_name, *is_static) {
                    continue;
                }
                if !self.eval_constraint_body(body)? {
                    all_satisfied = false;
                    break;
                }
            }

            // Evaluate inline constraint — F17: AST inline dievaluasi via
            // evaluate_ast_expr (current_this sudah di-set → field class
            // `addr` dkk ter-resolve), IR inline via evaluate_expr.
            if all_satisfied && !inline_constraints.is_empty() {
                for wc in &inline_constraints {
                    let result = match wc {
                        InlineConstraint::Ast(e) => self.evaluate_ast_expr(e)?,
                        InlineConstraint::Ir(ir) => self.evaluate_expr(ir)?,
                    };
                    if !result.to_bool().unwrap_or(false) {
                        all_satisfied = false;
                        break;
                    }
                }
            }

            if all_satisfied {
                self.current_this = old_this;
                return Ok(SolveResult::Satisfied);
            }
        }

        self.current_this = old_this;
        Ok(SolveResult::Unsatisfiable)
    }
}

/// Extract solve-before ordering from constraints
fn extract_before_order(class_def: &IrClassDef) -> HashMap<Symbol, HashSet<Symbol>> {
    let mut before_map: HashMap<Symbol, HashSet<Symbol>> = HashMap::new();
    for (_, _, body) in &class_def.constraints {
        for item in body {
            if let ConstraintItem::SolveBefore { vars } = item {
                if vars.len() >= 2 {
                    let first = &vars[0];
                    for later in &vars[1..] {
                        before_map.entry(*first)
                            .or_default()
                            .insert(*later);
                    }
                }
            }
        }
    }
    before_map
}

/// Order rand fields: solve-before fields come first, then remaining
fn order_fields(rand_fields: &[Symbol], before_map: &HashMap<Symbol, HashSet<Symbol>>) -> Vec<Symbol> {
    let mut ordered = Vec::new();
    let mut remaining: HashSet<Symbol> = rand_fields.iter().cloned().collect();

    for fname in rand_fields {
        if before_map.contains_key(fname) && remaining.contains(fname) {
            ordered.push(*fname);
            remaining.remove(fname);
        }
    }
    for fname in rand_fields {
        if remaining.contains(fname) {
            ordered.push(*fname);
        }
    }
    ordered
}

/// Try to extract domain information from a constraint expression.
/// Returns true if the constraint was fully analyzed.
/// `fields` = FieldSig per field class (signed/width) — dipakai narrowing
/// domain SIGNED (ROUND 36): `rand int x; constraint { x < 0; }` harus
/// narrow ke [-2^31, -1], bukan u64 [0, 0].
fn analyze_constraint_for_domains(
    expr: &Expr,
    domains: &mut HashMap<Symbol, VarDomain>,
    fields: &HashMap<Symbol, FieldSig>,
) -> bool {
    match expr {
        // Equality: var == value
        Expr::BinaryOp { op: BinaryOp::Eq, lhs, rhs } => {
            let var_name = if let Expr::Ident { name, .. } = lhs.as_ref() {
                *name
            } else if let Expr::Ident { name, .. } = rhs.as_ref() {
                *name
            } else {
                return false;
            };
            let other = if matches!(lhs.as_ref(), Expr::Ident { .. }) {
                rhs
            } else {
                lhs
            };
            // ROUND 36: field signed → ekstraksi i64 (nilai negatif `-3`),
            // bit di-mask ke lebar field; unsigned → u64.
            let value = extract_bits(other, fields.get(&var_name).copied());

            if let Some(value) = value {
                if let Some(domain) = domains.get_mut(&var_name) {
                    domain.fixed = Some(value);
                    domain.min = Some(value);
                    domain.max = Some(value);
                }
                true
            } else {
                false
            }
        }

        // In-equality: var < value, var <= value, var > value, var >= value
        Expr::BinaryOp { op, lhs, rhs } if matches!(*op, BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge) => {
            let is_left_var = matches!(lhs.as_ref(), Expr::Ident { .. });
            let is_right_var = matches!(rhs.as_ref(), Expr::Ident { .. });

            if is_left_var && !is_right_var {
                let var_name = if let Expr::Ident { name, .. } = lhs.as_ref() { *name } else { return false; };

                if let Some(domain) = domains.get_mut(&var_name) {
                    // ROUND 36: field SIGNED → narrow domain di rentang dua
                    // complement (x < 0 → [-2^31, -1], x > -10 → [-9, 2^31-1])
                    // dengan merge signed-aware; unsigned → u64 lama.
                    if let Some(sig) = fields.get(&var_name).copied() {
                        if sig.signed {
                            if let Some(v) = try_extract_i64(rhs) {
                                let (full_min, full_max) = signed_range(sig.width);
                                match op {
                                    BinaryOp::Lt => {
                                        domain.min = merge_min(domain.min, full_min, sig.width);
                                        domain.max = merge_max(
                                            domain.max,
                                            masked_bits(v.wrapping_sub(1), sig.width),
                                            sig.width,
                                        );
                                    }
                                    BinaryOp::Le => {
                                        domain.min = merge_min(domain.min, full_min, sig.width);
                                        domain.max = merge_max(
                                            domain.max,
                                            masked_bits(v, sig.width),
                                            sig.width,
                                        );
                                    }
                                    BinaryOp::Gt => {
                                        domain.min = merge_min(
                                            domain.min,
                                            masked_bits(v.wrapping_add(1), sig.width),
                                            sig.width,
                                        );
                                        domain.max = merge_max(domain.max, full_max, sig.width);
                                    }
                                    BinaryOp::Ge => {
                                        domain.min = merge_min(
                                            domain.min,
                                            masked_bits(v, sig.width),
                                            sig.width,
                                        );
                                        domain.max = merge_max(domain.max, full_max, sig.width);
                                    }
                                    _ => unreachable!(),
                                }
                                return true;
                            }
                            return false;
                        }
                    }
                    let val = try_extract_u64(rhs);
                    if val.is_none() { return false; }
                    let val = val.unwrap();
                    match op {
                        BinaryOp::Lt => {
                            let new_max = val.saturating_sub(1);
                            domain.max = Some(domain.max.map(|m| m.min(new_max)).unwrap_or(new_max));
                        }
                        BinaryOp::Le => {
                            domain.max = Some(domain.max.map(|m| m.min(val)).unwrap_or(val));
                        }
                        BinaryOp::Gt => {
                            let new_min = val.saturating_add(1);
                            domain.min = Some(domain.min.map(|m| m.max(new_min)).unwrap_or(new_min));
                        }
                        BinaryOp::Ge => {
                            domain.min = Some(domain.min.map(|m| m.max(val)).unwrap_or(val));
                        }
                        _ => unreachable!(),
                    }
                }
                true
            } else if !is_left_var && is_right_var {
                let var_name = if let Expr::Ident { name, .. } = rhs.as_ref() { *name } else { return false; };

                if let Some(domain) = domains.get_mut(&var_name) {
                    if let Some(sig) = fields.get(&var_name).copied() {
                        if sig.signed {
                            if let Some(v) = try_extract_i64(lhs) {
                                let (full_min, full_max) = signed_range(sig.width);
                                // `v < x` → x > v; `v >= x` → x <= v; dst.
                                match op {
                                    BinaryOp::Lt => {
                                        domain.min = merge_min(
                                            domain.min,
                                            masked_bits(v.wrapping_add(1), sig.width),
                                            sig.width,
                                        );
                                        domain.max = merge_max(domain.max, full_max, sig.width);
                                    }
                                    BinaryOp::Le => {
                                        domain.min = merge_min(
                                            domain.min,
                                            masked_bits(v, sig.width),
                                            sig.width,
                                        );
                                        domain.max = merge_max(domain.max, full_max, sig.width);
                                    }
                                    BinaryOp::Gt => {
                                        domain.min = merge_min(domain.min, full_min, sig.width);
                                        domain.max = merge_max(
                                            domain.max,
                                            masked_bits(v.wrapping_sub(1), sig.width),
                                            sig.width,
                                        );
                                    }
                                    BinaryOp::Ge => {
                                        domain.min = merge_min(domain.min, full_min, sig.width);
                                        domain.max = merge_max(
                                            domain.max,
                                            masked_bits(v, sig.width),
                                            sig.width,
                                        );
                                    }
                                    _ => unreachable!(),
                                }
                                return true;
                            }
                            return false;
                        }
                    }
                    let val = try_extract_u64(lhs);
                    if val.is_none() { return false; }
                    let val = val.unwrap();
                    match op {
                        BinaryOp::Lt => {
                            // var > val
                            let new_min = val.saturating_add(1);
                            domain.min = Some(domain.min.map(|m| m.max(new_min)).unwrap_or(new_min));
                        }
                        BinaryOp::Le => {
                            // var >= val
                            domain.min = Some(domain.min.map(|m| m.max(val)).unwrap_or(val));
                        }
                        BinaryOp::Gt => {
                            // var < val
                            let new_max = val.saturating_sub(1);
                            domain.max = Some(domain.max.map(|m| m.min(new_max)).unwrap_or(new_max));
                        }
                        BinaryOp::Ge => {
                            // var <= val
                            domain.max = Some(domain.max.map(|m| m.min(val)).unwrap_or(val));
                        }
                        _ => unreachable!(),
                    }
                }
                true
            } else {
                false
            }
        }

        // Dist: var dist { v := w, [lo:hi] :/ w } — nilai/range distribusi
        // diekstrak ke domain.inside (guided generation), sama seperti inside.
        // Tanpa ini, field dist hanya bisa dipenuhi rejection sampling — gagal
        // total untuk ruang nilai lebar (mis. 32-bit) walau satisfiable.
        Expr::Dist { expr, items } => {
            if let Expr::Ident { name, .. } = expr.as_ref() {
                if let Some(domain) = domains.get_mut(name) {
                    let sig = fields.get(name).copied();
                    for item in items {
                        match item {
                            DistItem::Value(e, _) => {
                                if let Some(v) = extract_bits(e, sig) {
                                    domain.inside.push(InsideRange { lo: v, hi: v });
                                    merge_interval(domain, v, v, sig);
                                } else {
                                    return false;
                                }
                            }
                            DistItem::Range(lo, hi, _) => {
                                if let (Some(a), Some(b)) =
                                    (extract_bits(lo, sig), extract_bits(hi, sig))
                                {
                                    // signed → urutkan via sign_extend; unsigned
                                    // → u64 biasa.
                                    let (l, h) = match sig {
                                        Some(s) if s.signed => {
                                            let (la, lb) = (sign_extend(a, s.width), sign_extend(b, s.width));
                                            (masked_bits(la.min(lb), s.width), masked_bits(la.max(lb), s.width))
                                        }
                                        _ => (a.min(b), a.max(b)),
                                    };
                                    domain.inside.push(InsideRange { lo: l, hi: h });
                                    merge_interval(domain, l, h, sig);
                                } else {
                                    return false;
                                }
                            }
                        }
                    }
                    return true;
                }
            }
            false
        }

        // Inside: var inside { range_list }
        Expr::Inside { expr, range_list } => {
            if let Expr::Ident { name, .. } = expr.as_ref() {
                if let Some(domain) = domains.get_mut(name) {
                    let sig = fields.get(name).copied();
                    for item in range_list {
                        match item {
                            // ROUND 36: nilai tunggal — termasuk literal NEGATIF
                            // (`-3` = UnaryOp Minus) yang sebelumnya jatuh ke
                            // `_ => return false` (analisis inside di-putus di
                            // tengah → domain parsial). extract_bits menangani
                            // keduanya via try_extract_i64/u64.
                            Expr::Value(Value::Decimal(_)) | Expr::UnaryOp { .. } => {
                                let val = match extract_bits(item, sig) {
                                    Some(v) => v,
                                    None => {
                                        if let Expr::Value(Value::Decimal(d)) = item {
                                            *d as u64
                                        } else {
                                            return false;
                                        }
                                    }
                                };
                                domain.inside.push(InsideRange { lo: val, hi: val });
                                merge_interval(domain, val, val, sig);
                            }
                            Expr::Value(Value::Hex { .. })
                            | Expr::Value(Value::Binary { .. })
                            | Expr::Value(Value::Octal { .. }) => {
                                if let Some(val) = extract_bits(item, sig) {
                                    domain.inside.push(InsideRange { lo: val, hi: val });
                                    merge_interval(domain, val, val, sig);
                                } else {
                                    return false;
                                }
                            }
                            // Range expression: handle [lo:hi] parsed as a BinaryOp
                            Expr::BinaryOp { lhs, rhs, .. } => {
                                if let (Some(lo), Some(hi)) =
                                    (extract_bits(lhs, sig), extract_bits(rhs, sig))
                                {
                                    // ROUND 36: field signed → rentang dua
                                    // complement wrap-around valid ([-5:5] =
                                    // [0xFFFFFFFB, 5]); unsigned → u64 biasa.
                                    let (lo, hi) = match sig {
                                        Some(s) if s.signed => {
                                            let (la, lb) = (sign_extend(lo, s.width), sign_extend(hi, s.width));
                                            (masked_bits(la.min(lb), s.width), masked_bits(la.max(lb), s.width))
                                        }
                                        _ => (lo.min(hi), lo.max(hi)),
                                    };
                                    let valid = match sig {
                                        Some(s) if s.signed => true,
                                        _ => lo <= hi,
                                    };
                                    if valid {
                                        domain.inside.push(InsideRange { lo, hi });
                                        merge_interval(domain, lo, hi, sig);
                                    }
                                } else {
                                    return false;
                                }
                            }
                            // Range `[a:b]` di dalam inside di-parse sebagai
                            // RangeSelect dgn base `Value(Decimal(0))` (lihat
                            // parser/expr.rs Token::Inside) — evaluasi
                            // msb/lsb sebagai rentang. Base selain 0 = member
                            // select asli (`inside {x[7:0]}`) → jangan
                            // diperlakukan sebagai rentang.
                            Expr::RangeSelect { expr: base, msb, lsb, .. }
                                if matches!(base.as_ref(), Expr::Value(Value::Decimal(0))) =>
                            {
                                if let (Some(a), Some(b)) =
                                    (extract_bits(msb, sig), extract_bits(lsb, sig))
                                {
                                    // ROUND 36: signed → urutkan di domain i64
                                    // VIA sign_extend (a/b sudah MASKED u64 —
                                    // `a as i64` memberi 4294967291, bukan -5!).
                                    // [-5:5] → lo=0xFFFFFFFB, hi=5 (WRAP) —
                                    // generate menangani interval wrap.
                                    // Unsigned → u64 biasa.
                                    let (lo, hi) = match sig {
                                        Some(s) if s.signed => {
                                            let (l, h) = (
                                                sign_extend(a, s.width).min(sign_extend(b, s.width)),
                                                sign_extend(a, s.width).max(sign_extend(b, s.width)),
                                            );
                                            (masked_bits(l, s.width), masked_bits(h, s.width))
                                        }
                                        _ => (a.min(b), a.max(b)),
                                    };
                                    domain.inside.push(InsideRange { lo, hi });
                                    merge_interval(domain, lo, hi, sig);
                                } else {
                                    return false;
                                }
                            }
                            _ => return false,
                        }
                    }
                    return true;
                }
            }
            false
        }

        // Not-equal: var != value
        Expr::BinaryOp { op: BinaryOp::Neq, lhs, rhs } => {
            let var_name = if let Expr::Ident { name, .. } = lhs.as_ref() {
                *name
            } else if let Expr::Ident { name, .. } = rhs.as_ref() {
                *name
            } else {
                return false;
            };
            let other = if matches!(lhs.as_ref(), Expr::Ident { .. }) {
                rhs
            } else {
                lhs
            };
            let value = extract_bits(other, fields.get(&var_name).copied());

            if let Some(val) = value {
                if let Some(domain) = domains.get_mut(&var_name) {
                    domain.exclude.insert(val);
                }
                true
            } else {
                false
            }
        }

        // Complex constraints that can't be analyzed statically
        _ => false,
    }
}

/// Info tipe field rand untuk analisis domain signed (ROUND 36).
#[derive(Debug, Clone, Copy)]
struct FieldSig {
    signed: bool,
    width: usize,
}

/// Ekstrak nilai konstanta ke bit domain (mask lebar field): signed → i64
/// (menangani `-3` = UnaryOp Minus), unsigned → u64.
fn extract_bits(expr: &Expr, sig: Option<FieldSig>) -> Option<u64> {
    match sig {
        Some(s) if s.signed => try_extract_i64(expr).map(|v| masked_bits(v, s.width)),
        _ => try_extract_u64(expr),
    }
}

/// Ekstrak nilai i64 (menangani `-3` = UnaryOp Minus desimal) — dipakai utk
/// field SIGNED agar `x < -3`, `y > -10` bisa di-narrow domain-nya.
fn try_extract_i64(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::Value(Value::Decimal(v)) => Some(*v),
        Expr::UnaryOp {
            op: UnaryOp::Minus,
            expr: inner,
        } => try_extract_i64(inner).map(i64::wrapping_neg),
        _ => None,
    }
}

/// Masker bit two's complement ke lebar field (nilai signed negatif → bit
/// pattern di domain u64 solver).
fn masked_bits(v: i64, width: usize) -> u64 {
    if width >= 64 {
        v as u64
    } else {
        (v as u64) & ((1u64 << width) - 1)
    }
}

/// Rentang penuh field signed `[full_min, full_max]` sebagai MASKED bits
/// (dua's complement lebar `width`): min = 0x800..0 (=-2^(W-1)), max =
/// 0x7FF..F (=+2^(W-1)-1). KESALAHAN UMUM: `(1 << W) - 1` = 0xFF..F yang
/// di-domain signed adalah -1, BUKAN +2^(W-1)-1 — membuat bound atas field
/// signed selalu ter-cap di -1 (lihat ROUND 36: `x > -10` + `x < 100`).
fn signed_range(width: usize) -> (u64, u64) {
    if width >= 64 {
        (1u64 << 63, (1u64 << 63) - 1)
    } else {
        (1u64 << (width - 1), (1u64 << (width - 1)) - 1)
    }
}

/// Interpret bit pattern two's complement (masked, lebar `width`) sebagai
/// i64 SIGNED — utk membandingkan bound yang tersimpan sebagai u64 masked
/// (mis. 0xFFFFFFFB = -5 pada 32-bit). `bits as i64` tanpa ini memberi
/// 4294967291, bukan -5.
fn sign_extend(bits: u64, width: usize) -> i64 {
    if width >= 64 {
        bits as i64
    } else {
        let shift = 64 - width;
        ((bits << shift) as i64) >> shift
    }
}

/// Gabung bound bawah (min): ambil yang lebih KETAT — lebih besar secara
/// SIGNED (bit pattern di-sign-extend dari lebar `width`).
fn merge_min(cur: Option<u64>, new: u64, width: usize) -> Option<u64> {
    Some(match cur {
        None => new,
        Some(c) => {
            if sign_extend(c, width) >= sign_extend(new, width) {
                c
            } else {
                new
            }
        }
    })
}

/// Gabung bound atas (max): ambil yang lebih KETAT — lebih kecil secara
/// SIGNED (bit pattern di-sign-extend dari lebar `width`).
fn merge_max(cur: Option<u64>, new: u64, width: usize) -> Option<u64> {
    Some(match cur {
        None => new,
        Some(c) => {
            if sign_extend(c, width) <= sign_extend(new, width) {
                c
            } else {
                new
            }
        }
    })
}

/// Gabung satu interval [lo, hi] (bisa WRAP utk signed) ke bound domain:
/// signed → merge_min/max sign-extended; unsigned → u64 min/max biasa.
fn merge_interval(domain: &mut VarDomain, lo: u64, hi: u64, sig: Option<FieldSig>) {
    match sig {
        Some(s) if s.signed => {
            domain.min = merge_min(domain.min, lo, s.width);
            domain.max = merge_max(domain.max, hi, s.width);
        }
        _ => {
            domain.min = Some(domain.min.map(|m| m.min(lo)).unwrap_or(lo));
            domain.max = Some(domain.max.map(|m| m.max(hi)).unwrap_or(hi));
        }
    }
}

/// Try to extract a u64 value from an expression (constant)
fn try_extract_u64(expr: &Expr) -> Option<u64> {
    match expr {
        Expr::Value(Value::Decimal(v)) => Some(*v as u64),
        // ROUND 36: parse sesuai radix literal — `8'hF0` punya bits "F0"
        // yang TIDAK bisa di-parse sebagai desimal (`parse::<u64>()` gagal)
        // → domain `u > 8'hF0` kosong → rejection sampling 32-bit praktis
        // mustahil di combined constraints.
        Expr::Value(Value::Hex { bits, width: _, is_signed: _ }) => {
            u64::from_str_radix(bits, 16).ok()
        }
        Expr::Value(Value::Binary { bits, width: _, is_signed: _ }) => {
            u64::from_str_radix(bits, 2).ok()
        }
        Expr::Value(Value::Octal { bits, width: _, is_signed: _ }) => {
            u64::from_str_radix(bits, 8).ok()
        }
        _ => None,
    }
}
