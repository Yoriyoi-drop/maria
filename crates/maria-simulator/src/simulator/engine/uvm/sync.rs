//! Primitif sinkronisasi antar komponen UVM — event, barrier, mailbox,
//! semaphore, dan process handle (F21/LANG-24).
//! `uvm_event`: trigger()/wait_trigger()/triggered()/reset()/is_on()/wait_on().
//! `uvm_barrier`: new(name, threshold)/wait_for()/wait_for_count(n)/reset().
//! `mailbox`: new(bound)/put/get/try_put/try_get/num/size + blocking wait
//!            (uvm_try_mailbox_wait / uvm_mailbox_release_waiters).
//! `semaphore`: new(n)/get/put/try_get.
//! `process`: status/kill/await/self/suspend/resume.
//! Blocking `wait_*` TIDAK di-handle di sini (method hanya side-effect-free
//! query) — block.rs mendeteksi MethodCall wait_* dan memanggil
//! `uvm_try_wait` (suspend + daftarkan waiter) / `uvm_release_waiters`
//! (resume semua waiter saat trigger()/barrier penuh).
//! 1 file = 1 tanggung jawab: hanya sinkronisasi & primitif konkurensi.

use super::super::SimulationEngine;
use maria_core::diagnostics::DiagCode;
use maria_core::error::SimError;
use maria_compiler::hir::{LogicVec, ObjId};
use crate::simulator::types::*;
use crate::simulator::util::*;
use maria_core::Symbol;

