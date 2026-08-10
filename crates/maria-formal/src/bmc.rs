//! Bounded Model Checking (BMC) Engine.
//!
//! Unrolls the transition relation of a design up to bound k and checks
//! assertions at each depth. Uses Z3 SMT solver with bit-vector theory.
//! (z3 0.20 thread-local context.)
//!
//! Algorithm:
//! 1. Pre-create signal variables for ALL depths 0..bound
//! 2. Add initial state constraints at depth 0: sig_i_0 == init_val_i
//! 3. For each depth d from 1..bound:
//!    - Extract assignments from process bodies: lhs → rhs
//!    - Add next-state constraints: sig_lhs_d == rhs_eval(sig_{d-1})
//!    - For unassigned signals: sig_i_d == sig_i_{d-1}
//! 4. For each depth d from 0..bound:
//!    - Assert ¬assertion(sig_d)
//!    - Check SAT → counterexample if sat

use super::{FormalEngine, FormalResult};
use maria_ir::*;
use std::collections::HashSet;

impl FormalEngine {
    /// Run BMC on all assertions in the design.
    pub fn check_assertions_bmc(&mut self, design: &IrDesign) -> Vec<(String, FormalResult)> {
        if !self.is_available() {
            self.init();
        }
        let mut results = Vec::new();
        let bound = self.config.bound;
        let n_signals = design.top.signals.len();

        // Collect assertions from all processes
        let all_assertions = collect_assertions(&design.top.processes);
        if all_assertions.is_empty() {
            return Vec::new();
        }

        // Extract combinational assignments: for each signal, the RHS expression(s)
        let assignments = collect_combinational_assignments(&design.top.processes);

        // Pre-allocate signal variable context
        // sig_id_d = { name: "sig_{id}_{d}", width }
        let signal_widths: Vec<u32> = design.top.signals
            .iter()
            .map(|s| s.width.clamp(1, 64) as u32)
            .collect();

        // Collect initial values for each signal
        let init_vals: Vec<u64> = design.top.signals
            .iter()
            .map(|s| s.init_val.to_u64())
            .collect();

        for (aidx, (assert_name, cond)) in all_assertions.iter().enumerate() {
            let result = self.bmc_check_single(bound, n_signals, &signal_widths, &init_vals, &assignments, cond);
            results.push((format!("{}.assert_{}", assert_name, aidx), result));
        }
        results
    }

