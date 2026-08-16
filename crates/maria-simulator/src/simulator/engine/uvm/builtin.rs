//! Dispatcher `super` untuk method builtin UVM.
//!
//! 1 file = 1 tanggung jawab: HANYA `execute_super_method` — resolve parent
//! class (extends) dan dispatch ke method builtin layer yang sesuai. Method
//! builtin per family tinggal di:
//! - `sync.rs`    — event, barrier, mailbox, semaphore, process
//! - `object.rs`  — uvm_object, uvm_report_object, callback
//! - `component.rs` — uvm_component, sequence, sequencer, driver, monitor,
//!   analysis port/imp, subscriber
//! - `reg.rs`     — register model (reg_field, reg, reg_block, reg_map)
//! - `fifo.rs` / `seq.rs` — tlm fifo & handshake sequence

use super::super::SimulationEngine;
use maria_core::diagnostics::DiagCode;
use maria_core::error::SimError;
use maria_ir::*;

impl SimulationEngine {
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
        // VERIF-13: `super.new` di subclass comparator (`my_comp extends
        // uvm_in_order_comparator`) — analysis_imp internal + antrian expected
        // dibangun di sini (sebelum component check — comparator extends
        // component).
        if self.is_uvm_comparator_hierarchy(parent.as_str()) {
            return self.execute_uvm_comparator_method(obj_id, method, args);
        }
        // VERIF-15: `super.new` di subclass heartbeat (`my_hb extends
        // uvm_heartbeat`) — data heartbeat di-insert di sini.
        if self.is_uvm_heartbeat_hierarchy(parent.as_str()) {
            return self.execute_uvm_heartbeat_method(obj_id, method, args);
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
        // Parent UVM library class TIDAK terdaftar di design (filelist tanpa
        // library UVM penuh, mis. `uvm_default_report_server` yang di-extends
        // OpenTitan `dv_report_server`). Di UVM asli class tersebut
        // extends uvm_report_object → uvm_object; `super.new`/method lain di
        // subclass user harus di-dispatch ke builtin report_object (yang
        // meneruskan ke object). Tanpa fallback ini `super.new(name)` error
        // RT8001 "method 'new' not found in class 'uvm_default_report_server'"
        // → simulasi mati.
        if !self.design.classes.contains_key(&parent) && parent.as_str().starts_with("uvm_") {
            return self.execute_uvm_report_object_method(obj_id, method, args);
        }
        // Super dispatch: start search from parent class, skipping current class override
        let method_def = self.find_method_in_hierarchy(parent.as_str(), method)?.clone();
        self.execute_method_body(Some(obj_id), &method_def, args, method)
    }

}
