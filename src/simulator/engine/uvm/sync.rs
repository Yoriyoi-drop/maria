//! uvm_event + uvm_barrier — sinkronisasi antar komponen UVM (F21).
//! `uvm_event`: trigger()/wait_trigger()/triggered()/reset()/is_on()/wait_on().
//! `uvm_barrier`: new(name, threshold)/wait_for()/wait_for_count(n)/reset().
//! Blocking `wait_*` TIDAK di-handle di sini (method hanya side-effect-free
//! query) — block.rs mendeteksi MethodCall wait_* dan memanggil
//! `uvm_try_wait` (suspend + daftarkan waiter) / `uvm_release_waiters`
//! (resume semua waiter saat trigger()/barrier penuh).
//! 1 file = 1 tanggung jawab: hanya sinkronisasi event & barrier.

use super::super::SimulationEngine;
use crate::diagnostics::DiagCode;
use crate::error::SimError;
use crate::hir::{LogicVec, ObjId};
use crate::simulator::types::*;
use crate::simulator::util::*;
use crate::Symbol;

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
        continuation: Vec<crate::ast::Stmt>,
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
}