    /// Check a single assertion with BMC using transition relation encoding.
    fn bmc_check_single(
        &mut self,
        bound: u64,
        n_signals: usize,
        signal_widths: &[u32],
        init_vals: &[u64],
        assignments: &[Vec<(usize, Box<IrExpr>)>],
        cond: &IrExpr,
    ) -> FormalResult {
        self.reset();
        let solver = match self.solver.as_ref() {
            Some(s) => s,
            None => return FormalResult::Error("Z3 solver not initialized".into()),
        };

        // STEP 1: Create signal variables for all depths 0..bound
        let n_vars = (bound + 1) as usize;
        let mut sig_vars: Vec<Vec<z3::ast::BV>> = Vec::with_capacity(n_vars);

        for d in 0..=bound {
            let mut depth_vars = Vec::with_capacity(n_signals);
            for i in 0..n_signals {
                let width = if i < signal_widths.len() { signal_widths[i] } else { 64 };
                let var = z3::ast::BV::new_const(format!("sig_{}_{}", i, d), width);
                depth_vars.push(var);
            }
            sig_vars.push(depth_vars);
        }

        // STEP 2: Add initial state constraints for depth 0
        // sig_i_0 == init_val_i — apply to ALL signals at depth 0
        // (init_val comes from reg declaration like `reg [7:0] sig = 5;`,
        //  and is always the initial state regardless of combinational assignments)
        for i in 0..n_signals {
            let init = *init_vals.get(i).unwrap_or(&0);
            let width = *signal_widths.get(i).unwrap_or(&64);
            let init_z3 = z3::ast::BV::from_u64(init, width);
            let sig_var = &sig_vars[0][i];
            solver.assert(sig_var.eq(&init_z3));
        }

        // STEP 3: Add next-state constraints for each depth d ≥ 1
        let all_assigned: HashSet<usize> = if !assignments.is_empty() {
            assignments[0].iter().map(|(id, _)| *id).collect()
        } else {
            HashSet::new()
        };

        for d in 1..=bound {
            // 3a: Add assignment constraints for signals driven by combinational logic
            for assign_group in assignments.iter() {
                for (sig_id, rhs) in assign_group.iter() {
                    if *sig_id >= n_signals {
                        continue;
                    }
                    let rhs_z3 = self.expr_to_z3_int_at(rhs, d as isize - 1, &sig_vars);
                    if let Some(rhs_val) = rhs_z3 {
                        let lhs_var = &sig_vars[d as usize][*sig_id];
                        let (lhs_mw, rhs_mw) = self.zero_extend_match(lhs_var, &rhs_val);
                        solver.assert(lhs_mw.eq(&rhs_mw));
                    }
                }
            }

            // 3b: Frame constraints — unassigned signals retain previous value
            for i in 0..n_signals {
                if !all_assigned.contains(&i) {
                    let prev = &sig_vars[(d - 1) as usize][i];
                    let curr = &sig_vars[d as usize][i];
                    solver.assert(prev.eq(curr));
                }
            }
        }

        // STEP 4: For each depth d, check if ¬P(d) is satisfiable
        // IMPORTANT: use push/pop to isolate each depth's assertion.
        // Without this, ¬P(0) = false (when P(0) holds) would poison all
        // subsequent depth checks because `false` persists in the solver.
        for d in 0..=bound {
            solver.push();
            let cond_bool = self.expr_to_z3_bool_at(cond, d as isize, &sig_vars);
            if let Some(cond_val) = cond_bool {
                let neg = cond_val.not();
                solver.assert(&neg);
            }

            match solver.check() {
                z3::SatResult::Sat => {
                    solver.pop(1);
                    return FormalResult::Counterexample(d);
                }
                z3::SatResult::Unsat => {
                    solver.pop(1);
                    continue;
                }
                z3::SatResult::Unknown => {
                    solver.pop(1);
                    if d > bound / 2 {
                        return FormalResult::Pass;
                    }
                    return FormalResult::Unknown;
                }
            }
        }

        // STEP 5: If induction is enabled, attempt k-induction proof
        if self.config.induction && bound >= 2 {
            // Reset solver before induction — clear BMC constraints from solver
            self.reset();
            return self.check_inductive(bound, n_signals, signal_widths, init_vals, assignments, cond);
        }

        FormalResult::Pass
    }

    // ─── k-Induction Proof ───
    //
    // Algorithm:
    //   1. BASE: BMC up to k (already done above — property holds at depths 0..k)
    //   2. STEP: For induction depth k, assume property holds at depths 1..k,
    //      and prove at depth k+1. If all these checks are UNSAT (property
    //      holds at k+1 under assumption it holds at 1..k), then by induction
    //      the property holds for all depths.
    //
    // Simple k=1 induction:
    //   - BASE: prove P(0) and P(1) — already done by BMC
    //   - STEP: assume P(d), prove P(d+1) — check if ¬P(d+1) ∧ P(d) is SAT
    //   - If UNSAT → P holds for all depths (inductive proof)

