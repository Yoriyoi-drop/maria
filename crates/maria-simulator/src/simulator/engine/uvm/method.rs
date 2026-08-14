use super::super::SimulationEngine;
use maria_core::error::SimError;
use maria_core::diagnostics::DiagCode;
use maria_ir::*;
use maria_ast::*;
use maria_core::Symbol;
use crate::simulator::engine::uvm::constraint_solver::{InlineConstraint, SolveResult};
use std::collections::{HashMap, HashSet};

impl SimulationEngine {
    pub(crate) fn execute_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        let class_name = self
            .state
            .get_object(obj_id)
            .map(|o| o.class_name)
            .unwrap_or_default();
        if std::env::var("DBG_UVM").is_ok() && matches!(method, "start_item" | "finish_item" | "get_next_item" | "item_done") {
            eprintln!("[DBG-UVM] execute_method {} class={} is_seq={}", method, class_name, self.is_uvm_sequence_hierarchy(class_name.as_str()));
        }
        if class_name.is_empty() {
            // Method call pada null handle / class tak dikenal: warning + default
            // agar simulasi tetap berjalan (null-handle chain pada kode UVM).
            self.emit_warning(
                DiagCode::NullHandle,
                format!("cannot call method '{}' on object with unknown class (obj_id={}); using null default", method, obj_id),
            );
            return Ok(LogicVec::from_u64(0, 64));
        }
        if class_name == "__mailbox" {
            return self.execute_mailbox_method(obj_id, method, args);
        }
        if class_name == "__semaphore" {
            return self.execute_semaphore_method(obj_id, method, args);
        }
        // F21: uvm_event / uvm_barrier — sinkronisasi antar komponen.
        // is_uvm_*_hierarchy menelusuri extends chain: objek dibuat dengan
        // class_name asli (`uvm_event` dari tipe field), bukan `__uvm_event`.
        // HANYA intercept bila method TIDAK dioverride user (pola randomize:
        // "only intercept if class doesn't override") — class `my_event
        // extends uvm_event` dengan `new` override / method custom (mis.
        // notify_extra) harus jalan normal, bukan "unknown uvm_event method".
        let user_override = self
            .find_method_in_hierarchy(class_name.as_str(), method)
            .is_ok();
        if !user_override && self.is_uvm_event_hierarchy(class_name.as_str()) {
            return self.execute_uvm_event_method(obj_id, method, args);
        }
        if !user_override && self.is_uvm_barrier_hierarchy(class_name.as_str()) {
            return self.execute_uvm_barrier_method(obj_id, method, args);
        }
        // F22: uvm_subscriber — `new` builtin (auto-buat analysis_imp child +
        // field analysis_imp); `write`/method lain dioverride user → normal.
        if !user_override && self.is_uvm_subscriber_hierarchy(class_name.as_str()) {
            return self.execute_uvm_subscriber_method(obj_id, method, args);
        }
        // F23: uvm_tlm_fifo — `new` builtin (queue + analysis_export internal);
        // put/get/peek blocking di block.rs; query lain dioverride user → normal.
        if !user_override && self.is_uvm_tlm_fifo_hierarchy(class_name.as_str()) {
            return self.execute_uvm_tlm_fifo_method(obj_id, method, args);
        }
        if !user_override && self.is_uvm_fifo_export_hierarchy(class_name.as_str()) {
            return self.execute_uvm_fifo_export_method(obj_id, method, args);
        }
        // F24: uvm_seq_item_port — `new` builtin (data port + component);
        // get_next_item/item_done mendelegasi ke sequencer; user override normal.
        if !user_override && self.is_uvm_seq_item_port_hierarchy(class_name.as_str()) {
            return self.execute_uvm_seq_item_port_method(obj_id, method, args);
        }
        if class_name == "__process" {
            return self.execute_process_method(obj_id, method, args);
        }
        // Covergroup support: sample() records coverage data
        if method == "sample" && class_name.starts_with("__covergroup_") {
            let cg_name = &class_name.as_str()["__covergroup_".len()..];
            if self.design.covergroups.iter().any(|c| c.name == cg_name) {
                return self
                    .sample_covergroup(cg_name)
                    .map(|_| LogicVec::from_u64(1, 1));
            }
        }
        // F17: built-in randomize() harus dicek SEBELUM dispatch hierarki UVM.
        // Sebelumnya check ini berada setelah semua `is_uvm_*_hierarchy` —
        // untuk class UVM (uvm_sequence_item, uvm_object, dkk) randomize()
        // dicegat lebih dulu dan jatuh ke "randomize not implemented". Hanya
        // dipakai bila tidak ada override user (pola "only intercept if class
        // doesn't override").
        if method == "randomize" {
            let has_user_method = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_user_method {
                return self.execute_randomize(obj_id, class_name.as_str());
            }
        }
        // Check uvm_callbacks hierarchy (must be before general object dispatch)
        if self.is_uvm_callbacks_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_callbacks_add(obj_id, method, args);
            }
        }
        // Check uvm_callback hierarchy
        if self.is_uvm_callback_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_callback_method(obj_id, method, args);
            }
        }
        // Check uvm_driver hierarchy (most specific first)
        if self.is_uvm_driver_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_driver_method(obj_id, method, args);
            }
        }
        // Check uvm_sequencer hierarchy
        if self.is_uvm_sequencer_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_sequencer_method(obj_id, method, args);
            }
        }
        // Check uvm_sequence hierarchy
        if self.is_uvm_sequence_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_sequence_method(obj_id, method, args);
            }
        }
        // Check uvm_monitor hierarchy
        if self.is_uvm_monitor_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_monitor_method(obj_id, method, args);
            }
        }
        // Check uvm_analysis_port hierarchy
        if self.is_uvm_analysis_port_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_analysis_port_method(obj_id, method, args);
            }
        }
        // Check uvm_analysis_imp hierarchy
        if self.is_uvm_analysis_imp_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_analysis_imp_method(obj_id, method, args);
            }
        }
        // Check uvm_reg_block hierarchy (most specific reg layer first)
        if self.is_uvm_reg_block_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_reg_block_method(obj_id, method, args);
            }
        }
        // Check uvm_reg_map hierarchy
        if self.is_uvm_reg_map_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_reg_map_method(obj_id, method, args);
            }
        }
        // Check uvm_reg hierarchy
        if self.is_uvm_reg_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_reg_method(obj_id, method, args);
            }
        }
        // Check uvm_reg_field hierarchy
        if self.is_uvm_reg_field_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_reg_field_method(obj_id, method, args);
            }
        }
        // Check uvm_sequence_item hierarchy
        if self.is_uvm_sequence_item_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_sequence_item_method(obj_id, method, args);
            }
        }
        // Check for uvm_component hierarchy methods — only intercept if class doesn't override
        if self.is_uvm_component_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_component_method(obj_id, method, args);
            }
        }
        // Check for uvm_report_object hierarchy methods — only intercept if class doesn't override
        if self.is_uvm_report_object_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_report_object_method(obj_id, method, args);
            }
        }
        // Check for uvm_object hierarchy methods — only intercept if class doesn't override
        if self.is_uvm_object_hierarchy(class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_object_method(obj_id, method, args);
            }
        }

        // ─── Invoke pre-callbacks (uvm_callbacks::pre_*) ───
        let pre_cb_method = format!("pre_{}", method);
        self.invoke_callbacks(class_name.as_str(), &pre_cb_method, args)?;

        // Normal dispatch: find method in the full class hierarchy (virtual dispatch)
        let method_def = self.find_method_in_hierarchy(class_name.as_str(), method)?.clone();
        // Static methods don't receive `this`
        let this_opt = if method_def.is_static {
            None
        } else {
            Some(obj_id)
        };
        let result = self.execute_method_body(this_opt, &method_def, args, method);

        // ─── Invoke post-callbacks (uvm_callbacks::post_*) ───
        let post_cb_method = format!("post_{}", method);
        self.invoke_callbacks(class_name.as_str(), &post_cb_method, args)?;

        result
    }

    pub(crate) fn execute_randomize(&mut self, obj_id: ObjId, class_name: &str) -> Result<LogicVec, SimError> {
        // Try the smart constraint solver first (domain analysis + guided generation)
        match self.solve_constraints(obj_id, class_name, None)? {
            SolveResult::Satisfied => Ok(LogicVec::from_u64(1, 1)),
            SolveResult::Unsatisfiable => {
                // Fall back to rejection sampling with more attempts
                self.randomize_rejection_fallback(obj_id, class_name, None)
            }
        }
    }

    pub(crate) fn execute_randomize_with(
        &mut self,
        obj_id: ObjId,
        class_name: &str,
        with_clause: Option<&IrExpr>,
    ) -> Result<LogicVec, SimError> {
        if with_clause.is_none() {
            return self.execute_randomize(obj_id, class_name);
        }
        // Try the smart constraint solver first
        match self.solve_constraints(obj_id, class_name, with_clause.map(InlineConstraint::Ir))? {
            SolveResult::Satisfied => Ok(LogicVec::from_u64(1, 1)),
            SolveResult::Unsatisfiable => {
                // Fall back to rejection sampling with inline constraint
                self.randomize_rejection_fallback(
                    obj_id,
                    class_name,
                    with_clause.map(InlineConstraint::Ir),
                )
            }
        }
    }

    /// F17: randomize() with {...} di jalur AST (body method class) — inline
    /// constraint AST dievaluasi via evaluate_ast_expr dengan current_this,
    /// sehingga field class (`addr`) bisa diakses langsung tanpa elaborasi IR.
    pub(crate) fn execute_randomize_ast_with(
        &mut self,
        obj_id: ObjId,
        class_name: &str,
        with_clause: Option<&Expr>,
    ) -> Result<LogicVec, SimError> {
        if with_clause.is_none() {
            return self.execute_randomize(obj_id, class_name);
        }
        match self.solve_constraints(obj_id, class_name, with_clause.map(InlineConstraint::Ast))? {
            SolveResult::Satisfied => Ok(LogicVec::from_u64(1, 1)),
            SolveResult::Unsatisfiable => self.randomize_rejection_fallback(
                obj_id,
                class_name,
                with_clause.map(InlineConstraint::Ast),
            ),
        }
    }

    /// Fallback: pure rejection sampling with up to 10K attempts.
    /// Used when the domain-guided solver can't satisfy constraints.
    fn randomize_rejection_fallback(
        &mut self,
        obj_id: ObjId,
        class_name: &str,
        with_clause: Option<InlineConstraint<'_>>,
    ) -> Result<LogicVec, SimError> {
        let class_def = self
            .design
            .classes
            .get(&Symbol::intern(class_name))
            .ok_or_else(|| SimError::with_diag(DiagCode::NullHandle, format!("class '{}' not found", class_name)))?
            .clone();
        if class_def.rand_fields.is_empty() {
            return Ok(LogicVec::from_u64(1, 1));
        }
        let old_this = self.current_this;
        self.current_this = Some(obj_id);

        // Extract solve...before ordering
        let mut before_map: HashMap<Symbol, HashSet<Symbol>> = HashMap::new();
        for (_, _, body) in &class_def.constraints {
            for item in body {
                if let ConstraintItem::SolveBefore { vars } = item {
                    if vars.len() >= 2 {
                        let first = &vars[0];
                        for later in &vars[1..] {
                            before_map
                                .entry(*first)
                                .or_default()
                                .insert(*later);
                        }
                    }
                }
            }
        }

        // Order rand_fields: solve-before first
        let mut ordered_fields: Vec<Symbol> = Vec::new();
        let mut remaining: HashSet<Symbol> = class_def.rand_fields.iter().cloned().collect();
        for fname in &class_def.rand_fields {
            if before_map.contains_key(fname) && remaining.contains(fname) {
                ordered_fields.push(*fname);
                remaining.remove(fname);
            }
        }
        for fname in &class_def.rand_fields {
            if remaining.contains(fname) {
                ordered_fields.push(*fname);
            }
        }

        let max_attempts = 10_000;
        let mut seed = self.current_time;
        for _ in 0..max_attempts {
            // Generate random values for each rand field
            for fname in &ordered_fields {
                let field_info = class_def.fields.iter().find(|f| &f.name == fname);
                let width = field_info.map(|f| f.width).unwrap_or(1);
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let rv = LogicVec::from_u64(seed, width);
                if let Some(obj) = self.state.objects.get_mut(obj_id) {
                    obj.fields.insert(*fname, rv);
                }
            }

            // Evaluate all class constraints (rekursif — termasuk if/else F12)
            // LANG-33: block nonaktif (constraint_mode(0)) di-skip.
            // LANG-32: block STATIC dicek global per-class (semua instance).
            let class_sym = Symbol::intern(class_name);
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

            // Evaluate inline constraint
            if all_satisfied {
                if let Some(wc) = with_clause {
                    let wc_result = match wc {
                        InlineConstraint::Ast(e) => self.evaluate_ast_expr(e)?,
                        InlineConstraint::Ir(ir) => self.evaluate_expr(ir)?,
                    };
                    if !wc_result.to_bool().unwrap_or(false) {
                        all_satisfied = false;
                    }
                }
            }

            if all_satisfied {
                self.current_this = old_this;
                return Ok(LogicVec::from_u64(1, 1));
            }
        }

        self.current_this = old_this;
        // IEEE 1800-2017 §18.6.1: bila solusi tidak ditemukan, `randomize()`
        // mengembalikan 0 (BUKAN error fatal). Constraint yang bertentangan
        // (unsatisfiable) adalah kondisi runtime yang sah — testbench
        // memeriksa return value untuk retry/fallback.
        Ok(LogicVec::from_u64(0, 1))
    }

}
