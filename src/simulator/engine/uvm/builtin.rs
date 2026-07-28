use super::super::SimulationEngine;
use crate::diagnostics::DiagCode;
use crate::error::SimError;
use crate::ir::*;
use crate::Symbol;
use crate::simulator::types::*;
use crate::simulator::util::*;

impl SimulationEngine {
    pub(crate) fn execute_mailbox_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "new" => Ok(LogicVec::from_u64(1, 1)),
            "put" => {
                if args.is_empty() {
                    return Err(self.diag_error(DiagCode::DpiError, "mailbox::put expects 1 argument"));
                }
                self.mailbox_queues
                    .entry(obj_id)
                    .or_default()
                    .push_back(args[0].clone());
                Ok(LogicVec::from_u64(1, 1))
            }
            "get" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "mailbox not initialized");
                let q = self
                    .mailbox_queues
                    .get_mut(&obj_id)
                    .ok_or_else(|| err.clone())?;
                if q.is_empty() {
                    return Ok(LogicVec::default());
                }
                Ok(q.remove(0).unwrap_or(LogicVec::new(1)))
            }
            "try_get" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "mailbox not initialized");
                let q = self
                    .mailbox_queues
                    .get_mut(&obj_id)
                    .ok_or_else(|| err.clone())?;
                if q.is_empty() {
                    return Ok(LogicVec::from_u64(0, 1));
                }
                let _ = q.remove(0);
                Ok(LogicVec::from_u64(1, 1))
            }
            "try_put" => {
                if args.is_empty() {
                    return Err(self.diag_error(DiagCode::DpiError, "mailbox::try_put expects 1 argument"));
                }
                self.mailbox_queues
                    .entry(obj_id)
                    .or_default()
                    .push_back(args[0].clone());
                Ok(LogicVec::from_u64(1, 1))
            }
            "num" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "mailbox not initialized");
                let q = self
                    .mailbox_queues
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                Ok(LogicVec::from_u64(q.len() as u64, 32))
            }
            _ => Err(self.diag_error(DiagCode::NotImplemented, format!(
                "unknown mailbox method: {}",
                method
            ))),
        }
    }

    pub(crate) fn execute_semaphore_method(
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
                let err = SimError::with_diag(DiagCode::NullHandle, "semaphore not initialized");
                let c = self
                    .semaphore_counts
                    .get_mut(&obj_id)
                    .ok_or_else(|| err.clone())?;
                if *c < key_count {
                    return Err(self.diag_error(DiagCode::MemoryOutOfBounds, "semaphore::get: insufficient keys"));
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
                let err = SimError::with_diag(DiagCode::NullHandle, "semaphore not initialized");
                let c = self
                    .semaphore_counts
                    .get_mut(&obj_id)
                    .ok_or_else(|| err.clone())?;
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
                    .ok_or_else(|| SimError::with_diag(DiagCode::NullHandle, "semaphore not initialized"))?;
                if *c >= key_count {
                    *c -= key_count;
                    Ok(LogicVec::from_u64(1, 1))
                } else {
                    Ok(LogicVec::from_u64(0, 1))
                }
            }
            _ => Err(self.diag_error(DiagCode::NotImplemented, format!(
                "unknown semaphore method: {}",
                method
            ))),
        }
    }

    pub(crate) fn execute_process_method(
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
            _ => Err(self.diag_error(DiagCode::NotImplemented, format!(
                "unknown process method: {}",
                method
            ))),
        }
    }

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
                let comp_type = args.get(1).map(|a| logicvec_to_string(a)).unwrap_or_default();
                let cb_name = args.get(2).map(|a| logicvec_to_string(a)).unwrap_or_default();
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
                let comp_type = args.get(1).map(|a| logicvec_to_string(a)).unwrap_or_default();
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
                    for (cb_id, cb_name) in &data.callbacks {
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
            current = self.design.classes.get::<str>(&ct)
                .and_then(|c| c.extends.clone())
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
                    .map(|o| o.class_name.clone())
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

    pub(crate) fn execute_uvm_component_method(
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

    pub(crate) fn execute_uvm_sequence_item_method(
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
                Ok(string_to_logicvec(class_name.as_str()))
            }
            _ => self.execute_uvm_object_method(obj_id, method, args),
        }
    }

    pub(crate) fn execute_uvm_sequence_method(
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
                        Symbol::intern("__sequencer"),
                        LogicVec::from_u64(seqr_id as u64, 64),
                    );
                }
                // Call body()
                if self
                    .find_method_in_hierarchy(
                        &{
                            self.state
                                .get_object(obj_id)
                                .map(|o| o.class_name.to_string())
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
                let child = self.state.alloc_object(Symbol::intern(class_name.as_str()));
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

    pub(crate) fn execute_uvm_sequencer_method(
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
                    .ok_or_else(|| SimError::with_diag(DiagCode::NullHandle, "sequencer not initialized"))?;
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

    pub(crate) fn execute_uvm_driver_method(
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
                    .ok_or_else(|| SimError::with_diag(DiagCode::NullHandle, "driver not initialized"))?;
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
                    .ok_or_else(|| self.diag_error(DiagCode::NullHandle, "driver not initialized"))?;
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

    pub(crate) fn execute_uvm_monitor_method(
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

    pub(crate) fn execute_uvm_analysis_port_method(
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

    pub(crate) fn execute_uvm_analysis_imp_method(
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
                        .map(|o| o.class_name.to_string())
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

    pub(crate) fn execute_super_method(
        &mut self,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        let obj_id = self
            .current_this
            .ok_or_else(|| SimError::with_diag(DiagCode::NullHandle, "'super' used outside class method"))?;
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
                self.diag_error(DiagCode::DpiError, format!(
                    "class '{}' has no parent for super call",
                    class_name
                ))
            })?;
        // Check hierarchy from most specific to least
        if parent == "__uvm_driver" || self.is_uvm_driver_hierarchy(&parent.as_str()) {
            return self.execute_uvm_driver_method(obj_id, method, args);
        }
        if parent == "__uvm_monitor" || self.is_uvm_monitor_hierarchy(&parent.as_str()) {
            return self.execute_uvm_monitor_method(obj_id, method, args);
        }
        if parent == "__uvm_sequencer" || self.is_uvm_sequencer_hierarchy(&parent.as_str()) {
            return self.execute_uvm_sequencer_method(obj_id, method, args);
        }
        if parent == "__uvm_sequence" || self.is_uvm_sequence_hierarchy(&parent.as_str()) {
            return self.execute_uvm_sequence_method(obj_id, method, args);
        }
        if parent == "__uvm_sequence_item" || self.is_uvm_sequence_item_hierarchy(&parent.as_str()) {
            return self.execute_uvm_sequence_item_method(obj_id, method, args);
        }
        if parent == "__uvm_analysis_port" || self.is_uvm_analysis_port_hierarchy(&parent.as_str()) {
            return self.execute_uvm_analysis_port_method(obj_id, method, args);
        }
        if parent == "__uvm_analysis_imp" || self.is_uvm_analysis_imp_hierarchy(&parent.as_str()) {
            return self.execute_uvm_analysis_imp_method(obj_id, method, args);
        }
        // Check if parent is uvm_component hierarchy
        if parent == "__uvm_component" || self.is_uvm_component_hierarchy(&parent.as_str()) {
            return self.execute_uvm_component_method(obj_id, method, args);
        }
        // Check if parent is uvm_report_object hierarchy
        if parent == "__uvm_report_object" || self.is_uvm_report_object_hierarchy(&parent.as_str()) {
            return self.execute_uvm_report_object_method(obj_id, method, args);
        }
        // Check if parent is uvm_object hierarchy
        if parent == "__uvm_object" || self.is_uvm_object_hierarchy(&parent.as_str()) {
            return self.execute_uvm_object_method(obj_id, method, args);
        }
        // Super dispatch: start search from parent class, skipping current class override
        let method_def = self.find_method_in_hierarchy(parent.as_str(), method)?.clone();
        self.execute_method_body(Some(obj_id), &method_def, args, method)
    }

}