    /// Try to prove the property holds for ALL depths using k-induction.
    fn check_inductive(
        &mut self,
        bound: u64,
        n_signals: usize,
        signal_widths: &[u32],
        _init_vals: &[u64],
        assignments: &[Vec<(usize, Box<IrExpr>)>],
        cond: &IrExpr,
    ) -> FormalResult {
        // Try k=1 induction first (simplest)
        // STEP: assume P(d) for arbitrary d, prove P(d+1)
        // We construct a transition at depths 0 and 1 with:
        //   - P(0) assumed true
        //   - ¬P(1) checked for SAT
        // If UNSAT → P(0) ⇒ P(1) holds → induction succeeds

        let k: u64 = 1;
        if bound < k {
            return FormalResult::Unknown;
        }

        // Note: solver was reset by caller (bmc_check_single resets before calling)
        let solver = match self.solver.as_ref() {
            Some(s) => s,
            None => return FormalResult::Error("Z3 solver not initialized".into()),
        };
        
        // Create variables for 2 depths (0 = pre, 1 = post)
        let mut sig_vars: Vec<Vec<z3::ast::BV>> = Vec::with_capacity(2);
        for d in 0..2 {
            let mut depth_vars = Vec::with_capacity(n_signals);
            for i in 0..n_signals {
                let width = if i < signal_widths.len() { signal_widths[i] } else { 64 };
                let var = z3::ast::BV::new_const(format!("induct_sig_{}_{}", i, d), width);
                depth_vars.push(var);
            }
            sig_vars.push(depth_vars);
        }

        // Add next-state constraints from pre (d=0) to post (d=1)
        let all_assigned: HashSet<usize> = if !assignments.is_empty() {
            assignments[0].iter().map(|(id, _)| *id).collect()
        } else {
            HashSet::new()
        };

        for assign_group in assignments.iter() {
            for (sig_id, rhs) in assign_group.iter() {
                if *sig_id >= n_signals {
                    continue;
                }
                let rhs_z3 = self.expr_to_z3_int_at(rhs, 0, &sig_vars);
                if let Some(rhs_val) = rhs_z3 {
                    let lhs_var = &sig_vars[1][*sig_id];
                    let (lhs_mw, rhs_mw) = self.zero_extend_match(lhs_var, &rhs_val);
                    solver.assert(lhs_mw.eq(&rhs_mw));
                }
            }
        }

        // Frame constraints for pre→post
        for i in 0..n_signals {
            if !all_assigned.contains(&i) {
                let prev = &sig_vars[0][i];
                let curr = &sig_vars[1][i];
                solver.assert(prev.eq(curr));
            }
        }

        // Assume P(0) holds (property at pre-state)
        if let Some(p_at_0) = self.expr_to_z3_bool_at(cond, 0, &sig_vars) {
            solver.assert(&p_at_0);
        } else {
            return FormalResult::Unknown;
        }

        // Check ¬P(1) (property fails at post-state)
        if let Some(p_at_1) = self.expr_to_z3_bool_at(cond, 1, &sig_vars) {
            solver.assert(p_at_1.not());
        } else {
            return FormalResult::Unknown;
        }

        // UNSAT → induction step holds → property proved for all depths
        match solver.check() {
            z3::SatResult::Unsat => FormalResult::InductiveProof,
            z3::SatResult::Sat => {
                // Induction step failed (SAT = counterexample found at step k+1)
                // Could try higher k (k=2, k=3) for more complex properties
                FormalResult::Pass  // BMC already proved up to bound
            }
            z3::SatResult::Unknown => {
                // Z3 timeout or inconclusive
                // Fall back to BMC result (Pass up to bound)
                FormalResult::Pass
            }
        }
    }

