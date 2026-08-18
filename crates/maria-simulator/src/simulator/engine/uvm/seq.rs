//! uvm_seq_item_port + handshake blocking sequence/sequencer/driver (F24).
//! Alur UVM asli yang diimplementasikan:
//!   seq.start(seqr) → body → start_item(it)   → push ke queue sequencer
//!                                              + release waiter getter
//!   driver: get_next_item(req) → pop queue + tulis lvalue req (block bila kosong)
//!   seq:    finish_item(it)    → block sampai item_done (waiter per-item)
//!   driver: item_done()        → pop current + release waiter finish_item:{item}
//! Waiter dipakai `uvm_sync_waiters` keyed by SEQUENCER id, label
//! "get_next_item"/"try_next_item"/"finish_item:{item_id}" (release selektif,
//! pola uvm_fifo_release_waiters). Resume via ContinueAstBlock (t+1).
//! 1 file = 1 tanggung jawab: handshake sequence.

use super::super::SimulationEngine;
use maria_core::error::SimError;
use maria_compiler::hir::{LogicVec, ObjId};
use crate::simulator::types::*;
use crate::simulator::util::*;
use maria_core::Symbol;

impl SimulationEngine {
    /// Method `uvm_seq_item_port` — port driver↔sequencer. `connect(seqr)`
    /// menyimpan sequencer; get_next_item/item_done/try_next_item mendelegasi
    /// ke sequencer tsb (blocking get_next_item di-intercept block.rs).
    pub(crate) fn execute_uvm_seq_item_port_method(
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
                    .entry(obj_id)
                    .or_insert_with(|| UvmObjectData { name: name.clone() });
                let mut cd = UvmComponentData {
                    parent: None,
                    children: Vec::new(),
                    report_verbosity: super::object::UVM_MEDIUM,
                };
                if parent_obj != 0 {
                    cd.parent = Some(parent_obj);
                    if let Some(pd) = self.uvm_component_data.get_mut(&parent_obj) {
                        pd.children.push(obj_id);
                    }
                }
                self.uvm_component_data.insert(obj_id, cd);
                self.uvm_seq_item_port_data
                    .insert(obj_id, UvmSeqItemPortData { sequencer_id: None });
                Ok(LogicVec::from_u64(1, 1))
            }
            // `port.connect(seqr)` — arg pertama sequencer (atau export
            // sequencer). Kalau bukan sequencer, coba resolve parent (export
            // internal sequencer menyimpan parent = sequencer di
            // uvm_component_data).
            "connect" => {
                let target = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let seqr = self.uvm_resolve_sequencer(target);
                if let Some(pd) = self.uvm_seq_item_port_data.get_mut(&obj_id) {
                    pd.sequencer_id = Some(seqr);
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_next_item" | "try_next_item" | "item_done" => {
                let seqr = self
                    .uvm_seq_item_port_data
                    .get(&obj_id)
                    .and_then(|p| p.sequencer_id)
                    .unwrap_or(0);
                if seqr != 0 {
                    self.execute_uvm_sequencer_method(seqr, method, args)
                } else {
                    Ok(LogicVec::from_u64(0, 64))
                }
            }
            _ => self.execute_uvm_component_method(obj_id, method, args),
        }
    }

    /// Resolve id sequencer utk sebuah objek (driver → sequencer_id, port →
    /// sequencer_id, sequencer → dirinya sendiri). 0 = tak ada.
    pub(crate) fn uvm_seqr_for(&self, obj_id: ObjId) -> ObjId {
        if let Some(dd) = self.uvm_driver_data.get(&obj_id) {
            return dd.sequencer_id.unwrap_or(0);
        }
        if let Some(pd) = self.uvm_seq_item_port_data.get(&obj_id) {
            return pd.sequencer_id.unwrap_or(0);
        }
        if self.uvm_sequencer_data.contains_key(&obj_id) {
            return obj_id;
        }
        0
    }

    /// Arg `connect` bisa sequencer langsung atau export sequencer (obj child
    /// dengan parent = sequencer) — resolve ke sequencer sebenarnya.
    pub(crate) fn uvm_resolve_sequencer(&self, obj_id: ObjId) -> ObjId {
        if self.uvm_sequencer_data.contains_key(&obj_id) {
            return obj_id;
        }
        // Export/child: naik satu level parent.
        if let Some(cd) = self.uvm_component_data.get(&obj_id) {
            if let Some(p) = cd.parent {
                if self.uvm_sequencer_data.contains_key(&p) {
                    return p;
                }
            }
        }
        obj_id
    }

    /// Pop item terdepan dari queue sequencer + set current_item (grant).
    pub(crate) fn uvm_seq_pop(&mut self, seqr_id: ObjId) -> Option<ObjId> {
        let popped = self
            .uvm_sequencer_data
            .get_mut(&seqr_id)
            .and_then(|sd| {
                let item = sd.item_queue.first().copied();
                if item.is_some() {
                    sd.item_queue.remove(0);
                }
                item
            });
        if let (Some(item), Some(sd)) = (popped, self.uvm_sequencer_data.get_mut(&seqr_id)) {
            sd.current_item = Some(item);
        }
        popped
    }

    /// Apakah finish_item harus BLOCK? Benar bila ada driver/port terhubung ke
    /// sequencer (konsumen) DAN item belum selesai diproses — item masih di
    /// queue (belum di-grant) atau sedang current (diproses driver). Tanpa
    /// konsumen (pola F17 uvm_do tanpa driver) → no-op, tidak deadlock.
    pub(crate) fn uvm_seq_finish_blocks(&self, seqr_id: ObjId, item_id: ObjId) -> bool {
        let has_consumer = self
            .uvm_driver_data
            .values()
            .any(|d| d.sequencer_id == Some(seqr_id))
            || self
                .uvm_seq_item_port_data
                .values()
                .any(|p| p.sequencer_id == Some(seqr_id));
        if !has_consumer {
            return false;
        }
        if let Some(sd) = self.uvm_sequencer_data.get(&seqr_id) {
            if sd.current_item == Some(item_id) {
                return true;
            }
            if sd.item_queue.contains(&item_id) {
                return true;
            }
        }
        false
    }

    /// Register waiter blocking pada sequencer + suspend. Caller memastikan
    /// kondisi belum terpenuhi; resume via uvm_seq_release_*.
    pub(crate) fn uvm_seq_try_wait(
        &mut self,
        seqr_id: ObjId,
        label: String,
        continuation: Vec<maria_ast::Stmt>,
        fork_id: Option<usize>,
        this: Option<ObjId>,
        method_opt: Option<Symbol>,
    ) -> Result<(), SimError> {
        self.uvm_sync_waiters.entry(seqr_id).or_default().push(UvmSyncWaiter {
            continuation,
            fork_id,
            this,
            method: method_opt,
            wait_label: label,
        });
        Ok(())
    }

    /// Resume waiter getter (driver get_next_item) pada sequencer — label
    /// "get_next_item"/"try_next_item". Dipanggil start_item (push item).
    pub(crate) fn uvm_seq_release_getters(&mut self, seqr_id: ObjId) -> Result<(), SimError> {
        let labels = ["get_next_item", "try_next_item"];
        self.uvm_seq_release_labels(seqr_id, &labels)
    }

    /// Resume waiter finisher (sequence finish_item) utk item tertentu — label
    /// "finish_item:{item}". Dipanggil item_done.
    pub(crate) fn uvm_seq_release_finishers(
        &mut self,
        seqr_id: ObjId,
        item_id: ObjId,
    ) -> Result<(), SimError> {
        let label = format!("finish_item:{}", item_id);
        let labels = [label.as_str()];
        self.uvm_seq_release_labels(seqr_id, &labels)
    }

    /// Release selektif waiter pada sequencer berdasarkan label (pola
    /// uvm_fifo_release_waiters): partition → ContinueAstBlock t+1.
    pub(crate) fn uvm_seq_release_labels(
        &mut self,
        seqr_id: ObjId,
        labels: &[&str],
    ) -> Result<(), SimError> {
        let all = self.uvm_sync_waiters.remove(&seqr_id).unwrap_or_default();
        let (matched, rest): (Vec<_>, Vec<_>) =
            all.into_iter().partition(|w| labels.contains(&w.wait_label.as_str()));
        if !rest.is_empty() {
            self.uvm_sync_waiters.insert(seqr_id, rest);
        }
        let t = self.state.time as usize + 1;
        self.ensure_events(t);
        for w in matched {
            self.push_event(
                t,
                crate::simulator::engine::RegionEvent {
                    region: crate::simulator::engine::EventRegion::Active,
                    event: crate::simulator::engine::EventKind::ContinueAstBlock(
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
}