impl SimulationEngine {
    /// Method `uvm_event` — query & trigger (non-blocking).
    /// Blocking `wait_trigger`/`wait_on` dipanggil via blok langsung (bukan di
    /// sini) — method di sini hanya return status terkini (perilaku non-block).
    pub(crate) fn execute_uvm_event_method(
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
                    .unwrap_or_else(|| format!("event_{}", obj_id));
                self.uvm_event_data.insert(obj_id, UvmEventData::new(name));
                Ok(LogicVec::from_u64(1, 1))
            }
            "trigger" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_event not initialized");
                let e = self
                    .uvm_event_data
                    .get_mut(&obj_id)
                    .ok_or_else(|| err.clone())?;
                e.triggered = true;
                e.on = true;
                // F21: bangunkan semua waiter `wait_trigger`/`wait_on`.
                self.uvm_release_waiters(obj_id)?;
                Ok(LogicVec::from_u64(1, 1))
            }
            "triggered" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_event not initialized");
                let e = self
                    .uvm_event_data
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                Ok(LogicVec::from_u64(e.triggered as u64, 1))
            }
            "is_on" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_event not initialized");
                let e = self
                    .uvm_event_data
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                Ok(LogicVec::from_u64(e.on as u64, 1))
            }
            "reset" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_event not initialized");
                let e = self
                    .uvm_event_data
                    .get_mut(&obj_id)
                    .ok_or_else(|| err.clone())?;
                // F21 review: UVM reset mematikan on (wait_on akan block lagi).
                e.triggered = false;
                e.on = false;
                Ok(LogicVec::from_u64(1, 1))
            }
            "on_off" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_event not initialized");
                let e = self
                    .uvm_event_data
                    .get_mut(&obj_id)
                    .ok_or_else(|| err.clone())?;
                if let Some(v) = args.first() {
                    e.on = v.to_bool().unwrap_or(true);
                }
                Ok(LogicVec::from_u64(e.on as u64, 1))
            }
            // Non-blocking query fallback: `wait_trigger()`/`wait_on()` di
            // konteks non-blocking (mis. dipanggil sbg ekspresi) — return flag
            // terkini. Jalur blocking sebenarnya di block.rs `Stmt::Expr`.
            "wait_trigger" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_event not initialized");
                let e = self
                    .uvm_event_data
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                Ok(LogicVec::from_u64(e.triggered as u64, 1))
            }
            "wait_on" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_event not initialized");
                let e = self
                    .uvm_event_data
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                Ok(LogicVec::from_u64(e.on as u64, 1))
            }
            _ => Err(self.diag_error(
                DiagCode::NotImplemented,
                format!("unknown uvm_event method: {}", method),
            )),
        }
    }

    /// Method `uvm_barrier` — non-blocking query & state mutation.
    /// Blocking `wait_for`/`wait_for_count` via uvm_try_wait di block.rs.
    pub(crate) fn execute_uvm_barrier_method(
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
                    .unwrap_or_else(|| format!("barrier_{}", obj_id));
                let threshold = args.get(1).map(|a| a.to_u64() as u32).unwrap_or(1);
                self.uvm_barrier_data
                    .insert(obj_id, UvmBarrierData::new(name, threshold));
                Ok(LogicVec::from_u64(1, 1))
            }
            "wait_for" | "wait_for_count" => {
                // Jalur blocking: block.rs `Stmt::Expr` memanggil uvm_try_wait
                // yang menambah count + suspend. Di sini (ekspresi non-block)
                // cukup cek status — count sudah ditambah oleh uvm_try_wait.
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_barrier not initialized");
                let b = self
                    .uvm_barrier_data
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                let threshold = if method == "wait_for_count" {
                    args.first().map(|a| a.to_u64() as u32).unwrap_or(1)
                } else {
                    b.threshold
                };
                Ok(LogicVec::from_u64((b.count >= threshold) as u64, 1))
            }
            "reset" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_barrier not initialized");
                let b = self
                    .uvm_barrier_data
                    .get_mut(&obj_id)
                    .ok_or_else(|| err.clone())?;
                b.count = 0;
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_threshold" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_barrier not initialized");
                let b = self
                    .uvm_barrier_data
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                Ok(LogicVec::from_u64(b.threshold as u64, 32))
            }
            "set_threshold" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_barrier not initialized");
                let b = self
                    .uvm_barrier_data
                    .get_mut(&obj_id)
                    .ok_or_else(|| err.clone())?;
                if let Some(v) = args.first() {
                    b.threshold = (v.to_u64() as u32).max(1);
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => Err(self.diag_error(
                DiagCode::NotImplemented,
                format!("unknown uvm_barrier method: {}", method),
            )),
        }
    }

    /// Blocking wait `wait_trigger`/`wait_on`/`wait_for`/`wait_for_count`.
    /// Dipanggil dari block.rs `Stmt::Expr` SAAT method wait_* dijumpai.
    /// Return Ok(true) = harus suspend (waiter terdaftar, kontinuasi disimpan);
    /// Ok(false) = kondisi sudah terpenuhi, lanjut statement berikutnya.
    /// Side effect: `wait_for` MENAMBAH count sekali (statement tidak diulang
    /// karena continuation = statement SETELAH wait).
    pub(crate) fn uvm_try_wait(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
        continuation: Vec<maria_ast::Stmt>,
        fork_id: Option<usize>,
        this: Option<ObjId>,
        method_opt: Option<Symbol>,
    ) -> Result<bool, SimError> {
        match method {
            "wait_trigger" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_event not initialized");
                let e = self
                    .uvm_event_data
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                if e.triggered {
                    return Ok(false);
                }
                self.uvm_sync_waiters.entry(obj_id).or_default().push(UvmSyncWaiter {
                    continuation,
                    fork_id,
                    this,
                    method: method_opt,
                    wait_label: "wait_trigger".to_string(),
                });
                Ok(true)
            }
            "wait_on" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_event not initialized");
                let e = self
                    .uvm_event_data
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                if e.on {
                    return Ok(false);
                }
                self.uvm_sync_waiters.entry(obj_id).or_default().push(UvmSyncWaiter {
                    continuation,
                    fork_id,
                    this,
                    method: method_opt,
                    wait_label: "wait_on".to_string(),
                });
                Ok(true)
            }
            "wait_for" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_barrier not initialized");
                let b = self
                    .uvm_barrier_data
                    .get_mut(&obj_id)
                    .ok_or_else(|| err.clone())?;
                b.count += 1;
                if b.count >= b.threshold {
                    b.count = 0; // auto-reset (UVM asli)
                    self.uvm_release_waiters(obj_id)?;
                    return Ok(false);
                }
                self.uvm_sync_waiters.entry(obj_id).or_default().push(UvmSyncWaiter {
                    continuation,
                    fork_id,
                    this,
                    method: method_opt,
                    wait_label: "wait_for".to_string(),
                });
                Ok(true)
            }
            "wait_for_count" => {
                let err = SimError::with_diag(DiagCode::NullHandle, "uvm_barrier not initialized");
                let target = args.first().map(|a| a.to_u64() as u32).unwrap_or(1);
                let b = self
                    .uvm_barrier_data
                    .get(&obj_id)
                    .ok_or_else(|| err.clone())?;
                if b.count >= target {
                    self.uvm_release_waiters(obj_id)?;
                    return Ok(false);
                }
                self.uvm_sync_waiters.entry(obj_id).or_default().push(UvmSyncWaiter {
                    continuation,
                    fork_id,
                    this,
                    method: method_opt,
                    wait_label: "wait_for_count".to_string(),
                });
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Resume semua waiter blocking pada objek event/barrier — jadwalkan
    /// `ContinueAstBlock` untuk tiap kontinuasi (t+1, pola get_next_item).
    pub(crate) fn uvm_release_waiters(&mut self, obj_id: ObjId) -> Result<(), SimError> {
        let waiters = self.uvm_sync_waiters.remove(&obj_id).unwrap_or_default();
        let t = self.state.time as usize + 1;
        self.ensure_events(t);
        for w in waiters {
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

    // ─── mailbox (LANG-24) ───────────────────────────────────────────────

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

    // ─── semaphore ───────────────────────────────────────────────────────

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

    // ─── process handle ──────────────────────────────────────────────────

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
}