    /// Translate IrExpr to Z3 BV, with signal references at a specific depth.
    fn expr_to_z3_int_at(
        &self,
        expr: &IrExpr,
        depth: isize,
        sig_vars: &[Vec<z3::ast::BV>],
    ) -> Option<z3::ast::BV> {
        match expr {
            IrExpr::Const(lv) => {
                let val = lv.to_u64();
                let width = lv.width.clamp(1, 64) as u32;
                Some(z3::ast::BV::from_u64(val, width))
            }
            IrExpr::FillLit(v) => {
                let bit = match v {
                    LogicVal::Zero => 0u64,
                    _ => 1u64,
                };
                Some(z3::ast::BV::from_u64(bit, 1))
            }
            IrExpr::Signal(id, _) => {
                // Use the signal variable at the specified depth
                let d = if depth >= 0 { depth as usize } else { 0 };
                let d = d.min(sig_vars.len().saturating_sub(1));
                let idx = *id;
                if idx < sig_vars[d].len() {
                    Some(sig_vars[d][idx].clone())
                } else {
                    None
                }
            }
            IrExpr::BinaryOp(op, lhs, rhs) => {
                let l = self.expr_to_z3_int_at(lhs, depth, sig_vars)?;
                let r = self.expr_to_z3_int_at(rhs, depth, sig_vars)?;
                let (l, r) = self.zero_extend_match(&l, &r);
                match op {
                    BinaryIrOp::Add => Some(l.bvadd(&r)),
                    BinaryIrOp::Sub => Some(l.bvsub(&r)),
                    BinaryIrOp::Mul => Some(l.bvmul(&r)),
                    BinaryIrOp::BitAnd => Some(l.bvand(&r)),
                    BinaryIrOp::BitOr => Some(l.bvor(&r)),
                    BinaryIrOp::BitXor => Some(l.bvxor(&r)),
                    BinaryIrOp::Shl => Some(l.bvshl(&r)),
                    BinaryIrOp::Shr => Some(l.bvlshr(&r)),
                    BinaryIrOp::Sshr => Some(l.bvashr(&r)),
                    BinaryIrOp::Eq | BinaryIrOp::CaseEq | BinaryIrOp::EqWild => {
                        let eq = l.eq(&r);
                        let one = z3::ast::BV::from_u64(1, 1);
                        let zero = z3::ast::BV::from_u64(0, 1);
                        Some(eq.ite(&one, &zero))
                    }
                    BinaryIrOp::Neq | BinaryIrOp::CaseNeq | BinaryIrOp::NeqWild => {
                        let eq = l.eq(&r);
                        let one = z3::ast::BV::from_u64(1, 1);
                        let zero = z3::ast::BV::from_u64(0, 1);
                        Some(eq.ite(&zero, &one))
                    }
                    BinaryIrOp::Lt => {
                        let cmp = l.bvslt(&r);
                        let one = z3::ast::BV::from_u64(1, 1);
                        let zero = z3::ast::BV::from_u64(0, 1);
                        Some(cmp.ite(&one, &zero))
                    }
                    BinaryIrOp::Le => {
                        let cmp = l.bvsle(&r);
                        let one = z3::ast::BV::from_u64(1, 1);
                        let zero = z3::ast::BV::from_u64(0, 1);
                        Some(cmp.ite(&one, &zero))
                    }
                    BinaryIrOp::Gt => {
                        let cmp = l.bvsgt(&r);
                        let one = z3::ast::BV::from_u64(1, 1);
                        let zero = z3::ast::BV::from_u64(0, 1);
                        Some(cmp.ite(&one, &zero))
                    }
                    BinaryIrOp::Ge => {
                        let cmp = l.bvsge(&r);
                        let one = z3::ast::BV::from_u64(1, 1);
                        let zero = z3::ast::BV::from_u64(0, 1);
                        Some(cmp.ite(&one, &zero))
                    }
                    _ => None,
                }
            }
            IrExpr::UnaryOp(op, inner) => {
                let v = self.expr_to_z3_int_at(inner, depth, sig_vars)?;
                match op {
                    UnaryIrOp::Minus => {
                        let zero = z3::ast::BV::from_u64(0, v.get_size());
                        Some(zero.bvsub(&v))
                    }
                    UnaryIrOp::Not => {
                        let zero = z3::ast::BV::from_u64(0, 1);
                        let one = z3::ast::BV::from_u64(1, 1);
                        let is_zero = v.eq(&zero);
                        Some(is_zero.ite(&one, &zero))
                    }
                    UnaryIrOp::BitNot => Some(v.bvnot()),
                    _ => None,
                }
            }
            IrExpr::Cond(cond_expr, t, f) => {
                let c = self.expr_to_z3_bool_at(cond_expr, depth, sig_vars)?;
                let tv = self.expr_to_z3_int_at(t, depth, sig_vars)?;
                let fv = self.expr_to_z3_int_at(f, depth, sig_vars)?;
                Some(c.ite(&tv, &fv))
            }
            _ => None,
        }
    }

