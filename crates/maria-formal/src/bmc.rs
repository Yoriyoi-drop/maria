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

/// Hasil satu langkah k-induction pada kedalaman k.
enum InductionOutcome {
    /// UNSAT — invariant terbukti untuk semua depth.
    Proved,
    /// SAT — counterexample palsu di kedalaman ini; butuh k lebih dalam.
    Spurious,
    /// Z3 unknown / kondisi tidak dapat diterjemahkan.
    Inconclusive,
    /// Kesalahan internal (solver hilang).
    Error(String),
}

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

        // FORMAL-10 (tahap 1): kumpulkan asumsi (`assume`) — di-constrain
        // pada setiap depth sehingga counterexample hanya dilaporkan bila
        // reachable DI BAWAH asumsi (semantik formal assume-guarantee).
        let assumptions = collect_assumptions(&design.top.processes);

        // FIX BUG (ROUND 91): deklarasi initializer (`reg x = 1;`) di-elaborasi
        // menjadi proses Initial `decl_init_*` — TIDAK masuk init_val maupun
        // assignments kombinational. Kumpulkan sebagai constraint state awal.
        let init_assigns = collect_initial_state_assignments(&design.top.processes);

        // Pre-allocate signal variable context
        // sig_id_d = { name: "sig_{id}_{d}", width }
        let signal_widths: Vec<u32> = design
            .top
            .signals
            .iter()
            .map(|s| s.width.clamp(1, 64) as u32)
            .collect();

        // Collect initial values for each signal.
        // FIX BUG (ROUND 91): init_val yang mengandung X/Z (sinyal tanpa
        // initializer deklarasi — LogicVec::new = X-fill / wire = Z-fill)
        // TIDAK boleh dipaksa ke to_u64() (X dibaca 0) → constraint awal
        // palsu → false counterexample di depth 0. Sinyal tidak terdefinisi
        // dibiarkan unconstrained di depth 0 (abstraksi 2-state yang sound).
        let init_vals: Vec<Option<u64>> = design
            .top
            .signals
            .iter()
            .map(|s| {
                let has_unknown = s
                    .init_val
                    .bits
                    .iter()
                    .any(|b| matches!(b, LogicVal::X | LogicVal::Z));
                if has_unknown {
                    None
                } else {
                    Some(s.init_val.to_u64())
                }
            })
            .collect();

        for (aidx, (assert_name, cond)) in all_assertions.iter().enumerate() {
            let result = self.bmc_check_single(
                bound,
                n_signals,
                &signal_widths,
                &init_vals,
                &assignments,
                &init_assigns,
                &assumptions,
                cond,
            );
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
        init_vals: &[Option<u64>],
        assignments: &[Vec<(usize, Box<IrExpr>)>],
        init_assigns: &[(usize, IrExpr)],
        assumptions: &[IrExpr],
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
                let width = if i < signal_widths.len() {
                    signal_widths[i]
                } else {
                    64
                };
                let var = z3::ast::BV::new_const(format!("sig_{}_{}", i, d), width);
                depth_vars.push(var);
            }
            sig_vars.push(depth_vars);
        }

        // STEP 2: Add initial state constraints for depth 0
        // sig_i_0 == init_val_i — hanya untuk sinyal dengan init terdefinisi
        // penuh (tanpa X/Z); sinyal lain unconstrained (lihat catatan FIX di
        // atas).
        for i in 0..n_signals {
            if let Some(Some(init)) = init_vals.get(i) {
                let width = *signal_widths.get(i).unwrap_or(&64);
                let init_z3 = z3::ast::BV::from_u64(*init, width);
                let sig_var = &sig_vars[0][i];
                solver.assert(sig_var.eq(&init_z3));
            }
        }

        // STEP 2b: constraint state awal dari initializer deklarasi
        // (proses Initial `decl_init_*`) — sig_0 == rhs pada depth 0.
        for (sig_id, rhs) in init_assigns {
            if *sig_id >= n_signals {
                continue;
            }
            if let Some(rhs_val) = self.expr_to_z3_int_at(rhs, 0, &sig_vars) {
                let lhs_var = &sig_vars[0][*sig_id];
                let (lhs_mw, rhs_mw) = self.zero_extend_match(lhs_var, &rhs_val);
                solver.assert(lhs_mw.eq(&rhs_mw));
            }
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

        // STEP 2c: logika kombinational berlaku JUGA di depth 0 utk sinyal
        // tanpa nilai awal terdefinisi (always_comb meng-drive kontinu —
        // nilai di waktu 0 ditentukan inputnya, bukan bebas).
        let init_defined: HashSet<usize> = init_vals
            .iter()
            .enumerate()
            .filter(|(_, v)| v.is_some())
            .map(|(i, _)| i)
            .chain(init_assigns.iter().map(|(id, _)| *id))
            .collect();
        if let Some(group) = assignments.first() {
            for (sig_id, rhs) in group {
                if *sig_id >= n_signals || init_defined.contains(sig_id) {
                    continue;
                }
                if let Some(rhs_val) = self.expr_to_z3_int_at(rhs, 0, &sig_vars) {
                    let lhs_var = &sig_vars[0][*sig_id];
                    let (lhs_mw, rhs_mw) = self.zero_extend_match(lhs_var, &rhs_val);
                    solver.assert(lhs_mw.eq(&rhs_mw));
                }
            }
        }

        // FORMAL-10: asumsi berlaku pada SETIAP depth — di-assert DI LUAR
        // push/pop depth check agar persisten melintasi semua iterasi.
        for a in assumptions {
            for d in 0..=bound {
                if let Some(a_bool) = self.expr_to_z3_bool_at(a, d as isize, &sig_vars) {
                    solver.assert(&a_bool);
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
            return self.check_inductive(
                bound,
                n_signals,
                signal_widths,
                init_vals,
                assignments,
                cond,
            );
        }

        FormalResult::Pass
    }

    // ─── k-Induction Proof ───
    //
    // Algorithm (generalized k-induction, FORMAL-03/04):
    //   For k = 1..max_k:
    //     1. BASE: BMC already proved P holds at depths 0..bound with the
    //        real initial state (done above).
    //     2. STEP: build k+1 unconstrained frames F0..Fk connected by the
    //        transition relation, ASSUME P(F0)..P(Fk-1), then check ¬P(Fk):
    //          - UNSAT → no reachable-in-k-steps state violates P while all
    //            predecessors satisfy it → P holds for ALL depths → PROOF.
    //          - SAT  → spurious counterexample at this depth; try deeper k.
    //
    // A property that is not k-inductive often becomes (k+1)-inductive —
    // hence the iterative deepening instead of a fixed k=1 step.

    /// Try to prove the property holds for ALL depths using k-induction,
    /// iterating k = 1..max_k until a proof is found or depths exhausted.
    fn check_inductive(
        &mut self,
        bound: u64,
        n_signals: usize,
        signal_widths: &[u32],
        _init_vals: &[Option<u64>],
        assignments: &[Vec<(usize, Box<IrExpr>)>],
        cond: &IrExpr,
    ) -> FormalResult {
        let max_k = self.config.max_k.min(bound.max(1));
        for k in 1..=max_k {
            if bound < k {
                break;
            }
            // Reset solver per iterasi — constraint frame k sebelumnya
            // tidak boleh bocor ke iterasi berikutnya.
            self.reset();
            match self.induction_step(k, n_signals, signal_widths, assignments, cond) {
                InductionOutcome::Proved => return FormalResult::InductiveProof(k),
                InductionOutcome::Spurious => continue, // coba k lebih dalam
                InductionOutcome::Inconclusive => return FormalResult::Unknown,
                InductionOutcome::Error(e) => return FormalResult::Error(e),
            }
        }
        // Tidak terbukti sampai max_k — BMC tetap valid sampai bound.
        FormalResult::Pass
    }

    /// Satu langkah k-induction pada kedalaman k. Mengembalikan outcome:
    /// - Proved      : asumsi P(0..k-1) ∧ ¬P(k) UNSAT → invariant.
    /// - Spurious    : SAT (counterexample palsu di kedalaman ini) — butuh k
    ///                 lebih dalam.
    /// - Inconclusive: Z3 unknown / kondisi tidak bisa diterjemahkan.
    fn induction_step(
        &mut self,
        k: u64,
        n_signals: usize,
        signal_widths: &[u32],
        assignments: &[Vec<(usize, Box<IrExpr>)>],
        cond: &IrExpr,
    ) -> InductionOutcome {
        let solver = match self.solver.as_ref() {
            Some(s) => s,
            None => {
                return InductionOutcome::Error("Z3 solver not initialized".into());
            }
        };

        let n_frames = (k + 1) as usize;

        // Frames F0..Fk — TANPA initial-state constraints (inti induction:
        // membuktikan atas SEMUA state yang memenuhi asumsi, bukan hanya
        // yang reachable dari init).
        let mut sig_vars: Vec<Vec<z3::ast::BV>> = Vec::with_capacity(n_frames);
        for d in 0..n_frames {
            let mut depth_vars = Vec::with_capacity(n_signals);
            for i in 0..n_signals {
                let width = if i < signal_widths.len() {
                    signal_widths[i]
                } else {
                    64
                };
                let var = z3::ast::BV::new_const(format!("induct_sig_{}_{}", i, d), width);
                depth_vars.push(var);
            }
            sig_vars.push(depth_vars);
        }

        // Transisi antar frame berurutan Fi → Fi+1 (sama dengan BMC STEP 3).
        let all_assigned: HashSet<usize> = if !assignments.is_empty() {
            assignments[0].iter().map(|(id, _)| *id).collect()
        } else {
            HashSet::new()
        };

        for d in 1..n_frames {
            for assign_group in assignments.iter() {
                for (sig_id, rhs) in assign_group.iter() {
                    if *sig_id >= n_signals {
                        continue;
                    }
                    let rhs_z3 = self.expr_to_z3_int_at(rhs, d as isize - 1, &sig_vars);
                    if let Some(rhs_val) = rhs_z3 {
                        let lhs_var = &sig_vars[d][*sig_id];
                        let (lhs_mw, rhs_mw) = self.zero_extend_match(lhs_var, &rhs_val);
                        solver.assert(lhs_mw.eq(&rhs_mw));
                    }
                }
            }
            // Frame constraints — sinyal tanpa driver mempertahankan nilai.
            for i in 0..n_signals {
                if !all_assigned.contains(&i) {
                    let prev = &sig_vars[d - 1][i];
                    let curr = &sig_vars[d][i];
                    solver.assert(prev.eq(curr));
                }
            }
        }

        // CATATAN soundness: logika kombinational TIDAK di-constrain di F0
        // induction — F0 mewakili state arbitrer termasuk state transien
        // awal sebelum always_comb eksekusi; meng-constrain-nya bisa
        // mengeksklusi state reachable → bukti palsu.

        // ASUMSI: P(Fi) untuk i = 0..k-1.
        for d in 0..n_frames.saturating_sub(1) {
            match self.expr_to_z3_bool_at(cond, d as isize, &sig_vars) {
                Some(p) => solver.assert(&p),
                None => return InductionOutcome::Inconclusive,
            }
        }

        // CHECK: ¬P(Fk).
        let last = n_frames - 1;
        let neg_p = match self.expr_to_z3_bool_at(cond, last as isize, &sig_vars) {
            Some(p) => p.not(),
            None => return InductionOutcome::Inconclusive,
        };
        solver.assert(&neg_p);

        match solver.check() {
            z3::SatResult::Unsat => InductionOutcome::Proved,
            z3::SatResult::Sat => InductionOutcome::Spurious,
            z3::SatResult::Unknown => InductionOutcome::Inconclusive,
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
            // FIX BUG (ROUND 91): literal desimal unsized dibungkus Signed
            // sejak fix signedness (ROUND 36) — tanpa arm ini semua ekspresi
            // ber-Signed diterjemahkan None → constraint/assertion di-skip
            // diam-diam. Semantik BV: pola bit sama.
            IrExpr::Signed(inner) => self.expr_to_z3_int_at(inner, depth, sig_vars),
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
            IrExpr::BinaryOp(op, lhs, rhs) => match op {
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
            },
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

/// Collect assumptions (`assume` statements) — FORMAL-10 tahap 1.
/// Struktur sama dengan collect_assertions; dikonstrain di semua depth
/// sehingga counterexample hanya dilaporkan bila reachable di bawah asumsi.
pub(crate) fn collect_assumptions(processes: &[Process]) -> Vec<IrExpr> {
    let mut result = Vec::new();
    for process in processes {
        let body = match process {
            Process::Combinational { body, .. }
            | Process::Sequential { body, .. }
            | Process::Initial { name: _, body }
            | Process::CombReactive { body, .. } => body,
            _ => continue,
        };
        walk_assumptions(body, &mut result);
    }
    result
}

fn walk_assumptions(stmts: &[IrStmt], result: &mut Vec<IrExpr>) {
    for stmt in stmts {
        match stmt {
            IrStmt::Assume { cond, .. } => {
                result.push(cond.clone());
            }
            IrStmt::Block { stmts: inner } => walk_assumptions(inner, result),
            IrStmt::NamedBlock { stmts: inner, .. } => walk_assumptions(inner, result),
            IrStmt::If {
                true_branch,
                false_branch,
                ..
            } => {
                walk_assumptions(true_branch, result);
                walk_assumptions(false_branch, result);
            }
            IrStmt::Case { items, default, .. } => {
                for item in items {
                    walk_assumptions(item.body.as_slice(), result);
                }
                walk_assumptions(default, result);
            }
            _ => {}
        }
    }
}

/// Collect initial-state assignments dari proses Initial (FIX ROUND 91):
/// deklarasi initializer (`reg x = 1;`) di-elaborasi menjadi proses
/// Initial `decl_init_*` berisi BlockingAssign — dipakai sebagai constraint
/// state awal BMC. Walk berhenti di statement pertama yang bukan assignment
/// / block (setelah delay/event control, nilai tidak lagi "initial").
pub(crate) fn collect_initial_state_assignments(
    processes: &[Process],
) -> Vec<(usize, IrExpr)> {
    let mut out: Vec<(usize, IrExpr)> = Vec::new();
    for process in processes {
        if let Process::Initial { body, .. } = process {
            walk_initial_state(body, &mut out);
        }
    }
    out
}

fn walk_initial_state(stmts: &[IrStmt], out: &mut Vec<(usize, IrExpr)>) {
    for stmt in stmts {
        match stmt {
            IrStmt::BlockingAssign {
                lhs: IrLValue::Signal(id, _),
                rhs,
                ..
            } => out.push((*id, rhs.clone())),
            IrStmt::Block { stmts: inner } => walk_initial_state(inner, out),
            // Delay/EventControl/dll — berhenti; statement setelahnya sudah
            // melewati kemajuan waktu, bukan lagi initial state.
            _ => return,
        }
    }
}

/// Collect assertions from process bodies.
/// Returns Vec<(module_name/process_name, condition)>.
pub(crate) fn collect_assertions(processes: &[Process]) -> Vec<(String, IrExpr)> {    let mut result = Vec::new();
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
            IrStmt::If {
                true_branch,
                false_branch,
                ..
            } => {
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
) -> Vec<Vec<(usize, Box<IrExpr>)>> {
    let mut all_assignments: Vec<(usize, Box<IrExpr>)> = Vec::new();
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
            IrStmt::If {
                true_branch,
                false_branch,
                ..
            } => {
                walk_assignments(true_branch, result, seen);
                walk_assignments(false_branch, result, seen);
            }
            IrStmt::Case { items, default, .. } => {
                for item in items {                walk_assignments(&item.body, result, seen);
            }
            walk_assignments(default, result, seen);
            }
            _ => {}
        }
    }
}

// ─── FORMAL-16: Cover Property Formal Analysis ───

/// Collect cover properties from all processes.
/// Cover properties check reachability: is there a state where the
/// property holds? (SAT check, not UNSAT like assertions.)
pub(crate) fn collect_covers(processes: &[Process]) -> Vec<(String, IrExpr)> {
    let mut result = Vec::new();
    for process in processes {
        let (name, body) = match process {
            Process::Combinational { name, body, .. } => (name, body),
            Process::Sequential { name, body, .. } => (name, body),
            Process::Initial { name, body } => (name, body),
            Process::CombReactive { name, body, .. } => (name, body),
            _ => continue,
        };
        walk_covers(&name.to_string(), body, &mut result);
    }
    result
}

fn walk_covers(prefix: &str, stmts: &[IrStmt], result: &mut Vec<(String, IrExpr)>) {
    for stmt in stmts {
        match stmt {
            IrStmt::Cover { cond, .. } => {
                result.push((prefix.to_string(), cond.clone()));
            }
            IrStmt::Block { stmts: inner } => {
                walk_covers(prefix, inner, result);
            }
            IrStmt::NamedBlock { stmts: inner, .. } => {
                walk_covers(prefix, inner, result);
            }
            IrStmt::If {
                true_branch,
                false_branch,
                ..
            } => {
                walk_covers(prefix, true_branch, result);
                walk_covers(prefix, false_branch, result);
            }
            IrStmt::Case { items, default, .. } => {
                for item in items {
                    walk_covers(prefix, &item.body, result);
                }
                walk_covers(prefix, default, result);
            }
            _ => {}
        }
    }
}

impl FormalEngine {
    /// FORMAL-16: Run BMC on all cover properties in the design.
    pub fn check_covers_bmc(&mut self, design: &IrDesign) -> Vec<(String, FormalResult)> {
        if !self.is_available() {
            self.init();
        }
        let mut results = Vec::new();
        let bound = self.config.bound;
        let n_signals = design.top.signals.len();

        let all_covers = collect_covers(&design.top.processes);
        if all_covers.is_empty() {
            return Vec::new();
        }

        let assignments = collect_combinational_assignments(&design.top.processes);
        let init_assigns = collect_initial_state_assignments(&design.top.processes);

        let signal_widths: Vec<u32> = design
            .top
            .signals
            .iter()
            .map(|s| s.width.clamp(1, 64) as u32)
            .collect();

        let init_vals: Vec<Option<u64>> = design
            .top
            .signals
            .iter()
            .map(|s| {
                let has_unknown = s.init_val.bits.iter().any(|b| matches!(b, LogicVal::X | LogicVal::Z));
                if has_unknown { None } else { Some(s.init_val.to_u64()) }
            })
            .collect();

        for (cidx, (cover_name, cond)) in all_covers.iter().enumerate() {
            let result = self.cover_check_single(
                bound, n_signals, &signal_widths, &init_vals,
                &assignments, &init_assigns, cond,
            );
            results.push((format!("{}.cover_{}", cover_name, cidx), result));
        }
        results
    }

    /// FORMAL-16: Check a single cover property with BMC.
    /// Cover BMC checks if P is SAT at any depth (reachability).
    fn cover_check_single(
        &mut self, bound: u64, n_signals: usize, signal_widths: &[u32],
        init_vals: &[Option<u64>], assignments: &[Vec<(usize, Box<IrExpr>)>],
        init_assigns: &[(usize, IrExpr)], cond: &IrExpr,
    ) -> FormalResult {
        self.reset();
        let solver = match self.solver.as_ref() {
            Some(s) => s,
            None => return FormalResult::Error("Z3 solver not initialized".into()),
        };

        let n_vars = (bound + 1) as usize;
        let mut sig_vars: Vec<Vec<z3::ast::BV>> = Vec::with_capacity(n_vars);
        for d in 0..=bound {
            let mut depth_vars = Vec::with_capacity(n_signals);
            for i in 0..n_signals {
                let width = signal_widths.get(i).copied().unwrap_or(64);
                depth_vars.push(z3::ast::BV::new_const(format!("sig_{}_{}", i, d), width));
            }
            sig_vars.push(depth_vars);
        }

        for i in 0..n_signals {
            if let Some(Some(init)) = init_vals.get(i) {
                let width = *signal_widths.get(i).unwrap_or(&64);
                solver.assert(sig_vars[0][i].eq(&z3::ast::BV::from_u64(*init, width)));
            }
        }
        for (sig_id, rhs) in init_assigns {
            if *sig_id < n_signals {
                if let Some(rhs_val) = self.expr_to_z3_int_at(rhs, 0, &sig_vars) {
                    let (a, b) = self.zero_extend_match(&sig_vars[0][*sig_id], &rhs_val);
                    solver.assert(a.eq(&b));
                }
            }
        }

        let all_assigned: HashSet<usize> = assignments.first().map(|g| g.iter().map(|(id,_)| *id).collect()).unwrap_or_default();
        for d in 1..=bound {
            for assign_group in assignments.iter() {
                for (sig_id, rhs) in assign_group.iter() {
                    if *sig_id < n_signals {
                        if let Some(rhs_val) = self.expr_to_z3_int_at(rhs, d as isize - 1, &sig_vars) {
                            let (a, b) = self.zero_extend_match(&sig_vars[d as usize][*sig_id], &rhs_val);
                            solver.assert(a.eq(&b));
                        }
                    }
                }
            }
            for i in 0..n_signals {
                if !all_assigned.contains(&i) {
                    solver.assert(sig_vars[(d-1) as usize][i].eq(&sig_vars[d as usize][i]));
                }
            }
        }

        for d in 0..=bound {
            solver.push();
            if let Some(cond_val) = self.expr_to_z3_bool_at(cond, d as isize, &sig_vars) {
                solver.assert(&cond_val);
                match solver.check() {
                    z3::SatResult::Sat => { solver.pop(1); return FormalResult::Pass; }
                    z3::SatResult::Unsat => { solver.pop(1); continue; }
                    z3::SatResult::Unknown => { solver.pop(1); return FormalResult::Unknown; }
                }
            } else {
                solver.pop(1);
            }
        }
        FormalResult::Counterexample(bound)
    }
}

// ─── FORMAL-15: Unreachable Assertion Detection ───

/// Result of unreachable assertion analysis.
#[derive(Debug, Clone)]
pub struct UnreachableAssertInfo {
    /// Name of the assertion (process + index)
    pub name: String,
    /// Whether the assertion condition is always true (trivially satisfied)
    pub always_true: bool,
    /// Whether the assertion condition is satisfiable at all (can ever be false)
    pub can_violate: bool,
    /// Description of the analysis result
    pub description: String,
}

impl FormalEngine {
    /// FORMAL-15: Detect unreachable assertions.
    ///
    /// For each assertion, checks:
    /// 1. Is the condition always true (unsatisfiable negation)?
    /// 2. Can the condition ever be false in the transition system?
    ///
    /// An assertion is "unreachable" if its negation is unsatisfiable
    /// (meaning the assertion condition always holds — it can never be violated).
    pub fn detect_unreachable_assertions(
        &mut self,
        design: &IrDesign,
    ) -> Vec<UnreachableAssertInfo> {
        if !self.is_available() {
            self.init();
        }
        let mut results = Vec::new();
        let bound = self.config.bound;
        let n_signals = design.top.signals.len();

        let all_assertions = collect_assertions(&design.top.processes);
        if all_assertions.is_empty() {
            return results;
        }

        let assignments = collect_combinational_assignments(&design.top.processes);
        let init_assigns = collect_initial_state_assignments(&design.top.processes);
        let assumptions = collect_assumptions(&design.top.processes);

        let signal_widths: Vec<u32> = design
            .top.signals
            .iter()
            .map(|s| s.width.clamp(1, 64) as u32)
            .collect();

        let init_vals: Vec<Option<u64>> = design
            .top.signals
            .iter()
            .map(|s| {
                let has_unknown = s.init_val.bits.iter().any(|b| matches!(b, LogicVal::X | LogicVal::Z));
                if has_unknown { None } else { Some(s.init_val.to_u64()) }
            })
            .collect();

        for (aidx, (assert_name, cond)) in all_assertions.iter().enumerate() {
            let name = format!("{}.assert_{}", assert_name, aidx);

            // Check 1: Is ¬P always UNSAT? (P is always true)
            self.reset();
            let solver = match self.solver.as_ref() {
                Some(s) => s,
                None => {
                    results.push(UnreachableAssertInfo {
                        name,
                        always_true: false,
                        can_violate: false,
                        description: "Z3 solver not initialized".into(),
                    });
                    continue;
                }
            };

            // Build minimal transition system (1 step)
            let check_bound = bound.min(4); // quick check at low bound
            let n_vars = (check_bound + 1) as usize;
            let mut sig_vars: Vec<Vec<z3::ast::BV>> = Vec::with_capacity(n_vars);
            for d in 0..=check_bound {
                let mut depth_vars = Vec::with_capacity(n_signals);
                for i in 0..n_signals {
                    let width = signal_widths.get(i).copied().unwrap_or(64);
                    depth_vars.push(z3::ast::BV::new_const(
                        format!("reach_sig_{}_{}", i, d), width,
                    ));
                }
                sig_vars.push(depth_vars);
            }

            // Initial state constraints
            for i in 0..n_signals {
                if let Some(Some(init)) = init_vals.get(i) {
                    let width = *signal_widths.get(i).unwrap_or(&64);
                    solver.assert(sig_vars[0][i].eq(&z3::ast::BV::from_u64(*init, width)));
                }
            }
            for (sig_id, rhs) in &init_assigns {
                if *sig_id < n_signals {
                    if let Some(rhs_val) = self.expr_to_z3_int_at(rhs, 0, &sig_vars) {
                        let (a, b) = self.zero_extend_match(&sig_vars[0][*sig_id], &rhs_val);
                        solver.assert(a.eq(&b));
                    }
                }
            }

            // Transition constraints
            let all_assigned: HashSet<usize> = assignments.first()
                .map(|g| g.iter().map(|(id, _)| *id).collect())
                .unwrap_or_default();
            for d in 1..=check_bound {
                for assign_group in &assignments {
                    for (sig_id, rhs) in assign_group.iter() {
                        if *sig_id < n_signals {
                            if let Some(rhs_val) = self.expr_to_z3_int_at(rhs, d as isize - 1, &sig_vars) {
                                let (a, b) = self.zero_extend_match(&sig_vars[d as usize][*sig_id], &rhs_val);
                                solver.assert(a.eq(&b));
                            }
                        }
                    }
                }
                for i in 0..n_signals {
                    if !all_assigned.contains(&i) {
                        solver.assert(sig_vars[(d-1) as usize][i].eq(&sig_vars[d as usize][i]));
                    }
                }
            }

            // Assumptions
            for a in &assumptions {
                for d in 0..=check_bound {
                    if let Some(a_bool) = self.expr_to_z3_bool_at(a, d as isize, &sig_vars) {
                        solver.assert(&a_bool);
                    }
                }
            }

            // Check 1: ¬P at depth 0 — if UNSAT, P is always true
            solver.push();
            let always_true = if let Some(cond_bool) = self.expr_to_z3_bool_at(cond, 0, &sig_vars) {
                solver.assert(&cond_bool.not());
                match solver.check() {
                    z3::SatResult::Unsat => true,
                    _ => false,
                }
            } else {
                false
            };
            solver.pop(1);

            // Check 2: ¬P at ANY depth 0..bound — if UNSAT, never violates
            let mut can_violate = false;
            for d in 0..=check_bound {
                solver.push();
                if let Some(cond_bool) = self.expr_to_z3_bool_at(cond, d as isize, &sig_vars) {
                    solver.assert(&cond_bool.not());
                    if let z3::SatResult::Sat = solver.check() {
                        can_violate = true;
                        solver.pop(1);
                        break;
                    }
                }
                solver.pop(1);
            }

            let description = if always_true {
                "assertion condition is always true (trivially satisfied — never violated)".to_string()
            } else if !can_violate {
                "assertion condition is unreachable in the transition system (never false at any depth)".to_string()
            } else {
                "assertion is reachable (can be violated)".to_string()
            };

            results.push(UnreachableAssertInfo {
                name,
                always_true,
                can_violate,
                description,
            });
        }
        results
    }
}
