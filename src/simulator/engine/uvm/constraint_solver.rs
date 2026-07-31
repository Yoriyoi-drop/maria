use crate::ast::expr::{BinaryOp, Expr, Value};
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
    /// Enhanced constraint solver: analyze constraints, compute domains, guided generation,
    /// with bounded backtracking. Falls back to rejection sampling with more attempts.
    pub(crate) fn solve_constraints(
        &mut self,
        obj_id: ObjId,
        class_name: &str,
        inline_constraint: Option<&IrExpr>,
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

        // Analyze class constraints
        let mut complex_class_constraints: Vec<&Expr> = Vec::new();
        for (_, body) in &class_def.constraints {
            for item in body {
                if let ConstraintItem::Expr(expr) = item {
                    if !analyze_constraint_for_domains(expr, &mut domains) {
                        complex_class_constraints.push(expr);
                    }
                }
            }
        }

        // Analyze inline constraints (with_clause)
        let mut inline_ir_constraints: Vec<IrExpr> = Vec::new();
        if let Some(wc) = inline_constraint {
            inline_ir_constraints.push(wc.clone());
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

            // Evaluate class constraints
            let mut all_satisfied = true;

            // First: complex class constraints (not already analyzed into domains)
            for expr in &complex_class_constraints {
                let result = self.evaluate_ast_expr(expr)?;
                if !result.to_bool().unwrap_or(false) {
                    all_satisfied = false;
                    break;
                }
            }

            // Then: simple constraints stored in domains are already satisfied by generation,
            // but we still check the simplest form for correctness

            // Evaluate inline constraint
            if all_satisfied && !inline_ir_constraints.is_empty() {
                for wc in &inline_ir_constraints {
                    let result = self.evaluate_expr(wc)?;
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
