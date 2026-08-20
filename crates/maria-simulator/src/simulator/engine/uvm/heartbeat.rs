//! uvm_heartbeat (VERIF-15).
//! Komponen monitor liveness: object yang di-monitor wajib memanggil
//! `heartbeat(obj)` minimal `required` kali sebelum test selesai. `set_heartbeat`
//! mendaftarkan object + jumlah wajib; `heartbeat(obj)` menambah counter;
//! `check()` (biasanya di check_phase) mengembalikan 0 dan emit UVM_ERROR
//! bila ada object yang heartbeat-nya kurang. 1 file = 1 tanggung jawab:
//! heartbeat monitoring.

use super::super::SimulationEngine;
use maria_core::error::SimError;
use maria_compiler::hir::{LogicVec, ObjId};
use crate::simulator::types::*;
use crate::simulator::util::*;

impl SimulationEngine {
    /// Method builtin `uvm_heartbeat`.
    pub(crate) fn execute_uvm_heartbeat_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "new" => {
                let name = args
                    .first()
                    .map(logicvec_to_string)
                    .unwrap_or_else(|| format!("heartbeat_{}", obj_id));
                self.uvm_object_data
                    .entry(obj_id)
                    .or_insert_with(|| UvmObjectData { name: name.clone() });
                self.uvm_heartbeat_data
                    .entry(obj_id)
                    .or_insert_with(UvmHeartbeatData::default);
                Ok(LogicVec::from_u64(1, 1))
            }
            // set_heartbeat(object, count) — daftarkan object wajib heartbeat.
            "set_heartbeat" => {
                let target = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let count = args.get(1).map(|a| a.to_u64()).unwrap_or(1);
                if let Some(hb) = self.uvm_heartbeat_data.get_mut(&obj_id) {
                    hb.required.insert(target, count);
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            // heartbeat(object) — object ter-monitor memberi sinyal hidup.
            "heartbeat" => {
                let target = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                if let Some(hb) = self.uvm_heartbeat_data.get_mut(&obj_id) {
                    let c = hb.received.entry(target).or_insert(0);
                    *c += 1;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            // check() — 1 bila semua object memenuhi required; 0 + UVM_ERROR
            // bila ada yang kurang (pola UVM asli: report di check_phase).
            "check" => {
                let mut ok = true;
                let missing: Vec<(u64, u64, u64)> = self
                    .uvm_heartbeat_data
                    .get(&obj_id)
                    .map(|hb| {
                        hb.required
                            .iter()
                            .map(|(o, req)| (*o as u64, *req, *hb.received.get(o).unwrap_or(&0)))
                            .collect()
                    })
                    .unwrap_or_default();
                for (o, req, got) in &missing {
                    if got < req {
                        ok = false;
                        let name = self
                            .uvm_object_data
                            .get(&(*o as ObjId))
                            .map(|d| d.name.as_str())
                            .unwrap_or("unknown");
                        self.emit_severity(
                            "error",
                            &format!(
                                "[UVM_HEARTBEAT] '{}' heartbeat kurang: {}/{}",
                                name, got, req
                            ),
                        );
                    }
                }
                Ok(LogicVec::from_u64(if ok { 1 } else { 0 }, 1))
            }
            // get_heartbeat_count(object) — query jumlah heartbeat diterima.
            "get_heartbeat_count" => {
                let target = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let c = self
                    .uvm_heartbeat_data
                    .get(&obj_id)
                    .and_then(|hb| hb.received.get(&target))
                    .copied()
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(c, 64))
            }
            _ => self.execute_uvm_component_method(obj_id, method, args),
        }
    }
}
