use super::SimulationEngine;
use crate::simulator::util::*;
use crate::ast::*;
use crate::error::SimError;
use crate::ir::*;
use crate::Symbol;
use crate::simulator::parallel;
use crate::simulator::state::SimulationState;
use crate::simulator::types::*;
use crate::simulator::value::*;
use crate::waveform::FstWaveWriter;
use crate::waveform::VcdWriter;
use rand::Rng;
use rand::SeedableRng;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;


impl SimulationEngine {
    fn find_phase_class_name(&self) -> Option<String> {
        let phase_methods = ["build_phase", "connect_phase", "run_phase"];
        let mut best: Option<(String, usize)> = None;
        for (name, cls) in &self.design.classes {
            if !self.is_uvm_test_hierarchy(name) {
                continue;
            }
            let count = phase_methods
                .iter()
                .filter(|pm| cls.methods.iter().any(|m| &m.name == *pm))
                .count();
            if count > 0 && best.as_ref().map_or(true, |b| count > b.1) {
                best = Some((name.clone(), count));
            }
        }
        // fallback: any class with phase methods
        if best.is_none() {
            for (name, cls) in &self.design.classes {
                let count = phase_methods
                    .iter()
                    .filter(|pm| cls.methods.iter().any(|m| &m.name == *pm))
                    .count();
                if count > 0 && best.as_ref().map_or(true, |b| count > b.1) {
                    best = Some((name.clone(), count));
                }
            }
        }
        best.map(|(name, _)| name)
    }

