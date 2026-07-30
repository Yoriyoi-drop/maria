use super::super::SimulationEngine;
use crate::error::SimError;
use crate::diagnostics::DiagCode;
use crate::ir::*;
use crate::ast::*;
use crate::Symbol;
use crate::simulator::engine::uvm::constraint_solver::SolveResult;
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
            .map(|o| o.class_name.clone())
            .unwrap_or_default();
        if class_name.is_empty() {
            return Err(SimError::with_diag(
                DiagCode::DpiError,
                format!("cannot call method '{}' on object with unknown class", method),
            ));
        }
        if class_name == "__mailbox" {
            return self.execute_mailbox_method(obj_id, method, args);
        }
        if class_name == "__semaphore" {
            return self.execute_semaphore_method(obj_id, method, args);
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
        // Check uvm_callbacks hierarchy (must be before general object dispatch)
        if self.is_uvm_callbacks_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_callbacks_add(obj_id, method, args);
            }
        }
        // Check uvm_callback hierarchy
        if self.is_uvm_callback_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_callback_method(obj_id, method, args);
            }
        }
        // Check uvm_driver hierarchy (most specific first)
        if self.is_uvm_driver_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_driver_method(obj_id, method, args);
            }
        }
        // Check uvm_sequencer hierarchy
        if self.is_uvm_sequencer_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_sequencer_method(obj_id, method, args);
            }
        }
        // Check uvm_sequence hierarchy
        if self.is_uvm_sequence_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_sequence_method(obj_id, method, args);
            }
        }
        // Check uvm_monitor hierarchy
        if self.is_uvm_monitor_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_monitor_method(obj_id, method, args);
            }
        }
        // Check uvm_analysis_port hierarchy
        if self.is_uvm_analysis_port_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_analysis_port_method(obj_id, method, args);
            }
        }
        // Check uvm_analysis_imp hierarchy
        if self.is_uvm_analysis_imp_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_analysis_imp_method(obj_id, method, args);
            }
        }
        // Check uvm_reg_block hierarchy (most specific reg layer first)
        if self.is_uvm_reg_block_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_reg_block_method(obj_id, method, args);
            }
        }
        // Check uvm_reg_map hierarchy
        if self.is_uvm_reg_map_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_reg_map_method(obj_id, method, args);
            }
        }
        // Check uvm_reg hierarchy
        if self.is_uvm_reg_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_reg_method(obj_id, method, args);
            }
        }
        // Check uvm_reg_field hierarchy
        if self.is_uvm_reg_field_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_reg_field_method(obj_id, method, args);
            }
        }
        // Check uvm_sequence_item hierarchy
        if self.is_uvm_sequence_item_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_sequence_item_method(obj_id, method, args);
            }
        }
        // Check for uvm_component hierarchy methods — only intercept if class doesn't override
        if self.is_uvm_component_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_component_method(obj_id, method, args);
            }
        }
        // Check for uvm_report_object hierarchy methods — only intercept if class doesn't override
        if self.is_uvm_report_object_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_report_object_method(obj_id, method, args);
            }
        }
        // Check for uvm_object hierarchy methods — only intercept if class doesn't override
        if self.is_uvm_object_hierarchy(&class_name.as_str()) {
            let has_override = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_override {
                return self.execute_uvm_object_method(obj_id, method, args);
            }
        }

        // ─── Invoke pre-callbacks (uvm_callbacks::pre_*) ───
        let pre_cb_method = format!("pre_{}", method);
        self.invoke_callbacks(class_name.as_str(), &pre_cb_method, args)?;

        // Check for built-in randomize() — only if no user-defined override exists
        if method == "randomize" {
            let has_user_method = self.find_method_in_hierarchy(class_name.as_str(), method).is_ok();
            if !has_user_method {
                return self.execute_randomize(obj_id, class_name.as_str());
            }
        }
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
        match self.solve_constraints(obj_id, class_name, with_clause)? {
            SolveResult::Satisfied => Ok(LogicVec::from_u64(1, 1)),
            SolveResult::Unsatisfiable => {
                // Fall back to rejection sampling with inline constraint
                self.randomize_rejection_fallback(obj_id, class_name, with_clause)
            }
        }
    }

    /// Fallback: pure rejection sampling with up to 10K attempts.
    /// Used when the domain-guided solver can't satisfy constraints.
    fn randomize_rejection_fallback(
        &mut self,
        obj_id: ObjId,
        class_name: &str,
        with_clause: Option<&IrExpr>,
    ) -> Result<LogicVec, SimError> {
        let class_def = self
            .design
            .classes
            .get(class_name)
            .ok_or_else(|| SimError::with_diag(DiagCode::NullHandle, format!("class '{}' not found", class_name)))?
            .clone();
        if class_def.rand_fields.is_empty() {
            return Ok(LogicVec::from_u64(1, 1));
        }
        let old_this = self.current_this;
        self.current_this = Some(obj_id);

        // Extract solve...before ordering
        let mut before_map: HashMap<Symbol, HashSet<Symbol>> = HashMap::new();
        for (_, body) in &class_def.constraints {
            for item in body {
                if let ConstraintItem::SolveBefore { vars } = item {
                    if vars.len() >= 2 {
                        let first = &vars[0];
                        for later in &vars[1..] {
                            before_map
                                .entry(*first)
                                .or_insert_with(HashSet::new)
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
        let mut seed = self.current_time as u64;
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

            // Evaluate all class constraints
            let mut all_satisfied = true;
            for (_, body) in &class_def.constraints {
                for item in body {
                    match item {
                        ConstraintItem::Expr(expr) => {
                            let result = self.evaluate_ast_expr(expr)?;
                            if !result.to_bool().unwrap_or(false) {
                                all_satisfied = false;
                                break;
                            }
                        }
                        ConstraintItem::SolveBefore { .. } => {}
                    }
                }
                if !all_satisfied {
                    break;
                }
            }

            // Evaluate inline constraint
            if all_satisfied {
                if let Some(wc) = with_clause {
                    let wc_result = self.evaluate_expr(wc)?;
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
        Err(SimError::with_diag(
            DiagCode::InternalError,
            format!("randomize failed: could not satisfy all constraints after {} attempts", max_attempts),
        ))
    }

}
