use super::super::SimulationEngine;
use crate::error::SimError;
use crate::ir::*;
use crate::ast::*;
use crate::Symbol;
use std::collections::HashSet;

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
            return Err(SimError::runtime(format!(
                "cannot call method '{}' on object with unknown class",
                method
            )));
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
        self.execute_method_body(this_opt, &method_def, args, method)
    }

    pub(crate) fn execute_randomize(&mut self, obj_id: ObjId, class_name: &str) -> Result<LogicVec, SimError> {
        // Clone all data we need to avoid borrow conflicts
        let class_def = self
            .design
            .classes
            .get(class_name)
            .ok_or_else(|| format!("class '{}' not found", class_name))?
            .clone();
        if class_def.rand_fields.is_empty() {
            return Ok(LogicVec::from_u64(1, 1));
        }
        let old_this = self.current_this;
        self.current_this = Some(obj_id);

        // Extract solve...before ordering constraints
        let mut before_map: std::collections::HashMap<Symbol, std::collections::HashSet<Symbol>> =
            std::collections::HashMap::new();
        for (_, body) in &class_def.constraints {
            for item in body {
                if let ConstraintItem::SolveBefore { vars } = item {
                    if vars.len() >= 2 {
                        let first = &vars[0];
                        for later in &vars[1..] {
                            before_map
                                .entry(first.clone())
                                .or_insert_with(std::collections::HashSet::new)
                                .insert(later.clone());
                        }
                    }
                }
            }
        }

        // Order rand_fields: fields in solve-before come first
        let mut ordered_fields: Vec<Symbol> = Vec::new();
        let mut remaining: std::collections::HashSet<Symbol> =
            class_def.rand_fields.iter().cloned().collect::<HashSet<Symbol>>();
        for fname in &class_def.rand_fields {
            if before_map.contains_key::<str>(fname.as_str()) && remaining.contains::<str>(fname.as_str()) {
                ordered_fields.push(fname.clone());
                remaining.remove::<str>(fname.as_str());
            }
        }
        for fname in &class_def.rand_fields {
            if remaining.contains::<str>(fname.as_str()) {
                ordered_fields.push(fname.clone());
            }
        }

        let max_attempts = 100;
        let mut seed = self.current_time as u64;
        for _ in 0..max_attempts {
            // Generate random values for each rand field in solve-order
            for fname in &ordered_fields {
                let field_info = class_def.fields.iter().find(|f| &f.name == fname);
                let width = field_info.map(|f| f.width).unwrap_or(1);
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let rv = LogicVec::from_u64(seed, width);
                if let Some(obj) = self.state.objects.get_mut(obj_id) {
                    obj.fields.insert(fname.clone(), rv);
                }
            }

            // Evaluate all constraints (skip SolveBefore items)
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
                        ConstraintItem::SolveBefore { .. } => {
                            // Just an ordering hint, skip during evaluation
                        }
                    }
                }
                if !all_satisfied {
                    break;
                }
            }

            if all_satisfied {
                self.current_this = old_this;
                return Ok(LogicVec::from_u64(1, 1));
            }
        }

        self.current_this = old_this;
        Err(SimError::runtime(format!(
            "randomize failed: could not satisfy all constraints after {} attempts",
            max_attempts
        )))
    }

    pub(crate) fn execute_randomize_with(
        &mut self,
        obj_id: ObjId,
        class_name: &str,
        with_clause: Option<&IrExpr>,
    ) -> Result<LogicVec, SimError> {
        let class_def = self
            .design
            .classes
            .get(class_name)
            .ok_or_else(|| format!("class '{}' not found", class_name))?
            .clone();
        if class_def.rand_fields.is_empty() {
            return Ok(LogicVec::from_u64(1, 1));
        }
        if with_clause.is_none() {
            return self.execute_randomize(obj_id, class_name);
        }
        let wc = with_clause.unwrap();
        let old_this = self.current_this;
        self.current_this = Some(obj_id);

        let mut before_map: std::collections::HashMap<Symbol, std::collections::HashSet<Symbol>> =
            std::collections::HashMap::new();
        for (_, body) in &class_def.constraints {
            for item in body {
                if let ConstraintItem::SolveBefore { vars } = item {
                    if vars.len() >= 2 {
                        let first = &vars[0];
                        for later in &vars[1..] {
                            before_map
                                .entry(first.clone())
                                .or_insert_with(std::collections::HashSet::new)
                                .insert(later.clone());
                        }
                    }
                }
            }
        }

        let mut ordered_fields: Vec<Symbol> = Vec::new();
        let mut remaining: std::collections::HashSet<Symbol> =
            class_def.rand_fields.iter().cloned().collect::<HashSet<Symbol>>();
        for fname in &class_def.rand_fields {
            if before_map.contains_key::<str>(fname.as_str()) && remaining.contains::<str>(fname.as_str()) {
                ordered_fields.push(fname.clone());
                remaining.remove::<str>(fname.as_str());
            }
        }
        for fname in &class_def.rand_fields {
            if remaining.contains::<str>(fname.as_str()) {
                ordered_fields.push(fname.clone());
            }
        }

        let max_attempts = 100;
        let mut seed = self.current_time as u64;
        for _ in 0..max_attempts {
            for fname in &ordered_fields {
                let field_info = class_def.fields.iter().find(|f| &f.name == fname);
                let width = field_info.map(|f| f.width).unwrap_or(1);
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let rv = LogicVec::from_u64(seed, width);
                if let Some(obj) = self.state.objects.get_mut(obj_id) {
                    obj.fields.insert(fname.clone(), rv);
                }
            }

            let mut all_satisfied = true;
            // Evaluate class constraints
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
            // Evaluate inline constraint (with_clause)
            if all_satisfied {
                let wc_result = self.evaluate_expr(wc)?;
                if !wc_result.to_bool().unwrap_or(false) {
                    all_satisfied = false;
                }
            }

            if all_satisfied {
                self.current_this = old_this;
                return Ok(LogicVec::from_u64(1, 1));
            }
        }

        self.current_this = old_this;
        Err(SimError::runtime(format!(
            "randomize with failed: could not satisfy constraints after {} attempts",
            max_attempts
        )))
    }

}
