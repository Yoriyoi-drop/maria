use crate::ast::expr::{BinaryOp, DistItem, DistWeight, Expr, Value};
use crate::ast::types::ConstraintItem;
use crate::diagnostics::DiagCode;
use crate::error::SimError;
use crate::ir::*;
use crate::simulator::engine::SimulationEngine;
use crate::Symbol;
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

impl VarDomain {
    fn generate(&self, seed: &mut u64, width: usize) -> LogicVec {
        if let Some(fixed) = self.fixed {
            return LogicVec::from_u64(fixed, width);
        }

        // If inside constraints define specific ranges, pick from those
            if !self.inside.is_empty() {
                let total_values: u64 = self.inside.iter().map(|r| r.hi.saturating_sub(r.lo) + 1).sum();
                if total_values < u64::MAX && total_values > 0 {
                    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    let pick = *seed % total_values;
                    let mut prev_accum = 0u64;
                    for range in &self.inside {
                        let range_width = range.hi - range.lo + 1;
                        let accum = prev_accum + range_width;
                        if pick < accum {
                            let offset = pick - prev_accum;
                            return LogicVec::from_u64(range.lo + offset, width);
                        }
                        prev_accum = accum;
                    }
                }
            }

        // Use bounds
        let lo = self.min.unwrap_or(0);
        let max_allowed = (1u64 << width.min(63)).saturating_sub(1);
        let hi = self.max.unwrap_or(max_allowed).min(max_allowed);
        if hi == 0 || hi < lo {
            // Fall back to full range
            *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            return LogicVec::from_u64(*seed >> (64 - width.min(32)), width);
        }

        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let range_size = hi.saturating_sub(lo) + 1;
        let val = lo + (*seed % range_size);

        // Ensure exclusion
        if self.exclude.contains(&val) && range_size > self.exclude.len() as u64 {
            *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let val2 = lo + (*seed % range_size);
            if !self.exclude.contains(&val2) {
                return LogicVec::from_u64(val2, width);
            }
        }

        LogicVec::from_u64(val, width)
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
                    // `x dist {...}` SEBAGAI CONSTRAINT = membership: nilai `x`
                    // harus berada di himpunan distribusi (nilai & range),
                    // BUKAN mengambil nilai acak dari distribusi (IrExpr::Dist
                    // dipakai saat dist menjadi RHS assignment).
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
                        if !ok {
                            return Ok(false);
                        }
                        continue;
                    }
                    let r = self.evaluate_ast_expr(e)?;
                    if !r.to_bool().unwrap_or(false) {
                        return Ok(false);
                    }
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

