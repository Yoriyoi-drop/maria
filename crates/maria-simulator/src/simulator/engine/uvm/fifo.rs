//! uvm_tlm_fifo + analysis_export internal (F23).
//! FIFO TLM dgn blocking `put`/`get`/`peek` dan export analysis internal
//! (`fifo.analysis_export.write(item)` → put). Queue menyimpan ObjId item.
//! Blocking `get` saat kosong & `put` saat penuh men-suspend caller (waiter
//! di `uvm_sync_waiters`, method di-label "get"/"peek"/"put") — resume saat
//! data tersedia. `get`/`try_get` di jalur statement (block.rs) menulis item
//! ke lvalue arg (AST) setelah pop — di sini hanya logika FIFO murni.
//! 1 file = 1 tanggung jawab: FIFO TLM.

use super::super::SimulationEngine;
use crate::simulator::types::*;
use crate::simulator::util::*;
use maria_compiler::hir::{LogicVec, ObjId};
use maria_core::diagnostics::DiagCode;
use maria_core::error::SimError;
use maria_core::Symbol;

impl SimulationEngine {
    /// Method `uvm_tlm_fifo` — query & non-blocking ops.
    /// Blocking `get`/`put`/`peek` dijalankan di block.rs `Stmt::Expr`
    /// (suspend + waiter); di sini (konteks ekspresi) hanya status terkini.
    pub(crate) fn execute_uvm_tlm_fifo_method(
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
                    .unwrap_or_else(|| format!("fifo_{}", obj_id));
                let size = args.get(2).map(|a| a.to_u64() as usize).unwrap_or(1);
                let mut fd = UvmTlmFifoData::new(name, size);
                self.uvm_object_data
                    .entry(obj_id)
                    .or_insert_with(|| UvmObjectData {
                        name: fd.name.clone(),
                    });
                // Export analysis internal: `fifo.analysis_export.write(item)`
                // → put. Objek `__uvm_fifo_export` dgn parent fifo.
                let export_id = self.state.alloc_object(Symbol::intern("__uvm_fifo_export"));
                self.uvm_fifo_export_data.insert(export_id, obj_id);
                self.uvm_object_data
                    .entry(export_id)
                    .or_insert_with(|| UvmObjectData {
                        name: format!("{}_export", fd.name),
                    });
                fd.export_id = Some(export_id);
                self.uvm_tlm_fifo_data.insert(obj_id, fd);
                if let Some(obj) = self.state.get_object_mut(obj_id) {
                    obj.fields.insert(
                        Symbol::intern("analysis_export"),
                        LogicVec::from_u64(export_id as u64, 64),
                    );
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            // put — non-blocking fallback (jalur blocking di block.rs).
            "put" => {
                let item = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_tlm_fifo not initialized");
                let fd = self
                    .uvm_tlm_fifo_data
                    .get_mut(&obj_id)
                    .ok_or_else(|| err.clone())?;
                if fd.queue.len() < fd.capacity {
                    fd.queue.push_back(item);
                    self.uvm_fifo_release_waiters(obj_id, true)?;
                    Ok(LogicVec::from_u64(1, 1))
                } else {
                    // Penuh + non-blocking konteks: warning, item dibuang.
                    let fname = fd.name.clone();
                    self.emit_warning(
                        DiagCode::NullHandle,
                        format!(
                            "uvm_tlm_fifo '{}' full; non-blocking put dropped item {}",
                            fname, item
                        ),
                    );
                    Ok(LogicVec::from_u64(0, 1))
                }
            }
            "try_put" => {
                let item = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_tlm_fifo not initialized");
                let fd = self
                    .uvm_tlm_fifo_data
                    .get_mut(&obj_id)
                    .ok_or_else(|| err.clone())?;
                if fd.queue.len() < fd.capacity {
                    fd.queue.push_back(item);
                    self.uvm_fifo_release_waiters(obj_id, true)?;
                    Ok(LogicVec::from_u64(1, 1))
                } else {
                    Ok(LogicVec::from_u64(0, 1))
                }
            }
            // get/try_get/peek — fallback ekspresi: ambil tanpa tulis lvalue
            // (lvalue ditulis di jalur statement block.rs).
            "get" | "try_get" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_tlm_fifo not initialized");
                let fd = self
                    .uvm_tlm_fifo_data
                    .get_mut(&obj_id)
                    .ok_or_else(|| err.clone())?;
                if let Some(item) = fd.queue.pop_front() {
                    self.uvm_fifo_release_waiters(obj_id, false)?;
                    Ok(LogicVec::from_u64(item as u64, 64))
                } else {
                    Ok(LogicVec::from_u64(0, 64))
                }
            }
            "peek" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_tlm_fifo not initialized");
                let fd = self
                    .uvm_tlm_fifo_data
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                Ok(LogicVec::from_u64(
                    fd.queue.front().copied().unwrap_or(0) as u64,
                    64,
                ))
            }
            "capacity" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_tlm_fifo not initialized");
                let fd = self
                    .uvm_tlm_fifo_data
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                Ok(LogicVec::from_u64(fd.capacity as u64, 32))
            }
            "used" | "size_used" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_tlm_fifo not initialized");
                let fd = self
                    .uvm_tlm_fifo_data
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                Ok(LogicVec::from_u64(fd.queue.len() as u64, 32))
            }
            "is_empty" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_tlm_fifo not initialized");
                let fd = self
                    .uvm_tlm_fifo_data
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                Ok(LogicVec::from_u64(fd.queue.is_empty() as u64, 1))
            }
            "is_full" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_tlm_fifo not initialized");
                let fd = self
                    .uvm_tlm_fifo_data
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                Ok(LogicVec::from_u64(
                    (fd.queue.len() >= fd.capacity) as u64,
                    1,
                ))
            }
            "flush" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_tlm_fifo not initialized");
                let fd = self
                    .uvm_tlm_fifo_data
                    .get_mut(&obj_id)
                    .ok_or_else(|| err.clone())?;
                fd.queue.clear();
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_component_method(obj_id, method, args),
        }
    }

    /// Method `__uvm_fifo_export` — export analysis internal fifo.
    /// `write(item)` → put non-blocking ke fifo parent (item dibuang + warning
    /// bila penuh, sesuai semantics analysis write fire-and-forget UVM).
    pub(crate) fn execute_uvm_fifo_export_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        // Catatan: `new` TIDAK di-dispatch — export dibuat langsung di
        // execute_uvm_tlm_fifo_method "new" via alloc_object + insert data
        // (bukan melalui execute_method), jadi tidak ada arm "new" di sini.
        match method {
            "write" => {
                let item = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let fifo_id = self.uvm_fifo_export_data.get(&obj_id).copied().unwrap_or(0);
                if fifo_id != 0 {
                    let err =
                        SimError::with_diag(DiagCode::NullHandle, "uvm_tlm_fifo not initialized");
                    let fd = self
                        .uvm_tlm_fifo_data
                        .get_mut(&fifo_id)
                        .ok_or_else(|| err.clone())?;
                    if fd.queue.len() < fd.capacity {
                        fd.queue.push_back(item);
                        let fid = fifo_id;
                        self.uvm_fifo_release_waiters(fid, true)?;
                        return Ok(LogicVec::from_u64(1, 1));
                    }
                    let fname = fd.name.clone();
                    self.emit_warning(
                        DiagCode::NullHandle,
                        format!(
                            "uvm_tlm_fifo '{}' full; analysis write dropped item {}",
                            fname, item
                        ),
                    );
                }
                Ok(LogicVec::from_u64(0, 1))
            }
            _ => self.execute_uvm_analysis_port_method(obj_id, method, args),
        }
    }

    /// Blocking wait FIFO (dipanggil block.rs `Stmt::Expr`):
    /// - "get"/"peek": kosong → daftar waiter, suspend (Ok(true));
    ///   tersedia → pop/peek + tulis lvalue (di block.rs) → Ok(false).
    /// - "put": penuh → daftar waiter, suspend; ada ruang → push → Ok(false).
    /// Return Ok(true) = harus suspend.
    pub(crate) fn uvm_try_fifo_wait(
        &mut self,
        obj_id: ObjId,
        method: &str,
        continuation: Vec<maria_ast::Stmt>,
        fork_id: Option<usize>,
        this: Option<ObjId>,
        method_opt: Option<Symbol>,
    ) -> Result<bool, SimError> {
        let err = SimError::with_diag(DiagCode::NullHandle, "uvm_tlm_fifo not initialized");
        let fd = self
            .uvm_tlm_fifo_data
            .get(&obj_id)
            .ok_or_else(|| err.clone())?;
        let is_getter = matches!(method, "get" | "peek");
        if is_getter {
            if !fd.queue.is_empty() {
                return Ok(false);
            }
        } else if method == "put" {
            if fd.queue.len() < fd.capacity {
                return Ok(false);
            }
        } else {
            return Ok(false);
        }
        self.uvm_sync_waiters
            .entry(obj_id)
            .or_default()
            .push(UvmSyncWaiter {
                continuation,
                fork_id,
                this,
                method: method_opt,
                wait_label: method.to_string(),
            });
        Ok(true)
    }

    /// Resume waiter FIFO yang match: `getters=true` → "get"/"peek";
    /// `getters=false` → "put". Menjadwalkan ContinueAstBlock t+1.
    pub(crate) fn uvm_fifo_release_waiters(
        &mut self,
        obj_id: ObjId,
        getters: bool,
    ) -> Result<(), SimError> {
        let all = self.uvm_sync_waiters.remove(&obj_id).unwrap_or_default();
        let (matched, rest): (Vec<_>, Vec<_>) = all.into_iter().partition(|w| {
            if getters {
                w.wait_label == "get" || w.wait_label == "peek"
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
                    event: EventKind::ContinueAstBlock(w.continuation, w.fork_id, w.this, w.method),
                },
            );
        }
        Ok(())
    }
}