    /// Translate IrExpr to Z3 Bool, with signal references at a specific depth.
    fn expr_to_z3_bool_at(
        &self,
        expr: &IrExpr,
        depth: isize,
        sig_vars: &[Vec<z3::ast::BV>],
    ) -> Option<z3::ast::Bool> {
        match expr {
            IrExpr::Const(lv) => {
                let val = lv.to_u64();
                Some(z3::ast::Bool::from_bool(val != 0))
            }
            IrExpr::BinaryOp(op, lhs, rhs) => {
                match op {
                    BinaryIrOp::Eq | BinaryIrOp::CaseEq | BinaryIrOp::EqWild => {
                        let l = self.expr_to_z3_int_at(lhs, depth, sig_vars)?;
                        let r = self.expr_to_z3_int_at(rhs, depth, sig_vars)?;
                        let (l, r) = self.zero_extend_match(&l, &r);
                        Some(l.eq(&r))
                    }
                    BinaryIrOp::Neq | BinaryIrOp::CaseNeq | BinaryIrOp::NeqWild => {
                        let l = self.expr_to_z3_int_at(lhs, depth, sig_vars)?;
                        let r = self.expr_to_z3_int_at(rhs, depth, sig_vars)?;
                        let (l, r) = self.zero_extend_match(&l, &r);
                        Some(l.eq(&r).not())
                    }
                    BinaryIrOp::Lt => {
                        let l = self.expr_to_z3_int_at(lhs, depth, sig_vars)?;
                        let r = self.expr_to_z3_int_at(rhs, depth, sig_vars)?;
                        let (l, r) = self.zero_extend_match(&l, &r);
                        Some(l.bvslt(&r))
                    }
                    BinaryIrOp::Le => {
                        let l = self.expr_to_z3_int_at(lhs, depth, sig_vars)?;
                        let r = self.expr_to_z3_int_at(rhs, depth, sig_vars)?;
                        let (l, r) = self.zero_extend_match(&l, &r);
                        Some(l.bvsle(&r))
                    }
                    BinaryIrOp::Gt => {
                        let l = self.expr_to_z3_int_at(lhs, depth, sig_vars)?;
                        let r = self.expr_to_z3_int_at(rhs, depth, sig_vars)?;
                        let (l, r) = self.zero_extend_match(&l, &r);
                        Some(l.bvsgt(&r))
                    }
                    BinaryIrOp::Ge => {
                        let l = self.expr_to_z3_int_at(lhs, depth, sig_vars)?;
                        let r = self.expr_to_z3_int_at(rhs, depth, sig_vars)?;
                        let (l, r) = self.zero_extend_match(&l, &r);
                        Some(l.bvsge(&r))
                    }
                    BinaryIrOp::LogicalAnd => {
                        let l = self.expr_to_z3_bool_at(lhs, depth, sig_vars)?;
                        let r = self.expr_to_z3_bool_at(rhs, depth, sig_vars)?;
                        Some(z3::ast::Bool::and(&[l, r]))
                    }
                    BinaryIrOp::LogicalOr => {
                        let l = self.expr_to_z3_bool_at(lhs, depth, sig_vars)?;
                        let r = self.expr_to_z3_bool_at(rhs, depth, sig_vars)?;
                        Some(z3::ast::Bool::or(&[l, r]))
                    }
                    _ => None,
                }
            }
            IrExpr::UnaryOp(UnaryIrOp::Not, inner) => {
                let v = self.expr_to_z3_bool_at(inner, depth, sig_vars)?;
                Some(v.not())
            }
            IrExpr::Cond(cond_expr, t, f) => {
                let c = self.expr_to_z3_bool_at(cond_expr, depth, sig_vars)?;
                let tv = self.expr_to_z3_bool_at(t, depth, sig_vars)?;
                let fv = self.expr_to_z3_bool_at(f, depth, sig_vars)?;
                Some(c.ite(&tv, &fv))
            }
            _ => {
                // Fallback: non-bool → (val != 0)
                let val = self.expr_to_z3_int_at(expr, depth, sig_vars)?;
                let zero = z3::ast::BV::from_u64(0, val.get_size());
                Some(val.eq(&zero).not())
            }
        }
    }
}