    /// Enhanced constraint solver: analyze constraints, compute domains, guided generation,
    /// with bounded backtracking. Falls back to rejection sampling with more attempts.
    pub(crate) fn solve_constraints(
        &mut self,
        obj_id: ObjId,
        class_name: &str,
        inline_constraint: Option<InlineConstraint<'_>>,
    ) -> Result<SolveResult, SimError> {
        let class_def = self.design.classes.get(class_name)
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

        // Analyze class constraints: ekstrak domain HANYA dari item ekspresi
        // level-atas yang sederhana. Item `if/else` (F12) bersifat kondisional
        // — tidak diekstrak ke domain, divalidasi penuh di eval_constraint_body.
        for (_, body) in &class_def.constraints {
            for item in body {
                if let ConstraintItem::Expr(expr) = item {
                    let _ = analyze_constraint_for_domains(expr, &mut domains);
                }
            }
        }

        // Analyze inline constraints (with_clause) — F17: AST inline
        // (jalur method class) ikut diekstrak domain-nya, sehingga field lebar
        // tetap terpandu guided generation (bukan rejection murni).
        if let Some(InlineConstraint::Ast(expr)) = inline_constraint {
            let _ = analyze_constraint_for_domains(expr, &mut domains);
        }
        let mut inline_constraints: Vec<InlineConstraint<'_>> = Vec::new();
        if let Some(wc) = inline_constraint {
            inline_constraints.push(wc);
        }

        // Step 4: Guided generation with bounded backtracking
        let max_attempts = 10_000u32;
        let mut seed = self.current_time;

        for _ in 0..max_attempts {
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
            let mut all_satisfied = true;
            for (_, body) in &class_def.constraints {
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
    for (_, body) in &class_def.constraints {
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
fn analyze_constraint_for_domains(expr: &Expr, domains: &mut HashMap<Symbol, VarDomain>) -> bool {
    match expr {
        // Equality: var == value
        Expr::BinaryOp { op: BinaryOp::Eq, lhs, rhs } => {
            let (var_name, value) = if let Expr::Ident { name, .. } = lhs.as_ref() {
                if let Some(val) = try_extract_u64(rhs) {
                    (*name, val)
                } else {
                    return false;
                }
            } else if let Expr::Ident { name, .. } = rhs.as_ref() {
                if let Some(val) = try_extract_u64(lhs) {
                    (*name, val)
                } else {
                    return false;
                }
            } else {
                return false;
            };

            if let Some(domain) = domains.get_mut(&var_name) {
                domain.fixed = Some(value);
                domain.min = Some(value);
                domain.max = Some(value);
            }
            true
        }

        // In-equality: var < value, var <= value, var > value, var >= value
        Expr::BinaryOp { op, lhs, rhs } if matches!(*op, BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge) => {
            let is_left_var = matches!(lhs.as_ref(), Expr::Ident { .. });
            let is_right_var = matches!(rhs.as_ref(), Expr::Ident { .. });

            if is_left_var && !is_right_var {
                let var_name = if let Expr::Ident { name, .. } = lhs.as_ref() { *name } else { return false; };
                let val = try_extract_u64(rhs);
                if val.is_none() { return false; }
                let val = val.unwrap();

                if let Some(domain) = domains.get_mut(&var_name) {
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
                let val = try_extract_u64(lhs);
                if val.is_none() { return false; }
                let val = val.unwrap();

                if let Some(domain) = domains.get_mut(&var_name) {
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
                    for item in items {
                        match item {
                            DistItem::Value(e, _) => {
                                if let Some(v) = try_extract_u64(e) {
                                    domain.inside.push(InsideRange { lo: v, hi: v });
                                    domain.min = Some(domain.min.map(|m| m.min(v)).unwrap_or(v));
                                    domain.max = Some(domain.max.map(|m| m.max(v)).unwrap_or(v));
                                } else {
                                    return false;
                                }
                            }
                            DistItem::Range(lo, hi, _) => {
                                if let (Some(a), Some(b)) = (try_extract_u64(lo), try_extract_u64(hi)) {
                                    let (l, h) = (a.min(b), a.max(b));
                                    domain.inside.push(InsideRange { lo: l, hi: h });
                                    domain.min = Some(domain.min.map(|m| m.min(l)).unwrap_or(l));
                                    domain.max = Some(domain.max.map(|m| m.max(h)).unwrap_or(h));
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
                    for item in range_list {
                        match item {
                            Expr::Value(Value::Decimal(v)) => {
                                let val = *v as u64;
                                domain.inside.push(InsideRange { lo: val, hi: val });
                                domain.min = Some(domain.min.map(|m| m.min(val)).unwrap_or(val));
                                domain.max = Some(domain.max.map(|m| m.max(val)).unwrap_or(val));
                            }
                            // Range expression: handle [lo:hi] parsed as a BinaryOp
                            Expr::BinaryOp { lhs, rhs, .. } => {
                                if let (Some(lo), Some(hi)) = (try_extract_u64(lhs), try_extract_u64(rhs)) {
                                    if lo <= hi {
                                        domain.inside.push(InsideRange { lo, hi });
                                        domain.min = Some(domain.min.map(|m| m.min(lo)).unwrap_or(lo));
                                        domain.max = Some(domain.max.map(|m| m.max(hi)).unwrap_or(hi));
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
                                if let (Some(a), Some(b)) = (try_extract_u64(msb), try_extract_u64(lsb)) {
                                    let (lo, hi) = (a.min(b), a.max(b));
                                    domain.inside.push(InsideRange { lo, hi });
                                    domain.min = Some(domain.min.map(|m| m.min(lo)).unwrap_or(lo));
                                    domain.max = Some(domain.max.map(|m| m.max(hi)).unwrap_or(hi));
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
            let (var_name, value) = if let Expr::Ident { name, .. } = lhs.as_ref() {
                (*name, try_extract_u64(rhs))
            } else if let Expr::Ident { name, .. } = rhs.as_ref() {
                (*name, try_extract_u64(lhs))
            } else {
                return false;
            };

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

/// Try to extract a u64 value from an expression (constant)
fn try_extract_u64(expr: &Expr) -> Option<u64> {
    match expr {
        Expr::Value(Value::Decimal(v)) => Some(*v as u64),
        Expr::Value(Value::Hex { bits, width: _, is_signed: _ })
        | Expr::Value(Value::Binary { bits, width: _, is_signed: _ })
        | Expr::Value(Value::Octal { bits, width: _, is_signed: _ }) => {
            if let Ok(v) = bits.parse::<u64>() {
                Some(v)
            } else {
                None
            }
        }
        _ => None,
    }
}
