//! Layer objek dasar UVM — uvm_object, uvm_report_object, dan callback
//! (F16/F20). Method builtin untuk class builtin paling dasar: `new`,
//! `get_name`/`set_name`, `print`, objection, dan `uvm_report_*` yang
//! disambungkan ke severity system.
//! 1 file = 1 tanggung jawab: hanya layer objek/report/callback — komponen,
//! sequence, dan register model tinggal di component.rs / reg.rs.

use super::super::SimulationEngine;
use maria_core::diagnostics::DiagCode;
use maria_core::error::SimError;
use maria_core::Symbol;
use maria_ir::*;
use crate::simulator::types::*;
use crate::simulator::util::*;

impl SimulationEngine {
    pub(crate) fn execute_uvm_callback_method(
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
                self.uvm_object_data.insert(obj_id, UvmObjectData {
                    name: name.clone(),
                });
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_object_method(obj_id, method, args),
        }
    }

    /// Execute uvm_callbacks#add: register a callback on a component type.
    pub(crate) fn execute_uvm_callbacks_add(
        &mut self,
        _obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "add" => {
                // uvm_callbacks#add(cb_obj, comp_type)
                let cb_obj_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let comp_type = args.get(1).map(logicvec_to_string).unwrap_or_default();
                let cb_name = args.get(2).map(logicvec_to_string).unwrap_or_default();
                // Store by (component_type, cb_type_name)
                let cb_type = if let Some(obj) = self.state.get_object(cb_obj_id) {
                    obj.class_name.to_string()
                } else {
                    String::new()
                };
                let queue_key = (comp_type, cb_type.clone());
                let entry = self.callback_queues.entry(queue_key).or_insert_with(|| {
                    crate::simulator::types::UvmCallbackData {
                        cb_type_name: cb_type,
                        callbacks: Vec::new(),
                        enabled: true,
                    }
                });
                entry.callbacks.push((cb_obj_id, cb_name));
                Ok(LogicVec::from_u64(1, 1))
            }
            "delete" => {
                let cb_obj_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let comp_type = args.get(1).map(logicvec_to_string).unwrap_or_default();
                let cb_type = if let Some(obj) = self.state.get_object(cb_obj_id) {
                    obj.class_name.to_string()
                } else {
                    String::new()
                };
                let queue_key = (comp_type, cb_type);
                if let Some(entry) = self.callback_queues.get_mut(&queue_key) {
                    entry.callbacks.retain(|(id, _)| *id != cb_obj_id);
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "display" => {
                for ((comp_type, cb_type), data) in &self.callback_queues {
                    println!(
                        "UVM_CALLBACK: {} registered on {} ({} callbacks, enabled={})",
                        cb_type,
                        comp_type,
                        data.callbacks.len(),
                        data.enabled
                    );
                    for (cb_id, _cb_name) in &data.callbacks {
                        let name = self
                            .uvm_object_data
                            .get(cb_id)
                            .map(|d| d.name.as_str())
                            .unwrap_or("unnamed");
                        println!("  - {} (obj_id={})", name, cb_id);
                    }
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => Ok(LogicVec::from_u64(0, 1)),
        }
    }

    /// Invoke callbacks for a specific component type before/after method execution.
    pub(crate) fn invoke_callbacks(
        &mut self,
        comp_type: &str,
        callback_method: &str,
        args: &[LogicVec],
    ) -> Result<(), SimError> {
        // Check per-component-type, then per-parent-type (UVM callback inheritance)
        let mut visited = std::collections::HashSet::new();
        let mut current = Some(comp_type.to_string());
        while let Some(ct) = current {
            if visited.contains(&ct) {
                break;
            }
            visited.insert(ct.clone());

            // Check all callback queue entries for this component type
            let keys: Vec<(String, String)> = self.callback_queues.keys()
                .filter(|(ct_key, _)| ct_key == &ct)
                .cloned()
                .collect();

            for key in &keys {
                if let Some(data) = self.callback_queues.get(key) {
                    if !data.enabled {
                        continue;
                    }
                    // Invoke callback_method on each registered callback object
                    let cbs = data.callbacks.clone();
                    for (cb_id, _) in &cbs {
                        if self.find_method_in_hierarchy(
                            &{
                                self.state.get_object(*cb_id)
                                    .map(|o| o.class_name.to_string())
                                    .unwrap_or_default()
                            },
                            callback_method,
                        ).is_ok() {
                            self.execute_method(*cb_id, callback_method, args)?;
                        }
                    }
                }
            }

            // Walk up component hierarchy for inherited callback registrations
            current = self.design.classes.get(&Symbol::intern(&ct))
                .and_then(|c| c.extends)
                .map(|s| s.to_string());
        }
        Ok(())
    }

    pub(crate) fn execute_uvm_object_method(
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
                    .ok_or_else(|| SimError::with_diag(DiagCode::NullHandle, "uvm_object not initialized"))?;
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
                    .map(|o| o.class_name)
                    .unwrap_or_default();
                Ok(string_to_logicvec(class_name.as_str()))
            }
            "print" => {
                let data = self
                    .uvm_object_data
                    .get(&obj_id)
                    .ok_or_else(|| SimError::with_diag(DiagCode::NullHandle, "uvm_object not initialized"))?;
                let class_name = self
                    .state
                    .get_object(obj_id)
                    .map(|o| o.class_name)
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
                    // F18: jalankan fase akhir (extract/check/report/final)
                    // pada tree root test SEBELUM menghentikan sim.
                    self.execute_report_phases()?;
                    self.running = false;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => Err(self.diag_error(DiagCode::NotImplemented, format!(
                "uvm_object::{} not implemented",
                method
            ))),
        }
    }

    pub(crate) fn execute_uvm_report_object_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        // F16: uvm_report_* disambungkan ke severity system (F14/F15) —
        // emit_severity mencetak dengan prefix severity, increment counter
        // (ringkasan akhir sim), dan untuk fatal set fatal_hit + running
        // (hentikan sim seketika + exit code CLI non-zero).
        match method {
            "uvm_report_info" => {
                let id = args
                    .first()
                    .map(logicvec_to_string)
                    .unwrap_or_default();
                let msg = args
                    .get(1)
                    .map(logicvec_to_string)
                    .unwrap_or_default();
                self.emit_severity("info", &format!("@ {}: {} [{}]", self.current_time, msg, id));
                Ok(LogicVec::from_u64(1, 1))
            }
            "uvm_report_warning" => {
                let id = args
                    .first()
                    .map(logicvec_to_string)
                    .unwrap_or_default();
                let msg = args
                    .get(1)
                    .map(logicvec_to_string)
                    .unwrap_or_default();
                self.emit_severity("warning", &format!("@ {}: {} [{}]", self.current_time, msg, id));
                Ok(LogicVec::from_u64(1, 1))
            }
            "uvm_report_error" => {
                let id = args
                    .first()
                    .map(logicvec_to_string)
                    .unwrap_or_default();
                let msg = args
                    .get(1)
                    .map(logicvec_to_string)
                    .unwrap_or_default();
                self.emit_severity("error", &format!("@ {}: {} [{}]", self.current_time, msg, id));
                Ok(LogicVec::from_u64(1, 1))
            }
            "uvm_report_fatal" => {
                let id = args
                    .first()
                    .map(logicvec_to_string)
                    .unwrap_or_default();
                let msg = args
                    .get(1)
                    .map(logicvec_to_string)
                    .unwrap_or_default();
                self.emit_severity("fatal", &format!("@ {}: {} [{}]", self.current_time, msg, id));
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_object_method(obj_id, method, args),
        }
    }
}