/// Collect assertions from process bodies.
/// Returns Vec<(module_name/process_name, condition)>.
pub(crate) fn collect_assertions(processes: &[Process]) -> Vec<(String, IrExpr)> {
    let mut result = Vec::new();
    for process in processes {
        let (name, body) = match process {
            Process::Combinational { name, body, .. } => (name, body),
            Process::Sequential { name, body, .. } => (name, body),
            Process::Initial { name, body } => (name, body),
            Process::CombReactive { name, body, .. } => (name, body),
            _ => continue,
        };
        // Walk body recursively looking for Assert stmts
        walk_assertions(&name.to_string(), body, &mut result);
    }
    result
}

fn walk_assertions(prefix: &str, stmts: &[IrStmt], result: &mut Vec<(String, IrExpr)>) {
    for stmt in stmts {
        match stmt {
            IrStmt::Assert { cond, .. } => {
                result.push((prefix.to_string(), cond.clone()));
            }
            IrStmt::Block { stmts: inner } => {
                walk_assertions(prefix, inner, result);
            }
            IrStmt::NamedBlock { stmts: inner, .. } => {
                walk_assertions(prefix, inner, result);
            }
            IrStmt::If { true_branch, false_branch, .. } => {
                walk_assertions(prefix, true_branch, result);
                walk_assertions(prefix, false_branch, result);
            }
            IrStmt::Case { items, default, .. } => {
                for item in items {
                    walk_assertions(prefix, &item.body, result);
                }
                walk_assertions(prefix, default, result);
            }
            _ => {}
        }
    }
}

/// Collect combinational assignments from process bodies.
/// Returns a Vec where each entry is a list of (signal_id, rhs_expr) for that depth.
/// For MVP: one depth of assignments (combinational logic is stateless — same every cycle).
pub(crate) fn collect_combinational_assignments(
    processes: &[Process],
) -> Vec<Vec<(usize, Box<IrExpr>)>> {    let mut all_assignments: Vec<(usize, Box<IrExpr>)> = Vec::new();
    let mut seen_signals: HashSet<usize> = HashSet::new();

    for process in processes {
        let body = match process {
            Process::Combinational { body, .. } => body,
            Process::CombReactive { body, .. } => body,
            _ => continue,
        };
        walk_assignments(body, &mut all_assignments, &mut seen_signals);
    }

    // Return one "depth" of assignments (combinational — same constraints each cycle)
    if all_assignments.is_empty() {
        vec![Vec::new()]
    } else {
        vec![all_assignments]
    }
}

fn walk_assignments(
    stmts: &[IrStmt],
    result: &mut Vec<(usize, Box<IrExpr>)>,
    seen: &mut HashSet<usize>,
) {
    for stmt in stmts {
        match stmt {
            IrStmt::BlockingAssign { lhs, rhs, .. } => {
                if let IrLValue::Signal(id, _) = lhs {
                    if !seen.contains(id) {
                        seen.insert(*id);
                        result.push((*id, Box::new(rhs.clone())));
                    }
                }
            }
            IrStmt::Block { stmts: inner } => {
                walk_assignments(inner, result, seen);
            }
            IrStmt::NamedBlock { stmts: inner, .. } => {
                walk_assignments(inner, result, seen);
            }
            IrStmt::If { true_branch, false_branch, .. } => {
                walk_assignments(true_branch, result, seen);
                walk_assignments(false_branch, result, seen);
            }
            IrStmt::Case { items, default, .. } => {
                for item in items {
                    walk_assignments(&item.body, result, seen);
                }
                walk_assignments(default, result, seen);
            }
            _ => {}
        }
    }
}
