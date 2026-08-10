use super::super::SimulationEngine;
use maria_core::diagnostics::DiagCode;
use maria_core::error::SimError;
use maria_ir::*;
use maria_core::Symbol;
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
            "new" => {
                // LANG-24: `new(bound)` — simpan batas kapasitas (bounded mode).
                if let Some(bound) = args.first() {
                    let b = bound.to_u64() as usize;
                    if b > 0 {
                        self.mailbox_bounds.insert(obj_id, b);
                    }
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "put" => {
                if args.is_empty() {
                    return Err(self.diag_error(DiagCode::DpiError, "mailbox::put expects 1 argument"));
                }
                let bound = self.mailbox_bounds.get(&obj_id).copied().unwrap_or(0);
                let len = self.mailbox_queues.get(&obj_id).map(|q| q.len()).unwrap_or(0);
                if bound > 0 && len >= bound {
                    // Bounded + penuh di konteks ekspresi (non-blocking): warning,
                    // item dibuang (semantics try_put). Jalur statement blocking
                    // (block.rs) men-suspend putter — lihat uvm_try_mailbox_wait.
                    self.emit_warning(
                        DiagCode::NullHandle,
                        format!(
                            "mailbox #({}) full; non-blocking put dropped item (obj_id={})",
                            bound, obj_id
                        ),
                    );
                    return Ok(LogicVec::from_u64(0, 1));
                }
                self.mailbox_queues
                    .entry(obj_id)
                    .or_default()
                    .push_back(args[0].clone());
                self.uvm_mailbox_release_waiters(obj_id, false)?;
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
                let v = q.remove(0).unwrap_or(LogicVec::new(1));
                self.uvm_mailbox_release_waiters(obj_id, true)?;
                Ok(v)
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
                self.uvm_mailbox_release_waiters(obj_id, true)?;
                Ok(LogicVec::from_u64(1, 1))
            }
            "try_put" => {
                if args.is_empty() {
                    return Err(self.diag_error(DiagCode::DpiError, "mailbox::try_put expects 1 argument"));
                }
                let bound = self.mailbox_bounds.get(&obj_id).copied().unwrap_or(0);
                let len = self.mailbox_queues.get(&obj_id).map(|q| q.len()).unwrap_or(0);
                if bound > 0 && len >= bound {
                    return Ok(LogicVec::from_u64(0, 1));
                }
                self.mailbox_queues
                    .entry(obj_id)
                    .or_default()
                    .push_back(args[0].clone());
                self.uvm_mailbox_release_waiters(obj_id, false)?;
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
            "size" => {
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

    /// Blocking wait mailbox (dipanggil block.rs `Stmt::Expr`):
    /// - "get": kosong → daftar waiter, suspend (Ok(true)); ada → Ok(false).
    /// - "put": bounded + penuh → daftar waiter, suspend; ada ruang → Ok(false).
    /// Return Ok(true) = harus suspend. (LANG-24)
    pub(crate) fn uvm_try_mailbox_wait(
        &mut self,
        obj_id: ObjId,
        method: &str,
        continuation: Vec<maria_ast::Stmt>,
        fork_id: Option<usize>,
        this: Option<ObjId>,
        method_opt: Option<Symbol>,
    ) -> Result<bool, SimError> {
        match method {
            "get" => {
                let empty = self
                    .mailbox_queues
                    .get(&obj_id)
                    .map(|q| q.is_empty())
                    .unwrap_or(true);
                if !empty {
                    return Ok(false);
                }
            }
            "put" => {
                let bound = self.mailbox_bounds.get(&obj_id).copied().unwrap_or(0);
                if bound == 0 {
                    // Unbounded — tidak pernah penuh.
                    return Ok(false);
                }
                let len = self
                    .mailbox_queues
                    .get(&obj_id)
                    .map(|q| q.len())
                    .unwrap_or(0);
                if len < bound {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
        self.uvm_sync_waiters.entry(obj_id).or_default().push(UvmSyncWaiter {
            continuation,
            fork_id,
            this,
            method: method_opt,
            wait_label: method.to_string(),
        });
        Ok(true)
    }

    /// Resume waiter mailbox yang match: `getters=true` → "get";
    /// `getters=false` → "put". Menjadwalkan ContinueAstBlock t+1. (LANG-24)
    pub(crate) fn uvm_mailbox_release_waiters(
        &mut self,
        obj_id: ObjId,
        getters: bool,
    ) -> Result<(), SimError> {
        let all = self.uvm_sync_waiters.remove(&obj_id).unwrap_or_default();
        let (matched, rest): (Vec<_>, Vec<_>) = all.into_iter().partition(|w| {
            if getters {
                w.wait_label == "get"
            } else {
                w.wait_label == "put"
            }
        });
        if !rest.is_empty() {
            self.uvm_sync_waiters.insert(obj_id, rest);
        }
        let t = self.state.time as usize + 1;
        self.ensure_events(t);
        for w in matched {
            self.push_event(
                t,
                RegionEvent {
                    region: EventRegion::Active,
                    event: EventKind::ContinueAstBlock(
                        w.continuation,
                        w.fork_id,
                        w.this,
                        w.method,
                    ),
                },
            );
        }
        Ok(())
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
                    .map(logicvec_to_string)
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
                    .map(|o| o.class_name)
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
                // Store sequencer obj_id on the sequence object's fields.
                // `m_sequencer` adalah nama standar UVM (dipakai macro
                // `uvm_create` untuk `set_sequencer(m_sequencer)`) — tanpa ini,
                // `m_sequencer != null` gagal resolve → warning RT0001.
                if let Some(obj) = self.state.get_object_mut(obj_id) {
                    let v = LogicVec::from_u64(seqr_id as u64, 64);
                    obj.fields.insert(Symbol::intern("__sequencer"), v.clone());
                    obj.fields.insert(Symbol::intern("m_sequencer"), v);
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
                    // F24: release waiter getter (driver get_next_item yang
                    // block saat queue kosong) — item baru tersedia.
                    self.uvm_seq_release_getters(seqr_id)?;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "finish_item" => Ok(LogicVec::from_u64(1, 1)),
            "set_sequencer" => {
                // UVM: `set_sequencer(m_sequencer)` dipanggil oleh macro
                // `uvm_create`. Simpan sequencer pada field sequence agar
                // `m_sequencer`/`__sequencer` resolve (dan start_item tahu
                // target queue).
                let seqr_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                if let Some(obj) = self.state.get_object_mut(obj_id) {
                    let v = LogicVec::from_u64(seqr_id as u64, 64);
                    obj.fields.insert(Symbol::intern("__sequencer"), v.clone());
                    obj.fields.insert(Symbol::intern("m_sequencer"), v);
                }
                Ok(LogicVec::from_u64(1, 1))
            }
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
                    .map(logicvec_to_string)
                    .unwrap_or_default();
                // Create a new object of the sequence's type
                let class_name = self
                    .state
                    .get_object(obj_id)
                    .map(|o| o.class_name)
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
                                .entry(field.name)
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
                // F24: release item current + waiter finish_item:{item}. Jangan
                // double-pop: block.rs proceed path (get_next_item) sudah
                // meng-pop item dari queue. Hanya pop bila item MASIH terdepan
                // di queue (pola lama F17: builtin get_next_item tanpa pop).
                let done_item = self
                    .uvm_sequencer_data
                    .get_mut(&obj_id)
                    .and_then(|sd| {
                        let item = sd.current_item.take();
                        if let Some(it) = item {
                            if sd.item_queue.first() == Some(&it) {
                                sd.item_queue.remove(0);
                            }
                        }
                        item
                    });
                if let Some(item_id) = done_item {
                    self.uvm_seq_release_finishers(obj_id, item_id)?;
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

    // ─── uvm_subscriber (F22) ───────────────────────────────────────────
    // Komponen penerima broadcast analysis port. `new` membangun analysis_imp
    // child secara otomatis (class `__uvm_analysis_imp`, parent = subscriber)
    // dan menyimpannya di field `analysis_imp` — pola UVM asli
    // `uvm_subscriber` punya `uvm_analysis_imp #(T) analysis_imp` internal.
    // User `class my_sub extends uvm_subscriber` cukup meng-override `write()`;
    // `aport.connect(my_sub.analysis_imp)` → `aport.write(item)` → imp
    // (execute_uvm_analysis_imp_method) → parent.write (override user).
    pub(crate) fn execute_uvm_subscriber_method(
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
                    .or_insert_with(|| UvmObjectData { name: name.clone() });
                // Analysis-imp internal: alokasi objek `__uvm_analysis_imp`
                // dengan parent = subscriber, simpan id di field analysis_imp.
                let imp_name = format!("{}_imp", if name.is_empty() { "sub" } else { &name });
                let imp_id = self.state.alloc_object(Symbol::intern("__uvm_analysis_imp"));
                self.uvm_analysis_imp_data.insert(
                    imp_id,
                    UvmAnalysisImpData {
                        parent: Some(obj_id),
                        name: imp_name.clone(),
                    },
                );
                self.uvm_object_data
                    .entry(imp_id)
                    .or_insert_with(|| UvmObjectData { name: imp_name });
                if let Some(obj) = self.state.get_object_mut(obj_id) {
                    obj.fields.insert(
                        Symbol::intern("analysis_imp"),
                        LogicVec::from_u64(imp_id as u64, 64),
                    );
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            // `write` tanpa override user: no-op (broadcast tetap diterima,
            // tidak error). Override user dijalankan via jalur normal method.rs
            // (find_method_in_hierarchy menemukan write di class user).
            "write" => Ok(LogicVec::from_u64(1, 1)),
            _ => self.execute_uvm_component_method(obj_id, method, args),
        }
    }

    // ─── UVM Reg Layer Methods ──────────────────────────────────────────

    pub(crate) fn execute_uvm_reg_field_method(
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
                let parent_reg = args.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                self.uvm_object_data
                    .entry(obj_id)
                    .or_insert_with(|| UvmObjectData { name });
                let mut fd = UvmRegFieldData::new();
                if parent_reg != 0 {
                    fd.parent_reg = Some(parent_reg);
                    // Register this field with the parent register
                    if let Some(rd) = self.uvm_reg_data.get_mut(&parent_reg) {
                        rd.fields.push(obj_id);
                    }
                }
                self.uvm_reg_field_data.insert(obj_id, fd);
                Ok(LogicVec::from_u64(1, 1))
            }
            "set_access" => {
                let access = args.first().map(logicvec_to_string).unwrap_or_default();
                if let Some(fd) = self.uvm_reg_field_data.get_mut(&obj_id) {
                    fd.access = access;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "set" => {
                let val = args.first().cloned().unwrap_or(LogicVec::new(1));
                if let Some(fd) = self.uvm_reg_field_data.get_mut(&obj_id) {
                    fd.value = val.clone();
                    fd.desired = val;
                    fd.modified = true;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get" => {
                let val = self.uvm_reg_field_data.get(&obj_id)
                    .map(|fd| fd.value.clone())
                    .unwrap_or(LogicVec::new(1));
                Ok(val)
            }
            "get_desired" => {
                let val = self.uvm_reg_field_data.get(&obj_id)
                    .map(|fd| fd.desired.clone())
                    .unwrap_or(LogicVec::new(1));
                Ok(val)
            }
            "set_desired" => {
                let val = args.first().cloned().unwrap_or(LogicVec::new(1));
                if let Some(fd) = self.uvm_reg_field_data.get_mut(&obj_id) {
                    fd.desired = val;
                    fd.modified = true;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "randomize" => {
                if let Some(fd) = self.uvm_reg_field_data.get(&obj_id) {
                    let class_name = self.state.get_object(obj_id)
                        .map(|o| o.class_name.to_string())
                        .unwrap_or_default();
                    if !class_name.is_empty() {
                        return self.execute_randomize(obj_id, class_name.as_str());
                    }
                }
                // Fallback: randomize via engine
                let seed = self.current_time.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let width = self.uvm_reg_field_data.get(&obj_id)
                    .map(|fd| fd.width).unwrap_or(1);
                let rv = LogicVec::from_u64(seed, width);
                if let Some(fd) = self.uvm_reg_field_data.get_mut(&obj_id) {
                    fd.value = rv.clone();
                    fd.desired = rv.clone();
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "mirror" => {
                // Read from DUT via parent register (simplified: predict from current value)
                Ok(LogicVec::from_u64(1, 1))
            }
            "predict" => {
                let val = args.first().cloned().unwrap_or(LogicVec::new(1));
                if let Some(fd) = self.uvm_reg_field_data.get_mut(&obj_id) {
                    fd.value = val;
                    fd.modified = false;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "reset" => {
                if let Some(fd) = self.uvm_reg_field_data.get_mut(&obj_id) {
                    fd.value = LogicVec::new(fd.width.max(1));
                    fd.desired = LogicVec::new(fd.width.max(1));
                    fd.modified = false;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "set_bit_pos" => {
                let pos = args.first().map(|a| a.to_u64() as usize).unwrap_or(0);
                if let Some(fd) = self.uvm_reg_field_data.get_mut(&obj_id) {
                    fd.bit_pos = pos;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_bit_pos" => {
                let pos = self.uvm_reg_field_data.get(&obj_id)
                    .map(|fd| fd.bit_pos as u64)
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(pos, 32))
            }
            "get_n_bits" => {
                let w = self.uvm_reg_field_data.get(&obj_id)
                    .map(|fd| fd.width as u64)
                    .unwrap_or(1);
                Ok(LogicVec::from_u64(w, 32))
            }
            "get_access" => {
                let access = self.uvm_reg_field_data.get(&obj_id)
                    .map(|fd| fd.access.clone())
                    .unwrap_or_default();
                Ok(string_to_logicvec(&access))
            }
            "is_modified" => {
                let modified = self.uvm_reg_field_data.get(&obj_id)
                    .map(|fd| fd.modified)
                    .unwrap_or(false);
                Ok(LogicVec::from_u64(if modified { 1 } else { 0 }, 1))
            }
            _ => self.execute_uvm_object_method(obj_id, method, args),
        }
    }

    pub(crate) fn execute_uvm_reg_method(
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
                self.uvm_reg_data.entry(obj_id).or_insert_with(|| {
                    let mut rd = UvmRegData::new();
                    if args.len() > 1 {
                        rd.width = args[1].to_u64() as usize;
                    }
                    if args.len() > 2 {
                        rd.address = args[2].to_u64();
                    }
                    rd
                });
                Ok(LogicVec::from_u64(1, 1))
            }
            "configure" => {
                // configure(parent_block, regfile_path, offset)
                let block_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let offset = args.get(2).map(|a| a.to_u64()).unwrap_or(0);
                if block_id != 0 {
                    if let Some(rd) = self.uvm_reg_data.get_mut(&obj_id) {
                        rd.parent_block = Some(block_id);
                        if offset != 0 && args.len() > 2 {
                            rd.address = offset;
                        }
                    }
                    // Register with parent block (use local values, not borrowed references)
                    let reg_offset = self.uvm_reg_data.get(&obj_id)
                        .map(|rd| rd.address).unwrap_or(offset);
                    if let Some(bd) = self.uvm_reg_block_data.get_mut(&block_id) {
                        bd.regs_by_offset.insert(reg_offset, obj_id);
                        if let Some(map_id) = bd.default_map {
                            if let Some(md) = self.uvm_reg_map_data.get_mut(&map_id) {
                                md.regs_by_offset.insert(reg_offset, obj_id);
                            }
                        }
                    }
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "write" => {
                // write(status, value, map, path): model-side write (update desired + mirror)
                let val = args.get(1).cloned().unwrap_or(LogicVec::new(32));
                if let Some(rd) = self.uvm_reg_data.get_mut(&obj_id) {
                    rd.desired = val.clone();
                    rd.value = val;
                    rd.modified = true;
                }
                // Propagate to fields
                if let Some(rd) = self.uvm_reg_data.get(&obj_id) {
                    for fid in &rd.fields {
                        if let Some(fd) = self.uvm_reg_field_data.get_mut(fid) {
                            fd.modified = true;
                        }
                    }
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "read" => {
                // read(status, map, path): model-side read (return mirrored value)
                // Also returns status in first arg (simplified: always success)
                let val = self.uvm_reg_data.get(&obj_id)
                    .map(|rd| rd.value.clone())
                    .unwrap_or(LogicVec::new(32));
                Ok(val)
            }
            "set" => {
                let val = args.first().cloned().unwrap_or(LogicVec::new(32));
                if let Some(rd) = self.uvm_reg_data.get_mut(&obj_id) {
                    rd.desired = val.clone();
                    rd.modified = true;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get" => {
                let val = self.uvm_reg_data.get(&obj_id)
                    .map(|rd| rd.value.clone())
                    .unwrap_or(LogicVec::new(32));
                Ok(val)
            }
            "get_desired" => {
                let val = self.uvm_reg_data.get(&obj_id)
                    .map(|rd| rd.desired.clone())
                    .unwrap_or(LogicVec::new(32));
                Ok(val)
            }
            "update" => {
                // Write modified fields/registers to DUT (bus access)
                if let Some(rd) = self.uvm_reg_data.get_mut(&obj_id) {
                    if rd.modified {
                        rd.value = rd.desired.clone();
                        rd.modified = false;
                    }
                }
                // Reset field modified flags
                if let Some(rd) = self.uvm_reg_data.get(&obj_id) {
                    for fid in &rd.fields {
                        if let Some(fd) = self.uvm_reg_field_data.get_mut(fid) {
                            fd.modified = false;
                        }
                    }
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "mirror" => {
                // Read from DUT (simplified: keep current value)
                if let Some(rd) = self.uvm_reg_data.get_mut(&obj_id) {
                    rd.modified = false;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "randomize" => {
                let class_name = self.state.get_object(obj_id)
                    .map(|o| o.class_name.to_string())
                    .unwrap_or_default();
                if !class_name.is_empty() {
                    return self.execute_randomize(obj_id, class_name.as_str());
                }
                // Fallback: randomize each field
                if let Some(rd) = self.uvm_reg_data.get(&obj_id) {
                    let fields = rd.fields.clone();
                    for fid in &fields {
                        self.execute_uvm_reg_field_method(*fid, "randomize", &[])?;
                    }
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "reset" => {
                if let Some(rd) = self.uvm_reg_data.get_mut(&obj_id) {
                    rd.value = LogicVec::new(rd.width.max(1));
                    rd.desired = LogicVec::new(rd.width.max(1));
                    rd.modified = false;
                }
                // Reset all fields
                if let Some(rd) = self.uvm_reg_data.get(&obj_id) {
                    let fields = rd.fields.clone();
                    for fid in &fields {
                        self.execute_uvm_reg_field_method(*fid, "reset", &[])?;
                    }
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_fields" => {
                // Return list of field object IDs
                let fields = self.uvm_reg_data.get(&obj_id)
                    .map(|rd| rd.fields.clone())
                    .unwrap_or_default();
                // Pack field IDs into a single LogicVec (64-bit each)
                if fields.is_empty() {
                    Ok(LogicVec::new(0))
                } else {
                    let total_width = fields.len() * 64;
                    let mut bits = Vec::with_capacity(total_width);
                    for fid in &fields {
                        let id_vec = LogicVec::from_u64(*fid as u64, 64);
                        bits.extend(id_vec.bits.iter());
                    }
                    Ok(LogicVec { width: total_width, bits })
                }
            }
            "get_address" => {
                let addr = self.uvm_reg_data.get(&obj_id)
                    .map(|rd| rd.address)
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(addr, 64))
            }
            "get_n_bits" => {
                let w = self.uvm_reg_data.get(&obj_id)
                    .map(|rd| rd.width as u64)
                    .unwrap_or(32);
                Ok(LogicVec::from_u64(w, 32))
            }
            "is_modified" => {
                let modified = self.uvm_reg_data.get(&obj_id)
                    .map(|rd| rd.modified)
                    .unwrap_or(false);
                Ok(LogicVec::from_u64(if modified { 1 } else { 0 }, 1))
            }
            _ => self.execute_uvm_object_method(obj_id, method, args),
        }
    }

    pub(crate) fn execute_uvm_reg_block_method(
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
                self.uvm_reg_block_data
                    .entry(obj_id)
                    .or_insert_with(UvmRegBlockData::new);
                Ok(LogicVec::from_u64(1, 1))
            }
            "build" => {
                // Build registers: typically overridden by user class
                // Default: no-op, user's build() creates and configures registers
                Ok(LogicVec::from_u64(1, 1))
            }
            "default_map" => {
                // Get/set default address map
                let default_map = self.uvm_reg_block_data.get(&obj_id)
                    .and_then(|bd| bd.default_map)
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(default_map as u64, 64))
            }
            "set_default_map" => {
                let map_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                if let Some(bd) = self.uvm_reg_block_data.get_mut(&obj_id) {
                    bd.default_map = Some(map_id);
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_reg_by_offset" => {
                let offset = args.first().map(|a| a.to_u64()).unwrap_or(0);
                let reg_id = self.uvm_reg_block_data.get(&obj_id)
                    .and_then(|bd| bd.regs_by_offset.get(&offset).copied())
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(reg_id as u64, 64))
            }
            "get_registers" => {
                // Return all register object IDs in this block
                let regs: Vec<u64> = self.uvm_reg_block_data.get(&obj_id)
                    .map(|bd| bd.regs_by_offset.values().map(|&id| id as u64).collect())
                    .unwrap_or_default();
                if regs.is_empty() {
                    Ok(LogicVec::new(0))
                } else {
                    let total_width = regs.len() * 64;
                    let mut bits = Vec::with_capacity(total_width);
                    for &rid in &regs {
                        let id_vec = LogicVec::from_u64(rid, 64);
                        bits.extend(id_vec.bits.iter());
                    }
                    Ok(LogicVec { width: total_width, bits })
                }
            }
            "get_base_address" => {
                let addr = self.uvm_reg_block_data.get(&obj_id)
                    .map(|bd| bd.base_address)
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(addr, 64))
            }
            "set_base_address" => {
                let addr = args.first().map(|a| a.to_u64()).unwrap_or(0);
                if let Some(bd) = self.uvm_reg_block_data.get_mut(&obj_id) {
                    bd.base_address = addr;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_component_method(obj_id, method, args),
        }
    }

    pub(crate) fn execute_uvm_reg_map_method(
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
                self.uvm_reg_map_data
                    .entry(obj_id)
                    .or_insert_with(UvmRegMapData::new);
                Ok(LogicVec::from_u64(1, 1))
            }
            "add_reg" => {
                let reg_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let offset = args.get(1).map(|a| a.to_u64()).unwrap_or(0);
                if let Some(md) = self.uvm_reg_map_data.get_mut(&obj_id) {
                    md.regs_by_offset.insert(offset, reg_id);
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_reg_by_offset" => {
                let offset = args.first().map(|a| a.to_u64()).unwrap_or(0);
                let reg_id = self.uvm_reg_map_data.get(&obj_id)
                    .and_then(|md| md.regs_by_offset.get(&offset).copied())
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(reg_id as u64, 64))
            }
            "set_base_addr" => {
                let addr = args.first().map(|a| a.to_u64()).unwrap_or(0);
                if let Some(md) = self.uvm_reg_map_data.get_mut(&obj_id) {
                    md.base_address = addr;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_base_addr" => {
                let addr = self.uvm_reg_map_data.get(&obj_id)
                    .map(|md| md.base_address)
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(addr, 64))
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
            .map(|o| o.class_name)
            .unwrap_or_default();
        let parent = self
            .design
            .classes
            .get(&class_name)
            .and_then(|c| c.extends)
            .ok_or_else(|| {
                self.diag_error(DiagCode::DpiError, format!(
                    "class '{}' has no parent for super call",
                    class_name
                ))
            })?;
        // Check hierarchy from most specific to least
        // ── Reg layer (check before uvm_component since uvm_reg_block extends uvm_component) ──
        if parent == "__uvm_reg_block" || self.is_uvm_reg_block_hierarchy(parent.as_str()) {
            return self.execute_uvm_reg_block_method(obj_id, method, args);
        }
        if parent == "__uvm_reg_map" || self.is_uvm_reg_map_hierarchy(parent.as_str()) {
            return self.execute_uvm_reg_map_method(obj_id, method, args);
        }
        if parent == "__uvm_reg" || self.is_uvm_reg_hierarchy(parent.as_str()) {
            return self.execute_uvm_reg_method(obj_id, method, args);
        }
        if parent == "__uvm_reg_field" || self.is_uvm_reg_field_hierarchy(parent.as_str()) {
            return self.execute_uvm_reg_field_method(obj_id, method, args);
        }
        if parent == "__uvm_driver" || self.is_uvm_driver_hierarchy(parent.as_str()) {
            return self.execute_uvm_driver_method(obj_id, method, args);
        }
        if parent == "__uvm_monitor" || self.is_uvm_monitor_hierarchy(parent.as_str()) {
            return self.execute_uvm_monitor_method(obj_id, method, args);
        }
        if parent == "__uvm_sequencer" || self.is_uvm_sequencer_hierarchy(parent.as_str()) {
            return self.execute_uvm_sequencer_method(obj_id, method, args);
        }
        if parent == "__uvm_sequence" || self.is_uvm_sequence_hierarchy(parent.as_str()) {
            return self.execute_uvm_sequence_method(obj_id, method, args);
        }
        if parent == "__uvm_sequence_item" || self.is_uvm_sequence_item_hierarchy(parent.as_str()) {
            return self.execute_uvm_sequence_item_method(obj_id, method, args);
        }
        if parent == "__uvm_analysis_port" || self.is_uvm_analysis_port_hierarchy(parent.as_str()) {
            return self.execute_uvm_analysis_port_method(obj_id, method, args);
        }
        if parent == "__uvm_analysis_imp" || self.is_uvm_analysis_imp_hierarchy(parent.as_str()) {
            return self.execute_uvm_analysis_imp_method(obj_id, method, args);
        }
        // F21: `super.new(name)` di subclass user (`my_event extends uvm_event`)
        // — data event/barrier di-insert di sini (tanpa arm ini jatuh ke
        // execute_uvm_object_method yang hanya set nama, data sync tak dibuat).
        if self.is_uvm_event_hierarchy(parent.as_str()) {
            return self.execute_uvm_event_method(obj_id, method, args);
        }
        if self.is_uvm_barrier_hierarchy(parent.as_str()) {
            return self.execute_uvm_barrier_method(obj_id, method, args);
        }
        // F22: `super.new(name, parent)` di subclass user (`my_sub extends
        // uvm_subscriber`) — analysis_imp internal dibuat di sini (harus
        // SEBELUM component check — subscriber extends component).
        if self.is_uvm_subscriber_hierarchy(parent.as_str()) {
            return self.execute_uvm_subscriber_method(obj_id, method, args);
        }
        // F23: `super.new` di subclass fifo / export internal.
        if self.is_uvm_tlm_fifo_hierarchy(parent.as_str()) {
            return self.execute_uvm_tlm_fifo_method(obj_id, method, args);
        }
        if self.is_uvm_fifo_export_hierarchy(parent.as_str()) {
            return self.execute_uvm_fifo_export_method(obj_id, method, args);
        }
        // F24: `super.new` di subclass port (`my_port extends uvm_seq_item_port`)
        // — data port di-insert di sini (sebelum component check).
        if self.is_uvm_seq_item_port_hierarchy(parent.as_str()) {
            return self.execute_uvm_seq_item_port_method(obj_id, method, args);
        }
        // Check if parent is uvm_component hierarchy
        if parent == "__uvm_component" || self.is_uvm_component_hierarchy(parent.as_str()) {
            return self.execute_uvm_component_method(obj_id, method, args);
        }
        // Check if parent is uvm_report_object hierarchy
        if parent == "__uvm_report_object" || self.is_uvm_report_object_hierarchy(parent.as_str()) {
            return self.execute_uvm_report_object_method(obj_id, method, args);
        }
        // Check if parent is uvm_object hierarchy
        if parent == "__uvm_object" || self.is_uvm_object_hierarchy(parent.as_str()) {
            return self.execute_uvm_object_method(obj_id, method, args);
        }
        // Super dispatch: start search from parent class, skipping current class override
        let method_def = self.find_method_in_hierarchy(parent.as_str(), method)?.clone();
        self.execute_method_body(Some(obj_id), &method_def, args, method)
    }

}