    fn is_uvm_test_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_test" {
                return true;
            }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn execute_phases(&mut self) -> Result<(), SimError> {
        let class_name = match self.find_phase_class_name() {
            Some(c) => c,
            None => return Ok(()),
        };
        // Create root test object once, shared across all phases
        let obj_id = self.state.alloc_object(&class_name);
        self.root_test_obj_id = Some(obj_id);

        // build_phase: root then children
        if self
            .find_method_in_hierarchy(&class_name, "build_phase")
            .is_ok()
        {
            self.current_this = Some(obj_id);
            self.execute_method(obj_id, "build_phase", &[])?;
            self.current_this = None;
            self.call_phase_on_children(obj_id, "build_phase")?;
        }
        // connect_phase: root then children
        if self
            .find_method_in_hierarchy(&class_name, "connect_phase")
            .is_ok()
        {
            self.current_this = Some(obj_id);
            self.execute_method(obj_id, "connect_phase", &[])?;
            self.current_this = None;
            self.call_phase_on_children(obj_id, "connect_phase")?;
        }
        // run_phase: call root's run_phase (blocking since delays in methods are no-ops)
        if self
            .find_method_in_hierarchy(&class_name, "run_phase")
            .is_ok()
        {
            self.current_this = Some(obj_id);
            self.execute_method(obj_id, "run_phase", &[])?;
            self.current_this = None;
        }
        Ok(())
    }

    fn call_phase_on_children(&mut self, obj_id: ObjId, phase: &str) -> Result<(), SimError> {
        if let Some(cdata) = self.uvm_component_data.get(&obj_id) {
            let children = cdata.children.clone();
            for child_id in children {
                if let Some(obj) = self.state.get_object(child_id) {
                    let child_class = &obj.class_name;
                    if self.find_method_in_hierarchy(child_class, phase).is_ok() {
                        self.current_this = Some(child_id);
                        self.execute_method(child_id, phase, &[])?;
                        self.current_this = None;
                    }
                }
                self.call_phase_on_children(child_id, phase)?;
            }
        }
        Ok(())
    }

    fn is_uvm_object_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_object" {
                return true;
            }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_component_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_component" {
                return true;
            }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_report_object_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_report_object" {
                return true;
            }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_sequence_item_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_sequence_item" {
                return true;
            }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_sequence_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_sequence" {
                return true;
            }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_sequencer_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_sequencer" {
                return true;
            }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_monitor_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_monitor" {
                return true;
            }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_analysis_port_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_analysis_port" {
                return true;
            }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_analysis_imp_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_analysis_imp" {
                return true;
            }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_driver_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_driver" {
                return true;
            }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn execute_method(
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
            let cg_name = &class_name["__covergroup_".len()..];
            if self.design.covergroups.iter().any(|c| c.name == cg_name) {
                return self
                    .sample_covergroup(cg_name)
                    .map(|_| LogicVec::from_u64(1, 1));
            }
        }
        // Check uvm_driver hierarchy (most specific first)
        if self.is_uvm_driver_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_driver_method(obj_id, method, args);
            }
        }
        // Check uvm_sequencer hierarchy
        if self.is_uvm_sequencer_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_sequencer_method(obj_id, method, args);
            }
        }
        // Check uvm_sequence hierarchy
        if self.is_uvm_sequence_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_sequence_method(obj_id, method, args);
            }
        }
        // Check uvm_monitor hierarchy
        if self.is_uvm_monitor_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_monitor_method(obj_id, method, args);
            }
        }
        // Check uvm_analysis_port hierarchy
        if self.is_uvm_analysis_port_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_analysis_port_method(obj_id, method, args);
            }
        }
        // Check uvm_analysis_imp hierarchy
        if self.is_uvm_analysis_imp_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_analysis_imp_method(obj_id, method, args);
            }
        }
        // Check uvm_sequence_item hierarchy
        if self.is_uvm_sequence_item_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_sequence_item_method(obj_id, method, args);
            }
        }
        // Check for uvm_component hierarchy methods — only intercept if class doesn't override
        if self.is_uvm_component_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_component_method(obj_id, method, args);
            }
        }
        // Check for uvm_report_object hierarchy methods — only intercept if class doesn't override
        if self.is_uvm_report_object_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_report_object_method(obj_id, method, args);
            }
        }
        // Check for uvm_object hierarchy methods — only intercept if class doesn't override
        if self.is_uvm_object_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_object_method(obj_id, method, args);
            }
        }

        // Check for built-in randomize() — only if no user-defined override exists
        if method == "randomize" {
            let has_user_method = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_user_method {
                return self.execute_randomize(obj_id, &class_name);
            }
        }
        // Normal dispatch: find method in the full class hierarchy (virtual dispatch)
        let method_def = self.find_method_in_hierarchy(&class_name, method)?.clone();
        // Static methods don't receive `this`
        let this_opt = if method_def.is_static {
            None
        } else {
            Some(obj_id)
        };
        self.execute_method_body(this_opt, &method_def, args, method)
    }

    fn execute_randomize(&mut self, obj_id: ObjId, class_name: &str) -> Result<LogicVec, SimError> {
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
        let mut before_map: std::collections::HashMap<String, std::collections::HashSet<String>> =
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
        let mut ordered_fields: Vec<String> = Vec::new();
        let mut remaining: std::collections::HashSet<String> =
            class_def.rand_fields.iter().cloned().collect();
        for fname in &class_def.rand_fields {
            if before_map.contains_key(fname) && remaining.contains(fname) {
                ordered_fields.push(fname.clone());
                remaining.remove(fname);
            }
        }
        for fname in &class_def.rand_fields {
            if remaining.contains(fname) {
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

    fn execute_randomize_with(
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

        let mut before_map: std::collections::HashMap<String, std::collections::HashSet<String>> =
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

        let mut ordered_fields: Vec<String> = Vec::new();
        let mut remaining: std::collections::HashSet<String> =
            class_def.rand_fields.iter().cloned().collect();
        for fname in &class_def.rand_fields {
            if before_map.contains_key(fname) && remaining.contains(fname) {
                ordered_fields.push(fname.clone());
                remaining.remove(fname);
            }
        }
        for fname in &class_def.rand_fields {
            if remaining.contains(fname) {
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

    fn execute_mailbox_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "new" => Ok(LogicVec::from_u64(1, 1)),
            "put" => {
                if args.is_empty() {
                    return Err(SimError::runtime("mailbox::put expects 1 argument"));
                }
                self.mailbox_queues
                    .entry(obj_id)
                    .or_default()
                    .push(args[0].clone());
                Ok(LogicVec::from_u64(1, 1))
            }
            "get" => {
                let q = self
                    .mailbox_queues
                    .get_mut(&obj_id)
                    .ok_or_else(|| SimError::runtime("mailbox not initialized"))?;
                if q.is_empty() {
                    return Ok(LogicVec::default());
                }
                Ok(q.remove(0))
            }
            "try_get" => {
                let q = self
                    .mailbox_queues
                    .get_mut(&obj_id)
                    .ok_or_else(|| SimError::runtime("mailbox not initialized"))?;
                if q.is_empty() {
                    return Ok(LogicVec::from_u64(0, 1));
                }
                let _ = q.remove(0);
                Ok(LogicVec::from_u64(1, 1))
            }
            "try_put" => {
                if args.is_empty() {
                    return Err(SimError::runtime("mailbox::try_put expects 1 argument"));
                }
                self.mailbox_queues
                    .entry(obj_id)
                    .or_default()
                    .push(args[0].clone());
                Ok(LogicVec::from_u64(1, 1))
            }
            "num" => {
                let q = self
                    .mailbox_queues
                    .get(&obj_id)
                    .ok_or_else(|| SimError::runtime("mailbox not initialized"))?;
                Ok(LogicVec::from_u64(q.len() as u64, 32))
            }
            _ => Err(SimError::runtime(format!(
                "unknown mailbox method: {}",
                method
            ))),
        }
    }

    fn execute_semaphore_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "new" => {
                let init = if !args.is_empty() {
                    args[0].to_u64() as u32
                } else {
                    0
                };
                self.semaphore_counts.insert(obj_id, init);
                Ok(LogicVec::from_u64(1, 1))
            }
            "get" => {
                let key_count = if !args.is_empty() {
                    args[0].to_u64() as u32
                } else {
                    1
                };
                let c = self
                    .semaphore_counts
                    .get_mut(&obj_id)
                    .ok_or_else(|| SimError::runtime("semaphore not initialized"))?;
                if *c < key_count {
                    return Err(SimError::runtime("semaphore::get: insufficient keys"));
                }
                *c -= key_count;
                Ok(LogicVec::from_u64(*c as u64, 32))
            }
            "put" => {
                let key_count = if !args.is_empty() {
                    args[0].to_u64() as u32
                } else {
                    1
                };
                let c = self
                    .semaphore_counts
                    .get_mut(&obj_id)
                    .ok_or_else(|| SimError::runtime("semaphore not initialized"))?;
                *c += key_count;
                Ok(LogicVec::from_u64(*c as u64, 32))
            }
            "try_get" => {
                let key_count = if !args.is_empty() {
                    args[0].to_u64() as u32
                } else {
                    1
                };
                let c = self
                    .semaphore_counts
                    .get_mut(&obj_id)
                    .ok_or_else(|| SimError::runtime("semaphore not initialized"))?;
                if *c >= key_count {
                    *c -= key_count;
                    Ok(LogicVec::from_u64(1, 1))
                } else {
                    Ok(LogicVec::from_u64(0, 1))
                }
            }
            _ => Err(SimError::runtime(format!(
                "unknown semaphore method: {}",
                method
            ))),
        }
    }

    fn execute_process_method(
        &mut self,
        _obj_id: ObjId,
        method: &str,
        _args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "status" => {
                let status = self
                    .process_map
                    .get(&_obj_id)
                    .map(|p| p.status as u64)
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(status, 32))
            }
            "kill" => {
                let conts = if let Some(pi) = self.process_map.get_mut(&_obj_id) {
                    pi.status = ProcessStatus::Killed;
                    std::mem::take(&mut pi.await_continuations)
                } else {
                    Vec::new()
                };
                for cont in conts {
                    self.evaluate_block_with_delay(&cont)?;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "await" => {
                let status = self
                    .process_map
                    .get(&_obj_id)
                    .map(|p| p.status)
                    .unwrap_or(ProcessStatus::Finished);
                if status == ProcessStatus::Finished || status == ProcessStatus::Killed {
                    return Ok(LogicVec::from_u64(1, 1));
                }
                // Mark target as awaited — current process will yield at post-statement check
                self.pending_await_target = Some(_obj_id);
                Ok(LogicVec::from_u64(1, 1))
            }
            "self" => Ok(LogicVec::from_u64(_obj_id as u64, 64)),
            "suspend" => {
                if let Some(pi) = self.process_map.get_mut(&_obj_id) {
                    pi.status = ProcessStatus::Suspended;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "resume" => {
                if let Some(pi) = self.process_map.get_mut(&_obj_id) {
                    pi.status = ProcessStatus::Running;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => Err(SimError::runtime(format!(
                "unknown process method: {}",
                method
            ))),
        }
    }

    fn execute_uvm_object_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "new" => {
                let name = if !args.is_empty() {
                    logicvec_to_string(&args[0])
                } else {
                    String::new()
                };
                self.uvm_object_data.insert(obj_id, UvmObjectData { name });
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_name" => {
                let data = self
                    .uvm_object_data
                    .get(&obj_id)
                    .ok_or_else(|| SimError::runtime("uvm_object not initialized"))?;
                Ok(string_to_logicvec(&data.name))
            }
            "set_name" => {
                let name = if !args.is_empty() {
                    logicvec_to_string(&args[0])
                } else {
                    String::new()
                };
                if let Some(data) = self.uvm_object_data.get_mut(&obj_id) {
                    data.name = name;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_type_name" => {
                let class_name = self
                    .state
                    .get_object(obj_id)
                    .map(|o| o.class_name.clone())
                    .unwrap_or_default();
                Ok(string_to_logicvec(&class_name))
            }
            "print" => {
                let data = self
                    .uvm_object_data
                    .get(&obj_id)
                    .ok_or_else(|| SimError::runtime("uvm_object not initialized"))?;
                let class_name = self
                    .state
                    .get_object(obj_id)
                    .map(|o| o.class_name.clone())
                    .unwrap_or_default();
                println!(
                    "UVM_INFO @ {}: {} [{}]",
                    self.current_time, data.name, class_name
                );
                Ok(LogicVec::from_u64(1, 1))
            }
            "raise_objection" => {
                self.objection_count = self.objection_count.saturating_add(1);
                let name = self
                    .uvm_object_data
                    .get(&obj_id)
                    .map(|d| d.name.as_str())
                    .unwrap_or("unknown");
                println!(
                    "UVM_OBJECTION: {} raised (count={})",
                    name, self.objection_count
                );
                Ok(LogicVec::from_u64(1, 1))
            }
            "drop_objection" => {
                let name = self
                    .uvm_object_data
                    .get(&obj_id)
                    .map(|d| d.name.as_str())
                    .unwrap_or("unknown");
                if self.objection_count > 0 {
                    self.objection_count -= 1;
                }
                println!(
                    "UVM_OBJECTION: {} dropped (count={})",
                    name, self.objection_count
                );
                if self.objection_count == 0 && !self.objection_triggered {
                    self.objection_triggered = true;
                    println!("UVM_PHASE: All objections dropped, ending test");
                    // Schedule end-of-test via $finish behavior
                    self.running = false;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => Err(SimError::runtime(format!(
                "uvm_object::{} not implemented",
                method
            ))),
        }
    }

    fn execute_uvm_report_object_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "uvm_report_info" => {
                let id = args
                    .get(0)
                    .map(|a| logicvec_to_string(a))
                    .unwrap_or_default();
                let msg = args
                    .get(1)
                    .map(|a| logicvec_to_string(a))
                    .unwrap_or_default();
                eprintln!("UVM_INFO @ {}: {} [{}]", self.current_time, msg, id);
                Ok(LogicVec::from_u64(1, 1))
            }
            "uvm_report_warning" => {
                let id = args
                    .get(0)
                    .map(|a| logicvec_to_string(a))
                    .unwrap_or_default();
                let msg = args
                    .get(1)
                    .map(|a| logicvec_to_string(a))
                    .unwrap_or_default();
                eprintln!("UVM_WARNING @ {}: {} [{}]", self.current_time, msg, id);
                Ok(LogicVec::from_u64(1, 1))
            }
            "uvm_report_error" => {
                let id = args
                    .get(0)
                    .map(|a| logicvec_to_string(a))
                    .unwrap_or_default();
                let msg = args
                    .get(1)
                    .map(|a| logicvec_to_string(a))
                    .unwrap_or_default();
                eprintln!("UVM_ERROR @ {}: {} [{}]", self.current_time, msg, id);
                Ok(LogicVec::from_u64(1, 1))
            }
            "uvm_report_fatal" => {
                let id = args
                    .get(0)
                    .map(|a| logicvec_to_string(a))
                    .unwrap_or_default();
                let msg = args
                    .get(1)
                    .map(|a| logicvec_to_string(a))
                    .unwrap_or_default();
                eprintln!("UVM_FATAL @ {}: {} [{}]", self.current_time, msg, id);
                self.running = false;
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_object_method(obj_id, method, args),
        }
    }

    fn execute_uvm_component_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "new" => {
                let name = if !args.is_empty() {
                    logicvec_to_string(&args[0])
                } else {
                    String::new()
                };
                let parent_obj = args.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                self.uvm_object_data
                    .insert(obj_id, UvmObjectData { name: name.clone() });
                let mut cd = UvmComponentData {
                    parent: None,
                    children: Vec::new(),
                    report_verbosity: 2,
                };
                if parent_obj != 0 {
                    cd.parent = Some(parent_obj);
                    if let Some(pd) = self.uvm_component_data.get_mut(&parent_obj) {
                        pd.children.push(obj_id);
                    }
                }
                self.uvm_component_data.insert(obj_id, cd);
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_full_name" => {
                let mut names = Vec::new();
                let mut current = Some(obj_id);
                while let Some(id) = current {
                    let n = self
                        .uvm_object_data
                        .get(&id)
                        .map(|d| d.name.clone())
                        .unwrap_or_default();
                    names.push(n);
                    current = self.uvm_component_data.get(&id).and_then(|d| d.parent);
                }
                names.reverse();
                let full = names.join(".");
                Ok(string_to_logicvec(&full))
            }
            "get_parent" => {
                let pid = self
                    .uvm_component_data
                    .get(&obj_id)
                    .and_then(|d| d.parent)
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(pid as u64, 64))
            }
            "get_num_children" => {
                let n = self
                    .uvm_component_data
                    .get(&obj_id)
                    .map(|d| d.children.len() as u64)
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(n, 32))
            }
            "get_child" => {
                let idx = args.first().map(|a| a.to_u64() as usize).unwrap_or(0);
                let cid = self
                    .uvm_component_data
                    .get(&obj_id)
                    .and_then(|d| d.children.get(idx).copied())
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(cid as u64, 64))
            }
            "has_child" => {
                let name = args
                    .first()
                    .map(|a| logicvec_to_string(a))
                    .unwrap_or_default();
                let found = self
                    .uvm_component_data
                    .get(&obj_id)
                    .map(|d| {
                        d.children.iter().any(|cid| {
                            self.uvm_object_data
                                .get(cid)
                                .map(|od| od.name == name)
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false);
                Ok(LogicVec::from_u64(if found { 1 } else { 0 }, 1))
            }
            "set_report_verbosity" => {
                let level = args.first().map(|a| a.to_u64() as u32).unwrap_or(2);
                if let Some(d) = self.uvm_component_data.get_mut(&obj_id) {
                    d.report_verbosity = level;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_report_verbosity" => {
                let level = self
                    .uvm_component_data
                    .get(&obj_id)
                    .map(|d| d.report_verbosity)
                    .unwrap_or(2);
                Ok(LogicVec::from_u64(level as u64, 32))
            }
            "build_phase" | "connect_phase" | "run_phase" => Ok(LogicVec::from_u64(1, 1)),
            _ => self.execute_uvm_report_object_method(obj_id, method, args),
        }
    }

    fn execute_uvm_sequence_item_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "new" => {
                let name = if !args.is_empty() {
                    logicvec_to_string(&args[0])
                } else {
                    String::new()
                };
                self.uvm_object_data
                    .entry(obj_id)
                    .or_insert_with(|| UvmObjectData { name });
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_type_name" => {
                let class_name = self
                    .state
                    .get_object(obj_id)
                    .map(|o| o.class_name.clone())
                    .unwrap_or_default();
                Ok(string_to_logicvec(&class_name))
            }
            _ => self.execute_uvm_object_method(obj_id, method, args),
        }
    }

    fn execute_uvm_sequence_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "new" => {
                let name = if !args.is_empty() {
                    logicvec_to_string(&args[0])
                } else {
                    String::new()
                };
                self.uvm_object_data
                    .entry(obj_id)
                    .or_insert_with(|| UvmObjectData { name });
                Ok(LogicVec::from_u64(1, 1))
            }
            "start" => {
                // args[0] = sequencer obj_id
                let seqr_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                // Store sequencer obj_id on the sequence object's fields
                if let Some(obj) = self.state.get_object_mut(obj_id) {
                    obj.fields.insert(
                        "__sequencer".to_string(),
                        LogicVec::from_u64(seqr_id as u64, 64),
                    );
                }
                // Call body()
                if self
                    .find_method_in_hierarchy(
                        &{
                            self.state
                                .get_object(obj_id)
                                .map(|o| o.class_name.clone())
                                .unwrap_or_default()
                        },
                        "body",
                    )
                    .is_ok()
                {
                    self.execute_method(obj_id, "body", &[])?;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "body" => Ok(LogicVec::from_u64(1, 1)),
            "start_item" => {
                let item_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                // Get sequencer from stored field
                let seqr_id = self
                    .state
                    .get_object(obj_id)
                    .and_then(|o| o.fields.get("__sequencer"))
                    .map(|v| v.to_u64() as ObjId)
                    .unwrap_or(0);
                if seqr_id != 0 {
                    self.uvm_sequencer_data
                        .entry(seqr_id)
                        .or_insert_with(|| UvmSequencerData {
                            item_queue: Vec::new(),
                            current_item: None,
                        })
                        .item_queue
                        .push(item_id);
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "finish_item" => Ok(LogicVec::from_u64(1, 1)),
            "get_sequencer" => {
                let seqr_id = self
                    .state
                    .get_object(obj_id)
                    .and_then(|o| o.fields.get("__sequencer"))
                    .cloned()
                    .unwrap_or(LogicVec::from_u64(0, 64));
                Ok(seqr_id)
            }
            "create" => {
                let name = args
                    .first()
                    .map(|a| logicvec_to_string(a))
                    .unwrap_or_default();
                // Create a new object of the sequence's type
                let class_name = self
                    .state
                    .get_object(obj_id)
                    .map(|o| o.class_name.clone())
                    .unwrap_or_default();
                let child = self.state.alloc_object(&class_name);
                // Set name on the new object
                self.uvm_object_data
                    .entry(child)
                    .or_insert_with(|| UvmObjectData { name });
                // Initialize fields from class def
                if let Some(cls) = self.design.classes.get(&class_name) {
                    if let Some(obj) = self.state.get_object_mut(child) {
                        for field in &cls.fields {
                            obj.fields
                                .entry(field.name.clone())
                                .or_insert_with(|| LogicVec::from_u64(0, field.width));
                        }
                    }
                }
                Ok(LogicVec::from_u64(child as u64, 64))
            }
            _ => self.execute_uvm_sequence_item_method(obj_id, method, args),
        }
    }

    fn execute_uvm_sequencer_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "new" => {
                let name = if !args.is_empty() {
                    logicvec_to_string(&args[0])
                } else {
                    String::new()
                };
                let parent_obj = args.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                self.uvm_object_data
                    .insert(obj_id, UvmObjectData { name: name.clone() });
                let mut cd = UvmComponentData {
                    parent: None,
                    children: Vec::new(),
                    report_verbosity: 2,
                };
                if parent_obj != 0 {
                    cd.parent = Some(parent_obj);
                    if let Some(pd) = self.uvm_component_data.get_mut(&parent_obj) {
                        pd.children.push(obj_id);
                    }
                }
                self.uvm_component_data.insert(obj_id, cd);
                self.uvm_sequencer_data.insert(
                    obj_id,
                    UvmSequencerData {
                        item_queue: Vec::new(),
                        current_item: None,
                    },
                );
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_next_item" => {
                let data = self
                    .uvm_sequencer_data
                    .get_mut(&obj_id)
                    .ok_or_else(|| SimError::runtime("sequencer not initialized"))?;
                let item = data.item_queue.first().copied().unwrap_or(0);
                data.current_item = data.item_queue.first().copied();
                Ok(LogicVec::from_u64(item as u64, 64))
            }
            "item_done" => {
                if let Some(data) = self.uvm_sequencer_data.get_mut(&obj_id) {
                    if data.current_item.is_some() {
                        data.item_queue.remove(0);
                        data.current_item = None;
                    }
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_component_method(obj_id, method, args),
        }
    }

    fn execute_uvm_driver_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "new" => {
                let name = if !args.is_empty() {
                    logicvec_to_string(&args[0])
                } else {
                    String::new()
                };
                let parent_obj = args.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                self.uvm_object_data
                    .insert(obj_id, UvmObjectData { name: name.clone() });
                let mut cd = UvmComponentData {
                    parent: None,
                    children: Vec::new(),
                    report_verbosity: 2,
                };
                if parent_obj != 0 {
                    cd.parent = Some(parent_obj);
                    if let Some(pd) = self.uvm_component_data.get_mut(&parent_obj) {
                        pd.children.push(obj_id);
                    }
                }
                self.uvm_component_data.insert(obj_id, cd);
                self.uvm_driver_data.insert(
                    obj_id,
                    UvmDriverData {
                        sequencer_id: None,
                        current_item: None,
                    },
                );
                Ok(LogicVec::from_u64(1, 1))
            }
            "set_sequencer" => {
                let seqr_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                if let Some(data) = self.uvm_driver_data.get_mut(&obj_id) {
                    data.sequencer_id = Some(seqr_id);
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_next_item" => {
                let data = self
                    .uvm_driver_data
                    .get(&obj_id)
                    .ok_or_else(|| SimError::runtime("driver not initialized"))?;
                let seqr_id = data.sequencer_id.unwrap_or(0);
                if seqr_id != 0 {
                    self.execute_uvm_sequencer_method(seqr_id, "get_next_item", args)
                } else {
                    Ok(LogicVec::from_u64(0, 64))
                }
            }
            "item_done" => {
                let data = self
                    .uvm_driver_data
                    .get(&obj_id)
                    .ok_or_else(|| SimError::runtime("driver not initialized"))?;
                let seqr_id = data.sequencer_id.unwrap_or(0);
                if seqr_id != 0 {
                    self.execute_uvm_sequencer_method(seqr_id, "item_done", args)
                } else {
                    Ok(LogicVec::from_u64(1, 1))
                }
            }
            _ => self.execute_uvm_component_method(obj_id, method, args),
        }
    }

    fn execute_uvm_monitor_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "new" => {
                let name = if !args.is_empty() {
                    logicvec_to_string(&args[0])
                } else {
                    String::new()
                };
                let parent_obj = args.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                self.uvm_object_data
                    .insert(obj_id, UvmObjectData { name: name.clone() });
                let mut cd = UvmComponentData {
                    parent: None,
                    children: Vec::new(),
                    report_verbosity: 2,
                };
                if parent_obj != 0 {
                    cd.parent = Some(parent_obj);
                    if let Some(pd) = self.uvm_component_data.get_mut(&parent_obj) {
                        pd.children.push(obj_id);
                    }
                }
                self.uvm_component_data.insert(obj_id, cd);
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_component_method(obj_id, method, args),
        }
    }

    fn execute_uvm_analysis_port_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "new" => {
                let name = if !args.is_empty() {
                    logicvec_to_string(&args[0])
                } else {
                    String::new()
                };
                self.uvm_analysis_port_data.insert(
                    obj_id,
                    UvmAnalysisPortData {
                        connections: Vec::new(),
                        name: name.clone(),
                    },
                );
                self.uvm_object_data
                    .entry(obj_id)
                    .or_insert_with(|| UvmObjectData { name });
                Ok(LogicVec::from_u64(1, 1))
            }
            "connect" => {
                let imp_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                if let Some(data) = self.uvm_analysis_port_data.get_mut(&obj_id) {
                    data.connections.push(imp_id);
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "write" => {
                let item_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let connections = self
                    .uvm_analysis_port_data
                    .get(&obj_id)
                    .map(|d| d.connections.clone())
                    .unwrap_or_default();
                for imp_id in &connections {
                    let imp_args = vec![LogicVec::from_u64(item_id as u64, 64)];
                    self.execute_uvm_analysis_imp_method(*imp_id, "write", &imp_args)?;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_object_method(obj_id, method, args),
        }
    }

    fn execute_uvm_analysis_imp_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "new" => {
                let name = if !args.is_empty() {
                    logicvec_to_string(&args[0])
                } else {
                    String::new()
                };
                let parent_obj = args.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                self.uvm_analysis_imp_data.insert(
                    obj_id,
                    UvmAnalysisImpData {
                        parent: Some(parent_obj),
                        name: name.clone(),
                    },
                );
                self.uvm_object_data
                    .entry(obj_id)
                    .or_insert_with(|| UvmObjectData { name });
                Ok(LogicVec::from_u64(1, 1))
            }
            "write" => {
                let item_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let parent = self
                    .uvm_analysis_imp_data
                    .get(&obj_id)
                    .and_then(|d| d.parent)
                    .unwrap_or(0);
                let parent_name = if parent != 0 {
                    self.state
                        .get_object(parent)
                        .map(|o| o.class_name.clone())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                if parent != 0
                    && !parent_name.is_empty()
                    && self.find_method_in_hierarchy(&parent_name, "write").is_ok()
                {
                    let write_args = vec![LogicVec::from_u64(item_id as u64, 64)];
                    self.execute_method(parent, "write", &write_args)?;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_object_method(obj_id, method, args),
        }
    }

    fn execute_super_method(
        &mut self,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        let obj_id = self
            .current_this
            .ok_or_else(|| SimError::runtime("'super' used outside class method"))?;
        let class_name = self
            .state
            .get_object(obj_id)
            .map(|o| o.class_name.clone())
            .unwrap_or_default();
        let parent = self
            .design
            .classes
            .get(&class_name)
            .and_then(|c| c.extends.clone())
            .ok_or_else(|| {
                SimError::runtime(format!(
                    "class '{}' has no parent for super call",
                    class_name
                ))
            })?;
        // Check hierarchy from most specific to least
        if parent == "__uvm_driver" || self.is_uvm_driver_hierarchy(&parent) {
            return self.execute_uvm_driver_method(obj_id, method, args);
        }
        if parent == "__uvm_monitor" || self.is_uvm_monitor_hierarchy(&parent) {
            return self.execute_uvm_monitor_method(obj_id, method, args);
        }
        if parent == "__uvm_sequencer" || self.is_uvm_sequencer_hierarchy(&parent) {
            return self.execute_uvm_sequencer_method(obj_id, method, args);
        }
        if parent == "__uvm_sequence" || self.is_uvm_sequence_hierarchy(&parent) {
            return self.execute_uvm_sequence_method(obj_id, method, args);
        }
        if parent == "__uvm_sequence_item" || self.is_uvm_sequence_item_hierarchy(&parent) {
            return self.execute_uvm_sequence_item_method(obj_id, method, args);
        }
        if parent == "__uvm_analysis_port" || self.is_uvm_analysis_port_hierarchy(&parent) {
            return self.execute_uvm_analysis_port_method(obj_id, method, args);
        }
        if parent == "__uvm_analysis_imp" || self.is_uvm_analysis_imp_hierarchy(&parent) {
            return self.execute_uvm_analysis_imp_method(obj_id, method, args);
        }
        // Check if parent is uvm_component hierarchy
        if parent == "__uvm_component" || self.is_uvm_component_hierarchy(&parent) {
            return self.execute_uvm_component_method(obj_id, method, args);
        }
        // Check if parent is uvm_report_object hierarchy
        if parent == "__uvm_report_object" || self.is_uvm_report_object_hierarchy(&parent) {
            return self.execute_uvm_report_object_method(obj_id, method, args);
        }
        // Check if parent is uvm_object hierarchy
        if parent == "__uvm_object" || self.is_uvm_object_hierarchy(&parent) {
            return self.execute_uvm_object_method(obj_id, method, args);
        }
        // Super dispatch: start search from parent class, skipping current class override
        let method_def = self.find_method_in_hierarchy(&parent, method)?.clone();
        self.execute_method_body(Some(obj_id), &method_def, args, method)
    }

    fn execute_method_body(
        &mut self,
        obj_id: Option<ObjId>,
        method_def: &IrClassMethod,
        args: &[LogicVec],
        method: &str,
    ) -> Result<LogicVec, SimError> {
        let old_this = self.current_this;
        if let Some(oid) = obj_id {
            self.current_this = Some(oid);
        }

        let mut local_signals: HashMap<String, LogicVec> = HashMap::new();
        for (i, port) in method_def.ports.iter().enumerate() {
            let port_width = port.resolved_width(&HashMap::new()).unwrap_or(1);
            let val = if i < args.len() {
                args[i].resize(port_width)
            } else {
                LogicVec::new(port_width)
            };
            local_signals.insert(port.name.clone(), val);
        }

        for decl in &method_def.decls {
            for dv in &decl.names {
                let w = dv.resolved_width(&HashMap::new()).unwrap_or(1);
                local_signals.insert(dv.name.clone(), LogicVec::new(w));
            }
        }

        let depth = self.method_locals.len();
        self.method_locals.push(local_signals);

        let old_method = self.current_method.clone();
        self.current_method = Some(method.to_string());

        if !method_def.stmts.is_empty() {
            if method_def.is_task {
                let completed = self.evaluate_ast_block_with_delay_fork(&method_def.stmts, None)?;
                if !completed {
                    // Task suspended by delay — keep scope & context alive for continuation
                    self.current_method = old_method;
                    return Ok(LogicVec::new(0));
                }
            } else {
                let body = Stmt::Block {
                    stmts: method_def.stmts.clone(),
                };
                self.evaluate_ast_stmt(&body)?;
            }
        }

        let return_val = if method_def.is_task {
            LogicVec::new(0) // tasks return void
        } else {
            self.get_local(method).unwrap_or_else(|| LogicVec::new(1))
        };

        self.current_method = old_method;
        self.method_locals.truncate(depth);
        self.current_this = old_this;
        Ok(return_val)
    }

    fn get_foreach_count(&self, array_var: &str) -> usize {
        if let Some(obj_id) = self.current_this {
            if let Some(obj) = self.state.get_object(obj_id) {
                if let Some(cls) = self.design.classes.get(&obj.class_name) {
                    for field in &cls.fields {
                        if field.name == array_var {
                            return field.array_depth;
                        }
                    }
                }
            }
        }
        1
    }

    fn get_field_elem_width(&self, expr: &Expr) -> Option<usize> {
        match expr {
            Expr::Ident(name) => {
                if let Some(obj_id) = self.current_this {
                    if let Some(obj) = self.state.get_object(obj_id) {
                        if let Some(cls) = self.design.classes.get(&obj.class_name) {
                            for field in &cls.fields {
                                if field.name == *name && field.array_depth > 1 {
                                    return Some(field.elem_width);
                                }
                            }
                        }
                    }
                }
                None
            }
            Expr::MemberAccess { obj, field } => {
                if let Expr::Ident(s) = obj.as_ref() {
                    if s == "this" {
                        if let Some(obj_id) = self.current_this {
                            if let Some(obj) = self.state.get_object(obj_id) {
                                if let Some(cls) = self.design.classes.get(&obj.class_name) {
                                    for f in &cls.fields {
                                        if f.name == *field && f.array_depth > 1 {
                                            return Some(f.elem_width);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn find_method_in_hierarchy(
        &self,
        class_name: &str,
        method: &str,
    ) -> Result<IrClassMethod, SimError> {
        let mut current = class_name;
        loop {
            if let Some(cls) = self.design.classes.get(current) {
                if let Some(m) = cls.methods.iter().find(|m| m.name == method) {
                    return Ok(m.clone());
                }
                if let Some(parent) = &cls.extends {
                    current = parent;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        Err(SimError::runtime(format!(
            "method '{}' not found in class '{}' or its parents",
            method, class_name
        )))
    }
}
impl SimulationEngine {
    fn check_with_clause(
        &mut self,
        with_clause: Option<&IrExpr>,
        elem: &LogicVec,
    ) -> Result<bool, SimError> {
        if let Some(wc) = with_clause {
            let depth = self.method_locals.len();
            let mut scope = std::collections::HashMap::new();
            scope.insert("item".to_string(), elem.clone());
            self.method_locals.push(scope);
            let result = self.evaluate_expr(wc)?.to_bool().unwrap_or(false);
            self.method_locals.truncate(depth);
            Ok(result)
        } else {
            Ok(true)
        }
    }

    fn evaluate_array_method(
        &mut self,
        sig_id: SignalId,
        sig: &SignalInfo,
        method: &str,
        args: &[IrExpr],
        with_clause: Option<&IrExpr>,
    ) -> Result<LogicVec, SimError> {
        // Check if this is an associative array method
        if sig.is_associative {
            // Evaluate args first to avoid borrow conflicts with assoc_data access
            let args_eval: Vec<LogicVec> = args
                .iter()
                .map(|a| self.evaluate_expr(a))
                .collect::<Result<Vec<_>, SimError>>()?;
            let assoc_map = self.assoc_data.entry(sig_id).or_insert_with(HashMap::new);
            match method {
                "num" => {
                    let n = assoc_map.len();
                    return Ok(LogicVec::from_u64(n as u64, 32));
                }
                "delete" => {
                    if args_eval.is_empty() {
                        assoc_map.clear();
                    } else {
                        assoc_map.remove(&args_eval[0]);
                    }
                    return Ok(LogicVec::new(0));
                }
                "exists" => {
                    let found = assoc_map.contains_key(&args_eval[0]);
                    return Ok(LogicVec::from_u64(if found { 1 } else { 0 }, 1));
                }
                "first" => {
                    if let Some(key) = assoc_map.keys().next() {
                        return Ok(key.clone());
                    }
                    return Ok(LogicVec::new(0));
                }
                "last" => {
                    if let Some(key) = assoc_map.keys().last() {
                        return Ok(key.clone());
                    }
                    return Ok(LogicVec::new(0));
                }
                "next" => {
                    if let Some(key) = args_eval.first() {
                        let mut found = false;
                        let mut next_val = LogicVec::new(0);
                        for k in assoc_map.keys() {
                            if found {
                                next_val = k.clone();
                                break;
                            }
                            if *k == *key {
                                found = true;
                            }
                        }
                        return Ok(next_val);
                    }
                    return Ok(LogicVec::new(0));
                }
                "prev" => {
                    if let Some(key) = args_eval.first() {
                        let mut prev_val = LogicVec::new(0);
                        for k in assoc_map.keys() {
                            if *k == *key {
                                return Ok(prev_val);
                            }
                            prev_val = k.clone();
                        }
                        return Ok(LogicVec::new(0));
                    }
                    return Ok(LogicVec::new(0));
                }
                _ => {
                    // Fall through to default array methods (like push_back, etc.)
                }
            }
        }
        match method {
            "size" => {
                let lv = self.state.read_signal(sig_id);
                let count = if sig.elem_width > 0 {
                    lv.width / sig.elem_width
                } else {
                    0
                };
                Ok(LogicVec::from_u64(count as u64, 32))
            }
            "delete" => {
                if let Some(index_expr) = args.first() {
                    let idx_val = self.evaluate_expr(index_expr)?;
                    let idx = idx_val.to_u64() as usize;
                    let lv = self.state.read_signal(sig_id);
                    let elem_width = sig.elem_width;
                    let count = if elem_width > 0 {
                        lv.width / elem_width
                    } else {
                        0
                    };
                    if idx >= count {
                        return Err(SimError::runtime(format!(
                            "delete index {} out of range (size {})",
                            idx, count
                        )));
                    }
                    let before = lv.bits[..idx * elem_width].to_vec();
                    let after = lv.bits[(idx + 1) * elem_width..].to_vec();
                    let mut remaining = Vec::with_capacity(before.len() + after.len());
                    remaining.extend(before);
                    remaining.extend(after);
                    let new_lv = LogicVec {
                        width: remaining.len(),
                        bits: remaining,
                    };
                    self.state.write_signal(sig_id, new_lv);
                    Ok(LogicVec::new(0))
                } else {
                    self.state.write_signal(sig_id, LogicVec::new(0));
                    Ok(LogicVec::new(0))
                }
            }
            "pop_front" => {
                let lv = self.state.read_signal(sig_id);
                let elem_width = sig.elem_width;
                if lv.width < elem_width {
                    return Err(SimError::runtime("pop_front on empty queue"));
                }
                let mut bits = Vec::with_capacity(elem_width);
                for i in 0..elem_width {
                    bits.push(lv.bits.get(i).copied().unwrap_or(LogicVal::X));
                }
                let result = LogicVec {
                    width: elem_width,
                    bits,
                };
                let remaining = LogicVec {
                    width: lv.width - elem_width,
                    bits: lv.bits[elem_width..].to_vec(),
                };
                self.state.write_signal(sig_id, remaining);
                Ok(result)
            }
            "pop_back" => {
                let lv = self.state.read_signal(sig_id);
                let elem_width = sig.elem_width;
                if lv.width < elem_width {
                    return Err(SimError::runtime("pop_back on empty queue"));
                }
                let start = lv.width - elem_width;
                let mut bits = Vec::with_capacity(elem_width);
                for i in start..lv.width {
                    bits.push(lv.bits.get(i).copied().unwrap_or(LogicVal::X));
                }
                let result = LogicVec {
                    width: elem_width,
                    bits,
                };
                let remaining = LogicVec {
                    width: lv.width - elem_width,
                    bits: lv.bits[..start].to_vec(),
                };
                self.state.write_signal(sig_id, remaining);
                Ok(result)
            }
            "push_front" => {
                let arg_val = if let Some(a) = args.first() {
                    self.evaluate_expr(a)?
                } else {
                    return Err(SimError::runtime("push_front expects 1 argument"));
                };
                let elem_width = sig.elem_width;
                let padded = if arg_val.width >= elem_width {
                    let bits = arg_val.bits[..elem_width].to_vec();
                    LogicVec {
                        width: elem_width,
                        bits,
                    }
                } else {
                    let mut bits = arg_val.bits.clone();
                    bits.resize(elem_width, LogicVal::X);
                    LogicVec {
                        width: elem_width,
                        bits,
                    }
                };
                let mut existing = self.state.read_signal(sig_id).clone();
                let mut new_bits = Vec::with_capacity(existing.width + elem_width);
                new_bits.extend(padded.bits.iter().copied());
                new_bits.extend(existing.bits.iter().copied());
                existing.bits = new_bits;
                existing.width += elem_width;
                self.state.write_signal(sig_id, existing);
                Ok(LogicVec::new(0))
            }
            "exists" => {
                let index_expr = args
                    .first()
                    .ok_or_else(|| SimError::runtime("exists expects 1 argument"))?;
                let idx_val = self.evaluate_expr(index_expr)?;
                let idx = idx_val.to_u64() as usize;
                let lv = self.state.read_signal(sig_id);
                let elem_width = sig.elem_width;
                let count = if elem_width > 0 {
                    lv.width / elem_width
                } else {
                    0
                };
                Ok(LogicVec::from_u64(if idx < count { 1 } else { 0 }, 1))
            }
            "push_back" => {
                let arg_val = if let Some(a) = args.first() {
                    self.evaluate_expr(a)?
                } else {
                    return Err(SimError::runtime("push_back expects 1 argument"));
                };
                let elem_width = sig.elem_width;
                let padded = if arg_val.width >= elem_width {
                    let bits = arg_val.bits[..elem_width].to_vec();
                    LogicVec {
                        width: elem_width,
                        bits,
                    }
                } else {
                    let mut bits = arg_val.bits.clone();
                    bits.resize(elem_width, LogicVal::X);
                    LogicVec {
                        width: elem_width,
                        bits,
                    }
                };
                let mut existing = self.state.read_signal(sig_id).clone();
                existing.bits.extend(padded.bits.iter().copied());
                existing.width += elem_width;
                self.state.write_signal(sig_id, existing);
                Ok(LogicVec::new(0))
            }
            "insert" => {
                if args.len() < 2 {
                    return Err(SimError::runtime(
                        "insert expects 2 arguments (index, value)",
                    ));
                }
                let idx_val = self.evaluate_expr(&args[0])?;
                let idx = idx_val.to_u64() as usize;
                let arg_val = self.evaluate_expr(&args[1])?;
                let elem_width = sig.elem_width;
                let padded = if arg_val.width >= elem_width {
                    let bits = arg_val.bits[..elem_width].to_vec();
                    LogicVec {
                        width: elem_width,
                        bits,
                    }
                } else {
                    let mut bits = arg_val.bits.clone();
                    bits.resize(elem_width, LogicVal::X);
                    LogicVec {
                        width: elem_width,
                        bits,
                    }
                };
                let mut existing = self.state.read_signal(sig_id).clone();
                let count = if elem_width > 0 {
                    existing.width / elem_width
                } else {
                    0
                };
                let pos = idx.min(count);
                let mut new_bits = Vec::with_capacity(existing.width + elem_width);
                new_bits.extend(existing.bits[..pos * elem_width].iter().copied());
                new_bits.extend(padded.bits.iter().copied());
                new_bits.extend(existing.bits[pos * elem_width..].iter().copied());
                existing.bits = new_bits;
                existing.width += elem_width;
                self.state.write_signal(sig_id, existing);
                Ok(LogicVec::new(0))
            }
            "reverse" => {
                let mut lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width / elem_width;
                    let mut new_bits = Vec::with_capacity(lv.width);
                    for i in (0..count).rev() {
                        for j in 0..elem_width {
                            new_bits.push(lv.bits[i * elem_width + j]);
                        }
                    }
                    lv.bits = new_bits;
                }
                self.state.write_signal(sig_id, lv);
                Ok(LogicVec::new(0))
            }
            "sort" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width / elem_width;
                    let mut elems: Vec<LogicVec> = (0..count)
                        .map(|i| {
                            let mut bits = Vec::with_capacity(elem_width);
                            for j in 0..elem_width {
                                bits.push(lv.bits[i * elem_width + j]);
                            }
                            LogicVec {
                                width: elem_width,
                                bits,
                            }
                        })
                        .collect();
                    elems.sort_by(|a, b| a.to_u64().cmp(&b.to_u64()));
                    let mut new_bits = Vec::with_capacity(lv.width);
                    for e in &elems {
                        new_bits.extend(e.bits.iter().copied());
                    }
                    let sorted = LogicVec {
                        width: lv.width,
                        bits: new_bits,
                    };
                    self.state.write_signal(sig_id, sorted);
                }
                Ok(LogicVec::new(0))
            }
            "rsort" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width / elem_width;
                    let mut elems: Vec<LogicVec> = (0..count)
                        .map(|i| {
                            let mut bits = Vec::with_capacity(elem_width);
                            for j in 0..elem_width {
                                bits.push(lv.bits[i * elem_width + j]);
                            }
                            LogicVec {
                                width: elem_width,
                                bits,
                            }
                        })
                        .collect();
                    elems.sort_by(|a, b| b.to_u64().cmp(&a.to_u64()));
                    let mut new_bits = Vec::with_capacity(lv.width);
                    for e in &elems {
                        new_bits.extend(e.bits.iter().copied());
                    }
                    let sorted = LogicVec {
                        width: lv.width,
                        bits: new_bits,
                    };
                    self.state.write_signal(sig_id, sorted);
                }
                Ok(LogicVec::new(0))
            }
            "shuffle" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width / elem_width;
                    let mut elems: Vec<LogicVec> = (0..count)
                        .map(|i| {
                            let mut bits = Vec::with_capacity(elem_width);
                            for j in 0..elem_width {
                                bits.push(lv.bits[i * elem_width + j]);
                            }
                            LogicVec {
                                width: elem_width,
                                bits,
                            }
                        })
                        .collect();
                    use rand::seq::SliceRandom;
                    elems.shuffle(&mut rand::thread_rng());
                    let mut new_bits = Vec::with_capacity(lv.width);
                    for e in &elems {
                        new_bits.extend(e.bits.iter().copied());
                    }
                    let shuffled = LogicVec {
                        width: lv.width,
                        bits: new_bits,
                    };
                    self.state.write_signal(sig_id, shuffled);
                }
                Ok(LogicVec::new(0))
            }
            // --- Reduction methods ---
            "sum" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width / elem_width;
                    let mut result: u64 = 0;
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits,
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        result = result.wrapping_add(elem.to_u64());
                    }
                    Ok(LogicVec::from_u64(result, elem_width.max(32)))
                } else {
                    Ok(LogicVec::new(0))
                }
            }
            "product" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width / elem_width;
                    let mut result: u64 = 1;
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits,
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        result = result.wrapping_mul(elem.to_u64());
                    }
                    Ok(LogicVec::from_u64(result, elem_width.max(32)))
                } else {
                    Ok(LogicVec::new(0))
                }
            }
            "and" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    let mut result = LogicVec::fill(LogicVal::One, elem_width);
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            let idx = i * elem_width + j;
                            bits.push(lv.bits.get(idx).copied().unwrap_or(LogicVal::X));
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits: bits.clone(),
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        for j in 0..elem_width {
                            if bits.get(j) == Some(&LogicVal::Zero) {
                                result.bits[j] = LogicVal::Zero;
                            }
                        }
                    }
                    Ok(result)
                } else {
                    Ok(LogicVec::fill(LogicVal::One, elem_width.max(1)))
                }
            }
            "or" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    let mut result = LogicVec::fill(LogicVal::Zero, elem_width);
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            let idx = i * elem_width + j;
                            bits.push(lv.bits.get(idx).copied().unwrap_or(LogicVal::X));
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits: bits.clone(),
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        for j in 0..elem_width {
                            if bits.get(j) == Some(&LogicVal::One) {
                                result.bits[j] = LogicVal::One;
                            }
                        }
                    }
                    Ok(result)
                } else {
                    Ok(LogicVec::fill(LogicVal::Zero, elem_width.max(1)))
                }
            }
            "xor" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    let mut result = LogicVec::fill(LogicVal::Zero, elem_width);
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            let idx = i * elem_width + j;
                            bits.push(lv.bits.get(idx).copied().unwrap_or(LogicVal::X));
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits: bits.clone(),
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        for j in 0..elem_width {
                            if bits.get(j) == Some(&LogicVal::One) {
                                result.bits[j] = match result.bits[j] {
                                    LogicVal::Zero => LogicVal::One,
                                    LogicVal::One => LogicVal::Zero,
                                    other => other,
                                };
                            }
                        }
                    }
                    Ok(result)
                } else {
                    Ok(LogicVec::fill(LogicVal::Zero, elem_width.max(1)))
                }
            }
            // --- Ordering methods ---
            "min" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    let mut min_val = u64::MAX;
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits,
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        let v = elem.to_u64();
                        if v < min_val {
                            min_val = v;
                        }
                    }
                    Ok(LogicVec::from_u64(min_val, elem_width))
                } else {
                    Ok(LogicVec::new(1))
                }
            }
            "max" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    let mut max_val: u64 = 0;
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits,
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        let v = elem.to_u64();
                        if v > max_val {
                            max_val = v;
                        }
                    }
                    Ok(LogicVec::from_u64(max_val, elem_width))
                } else {
                    Ok(LogicVec::new(1))
                }
            }
            "unique" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    let mut seen = std::collections::HashSet::new();
                    let mut new_bits = Vec::new();
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits,
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        if seen.insert(elem.to_u64()) {
                            for j in 0..elem_width {
                                let idx = i * elem_width + j;
                                new_bits.push(lv.bits.get(idx).copied().unwrap_or(LogicVal::X));
                            }
                        }
                    }
                    let result = LogicVec {
                        width: new_bits.len(),
                        bits: new_bits,
                    };
                    self.state.write_signal(sig_id, result);
                }
                Ok(LogicVec::new(0))
            }
            // --- Locator methods ---
            "find" | "find_first" | "find_last" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    if with_clause.is_some() {
                        // If with_clause is provided, iterate and find matching elements
                        let mut result_elems: Vec<LogicVec> = Vec::new();
                        if method == "find_last" {
                            for i in (0..count).rev() {
                                let mut bits = Vec::with_capacity(elem_width);
                                for j in 0..elem_width {
                                    bits.push(lv.bits[i * elem_width + j]);
                                }
                                let elem = LogicVec {
                                    width: elem_width,
                                    bits,
                                };
                                if self.check_with_clause(with_clause, &elem)? {
                                    result_elems.push(elem);
                                }
                            }
                        } else {
                            for i in 0..count {
                                let mut bits = Vec::with_capacity(elem_width);
                                for j in 0..elem_width {
                                    bits.push(lv.bits[i * elem_width + j]);
                                }
                                let elem = LogicVec {
                                    width: elem_width,
                                    bits,
                                };
                                if self.check_with_clause(with_clause, &elem)? {
                                    result_elems.push(elem);
                                    if method == "find_first" {
                                        break;
                                    }
                                }
                            }
                        }
                        let total_width = result_elems.len() * elem_width;
                        let mut all_bits = Vec::with_capacity(total_width);
                        for e in &result_elems {
                            all_bits.extend(e.bits.iter());
                        }
                        return Ok(LogicVec {
                            width: total_width,
                            bits: all_bits,
                        });
                    }
                    if method == "find_first" && count > 0 {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[j]);
                        }
                        return Ok(LogicVec {
                            width: elem_width,
                            bits,
                        });
                    }
                    if method == "find_last" && count > 0 {
                        let start = (count - 1) * elem_width;
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[start + j]);
                        }
                        return Ok(LogicVec {
                            width: elem_width,
                            bits,
                        });
                    }
                    // "find" returns all elements (same as array)
                    return Ok(lv);
                }
                Ok(LogicVec::new(0))
            }
            "find_index" | "find_first_index" | "find_last_index" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    if with_clause.is_some() {
                        let mut indices: Vec<u64> = Vec::new();
                        if method == "find_last_index" {
                            for i in (0..count).rev() {
                                let mut bits = Vec::with_capacity(elem_width);
                                for j in 0..elem_width {
                                    bits.push(lv.bits[i * elem_width + j]);
                                }
                                let elem = LogicVec {
                                    width: elem_width,
                                    bits,
                                };
                                if self.check_with_clause(with_clause, &elem)? {
                                    indices.push(i as u64);
                                }
                            }
                        } else {
                            for i in 0..count {
                                let mut bits = Vec::with_capacity(elem_width);
                                for j in 0..elem_width {
                                    bits.push(lv.bits[i * elem_width + j]);
                                }
                                let elem = LogicVec {
                                    width: elem_width,
                                    bits,
                                };
                                if self.check_with_clause(with_clause, &elem)? {
                                    indices.push(i as u64);
                                    if method == "find_first_index" {
                                        break;
                                    }
                                }
                            }
                        }
                        let mut bits = Vec::new();
                        for idx in &indices {
                            let idx_vec = LogicVec::from_u64(*idx, 32);
                            bits.extend(idx_vec.bits.iter());
                        }
                        return Ok(LogicVec {
                            width: bits.len(),
                            bits,
                        });
                    }
                    // Return indices as 32-bit values packed into result
                    if method == "find_first_index" && count > 0 {
                        return Ok(LogicVec::from_u64(0, 32));
                    }
                    if method == "find_last_index" && count > 0 {
                        return Ok(LogicVec::from_u64((count - 1) as u64, 32));
                    }
                    // "find_index" returns all indices (0..count) as a packed queue
                    let mut bits = Vec::new();
                    for i in 0..count {
                        let idx_vec = LogicVec::from_u64(i as u64, 32);
                        bits.extend(idx_vec.bits.iter());
                    }
                    return Ok(LogicVec {
                        width: bits.len(),
                        bits,
                    });
                }
                Ok(LogicVec::new(0))
            }
            _ => Err(SimError::runtime(format!(
                "unknown array/queue method: {}",
                method
            ))),
        }
    }

    /// Evaluate sequence expressions recursively at a given cycle offset.
    /// Note: uses CURRENT signal state only — past values are not tracked.
    /// This gives simplified semantics where all Expr evaluations happen at the current time,
    /// but the cycle offset controls structural matching (delays, concatenations).
    fn eval_sequence_at_cycle(&mut self, seq: &IrSequence, cycles: u64) -> Result<bool, SimError> {
        match seq {
            IrSequence::Expr(expr) => {
                if cycles == 0 {
                    let val = self.evaluate_expr(expr)?;
                    Ok(val.to_bool() == Some(true))
                } else {
                    Ok(false)
                }
            }
            IrSequence::Delay(n) => Ok(cycles == *n),
            IrSequence::DelayRange(min, max) => Ok(cycles >= *min && cycles <= *max),
            IrSequence::Concat(a, b) => {
                // a must match at cycle k, b must match at cycles-k-1
                // Total: k + 1 + (cycles-k-1) = cycles
                for k in 0..cycles {
                    if self.eval_sequence_at_cycle(a, k)?
                        && self.eval_sequence_at_cycle(b, cycles - k - 1)?
                    {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            IrSequence::Or(a, b) => Ok(self.eval_sequence_at_cycle(a, cycles)?
                || self.eval_sequence_at_cycle(b, cycles)?),
            IrSequence::And(a, b) => Ok(self.eval_sequence_at_cycle(a, cycles)?
                && self.eval_sequence_at_cycle(b, cycles)?),
            IrSequence::Repeat(seq, n) => {
                if *n == 0 {
                    return Ok(true);
                }
                if *n == 1 {
                    return self.eval_sequence_at_cycle(seq, cycles);
                }
                for k in 0..=cycles {
                    if self.eval_sequence_at_cycle(seq, k)? {
                        let remaining = IrSequence::Repeat(Box::new((**seq).clone()), n - 1);
                        if self.eval_sequence_at_cycle(&remaining, cycles - k)? {
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            }
        }
    }

    /// Advance all active sequence attempts and evaluate them.
    /// Removes completed (matched or expired) attempts and executes pass/fail statements.
    fn evaluate_sequence_attempts(&mut self) -> Result<(), SimError> {
        // Pre-compute firing events (immutable borrow of self)
        let firing_events: Vec<bool> = self
            .sequence_attempts
            .iter()
            .map(|a| self.check_concurrent_clock_event(&a.clock_event))
            .collect();

        // Pre-clone sequences to avoid borrow conflicts during iteration
        let seqs: Vec<(Box<IrSequence>, u64)> = self
            .sequence_attempts
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx < firing_events.len() && firing_events[*idx])
            .map(|(_, a)| (a.sequence.clone(), a.cycles))
            .collect();

        // Evaluate all sequences (mutable borrow of self)
        let mut results: Vec<bool> = Vec::new();
        for (seq, cycles) in &seqs {
            results.push(self.eval_sequence_at_cycle(seq, *cycles)?);
        }

        // Update attempt states and mark completions
        let mut completed = Vec::new();
        let mut result_idx = 0;
        for (idx, attempt) in self.sequence_attempts.iter_mut().enumerate() {
            if idx < firing_events.len() && firing_events[idx] {
                let matched = if result_idx < results.len() {
                    results[result_idx]
                } else {
                    false
                };
                result_idx += 1;
                let max_cycles = attempt.sequence.max_cycles().unwrap_or(u64::MAX);
                if matched {
                    completed.push((idx, true));
                } else if attempt.cycles >= max_cycles {
                    completed.push((idx, false));
                }
                attempt.cycles += 1;
            }
        }

        // Process completed attempts (reverse order to preserve indices)
        for (idx, success) in completed.into_iter().rev() {
            if let Some(attempt) = self.sequence_attempts.get(idx) {
                let stmts = if success {
                    attempt.pass_stmt.clone()
                } else {
                    attempt.fail_stmt.clone()
                };
                if !stmts.is_empty() {
                    self.evaluate_block_with_delay_fork(&stmts, None)?;
                }
            }
            self.sequence_attempts.remove(idx);
        }
        Ok(())
    }

    fn check_concurrent_clock_event(&self, ce: &crate::ast::types::ClockEvent) -> bool {
        let sig_name = match ce {
            crate::ast::types::ClockEvent::Posedge(s) => s,
            crate::ast::types::ClockEvent::Negedge(s) => s,
            crate::ast::types::ClockEvent::Edge(s) => s,
        };
        let sig_id = match self.find_signal(sig_name) {
            Some(id) => id,
            None => return true,
        };
        let curr = self.state.read_signal(sig_id);
        match ce {
            crate::ast::types::ClockEvent::Posedge(_) => {
                if let Some(ref snap) = self.signal_snapshot {
                    let old = snap
                        .get(sig_id)
                        .cloned()
                        .unwrap_or_else(|| LogicVec::new(1));
                    old.to_bool() != Some(true) && curr.to_bool() == Some(true)
                } else {
                    curr.to_bool() == Some(true)
                }
            }
            crate::ast::types::ClockEvent::Negedge(_) => {
                if let Some(ref snap) = self.signal_snapshot {
                    let old = snap
                        .get(sig_id)
                        .cloned()
                        .unwrap_or_else(|| LogicVec::new(1));
                    old.to_bool() != Some(false) && curr.to_bool() == Some(false)
                } else {
                    curr.to_bool() == Some(false)
                }
            }
            crate::ast::types::ClockEvent::Edge(_) => {
                if let Some(ref snap) = self.signal_snapshot {
                    let old = snap
                        .get(sig_id)
                        .cloned()
                        .unwrap_or_else(|| LogicVec::new(1));
                    old.to_bool() != curr.to_bool()
                } else {
                    true
                }
            }
        }
    }
}
