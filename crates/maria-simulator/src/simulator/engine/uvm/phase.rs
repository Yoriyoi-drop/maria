//! `uvm_phase` — handle phase UVM: jump / get_name / skip (VERIF-05).
//!
//! UVM 1.2 `uvm_phase` adalah objek yang diteruskan ke method phase
//! (`build_phase(uvm_phase phase)`) dan menyediakan:
//! - `jump(string phase_name)` — melompat ke fase target: fase yang belum
//!   dijalankan di antaranya di-skip, eksekusi berlanjut dari fase target.
//!   Di engine: `uvm_phase_jump` di-set; `run_phase_tree` menggunakannya utk
//!   memulai ulang dari fase target (fase berikutnya tetap dijalankan).
//! - `get_name()` — nama phase saat ini (yang sedang dieksekusi).
//! - `skip()` — menandai phase ini di-skip (tidak dieksekusi).
//!
//! 1 file = 1 tanggung jawab: hanya uvm_phase.

use super::super::SimulationEngine;
use maria_core::Symbol;
use maria_core::diagnostics::DiagCode;
use maria_core::error::SimError;
use maria_compiler::hir::{LogicVec, ObjId};
use crate::simulator::util::{logicvec_to_string, string_to_logicvec};

impl SimulationEngine {
    /// Argumen fase: objek uvm_phase handle (dibuat sekali, di-cache) —
    /// di-inject sebagai arg[0] ke build_phase(uvm_phase phase) dsb agar user
    /// bisa memanggil phase.jump()/phase.get_name()/phase.skip().
    pub(crate) fn uvm_phase_args(&mut self) -> Vec<LogicVec> {
        let id = match self.uvm_phase_handle {
            Some(id) => id,
            None => {
                let id = self.state.alloc_object(Symbol::intern("uvm_phase"));
                self.uvm_phase_handle = Some(id);
                id
            }
        };
        vec![LogicVec::from_u64(id as u64, 64)]
    }

    /// Method `uvm_phase` — handle phase (non-block, query/request).
    pub(crate) fn execute_uvm_phase_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            // new: kembalikan obj_id handle (objek sudah dialokasi engine).
            "new" => Ok(LogicVec::from_u64(obj_id as u64, 64)),
            // Nama phase saat ini (yang sedang/sudah dijalankan run_phase_tree).
            "get_name" => Ok(string_to_logicvec(
                &self.uvm_current_phase
                    .clone()
                    .unwrap_or_default(),
            )),
            // VERIF-05: jump ke fase target — set uvm_phase_jump; run_phase_tree
            // memulai ulang dari fase target saat dipanggil berikutnya.
            "jump" => {
                let target = args
                    .first()
                    .map(logicvec_to_string)
                    .unwrap_or_default();
                if target.is_empty() {
                    self.emit_warning(
                        DiagCode::DpiError,
                        "uvm_phase::jump: nama phase target kosong",
                    );
                } else {
                    self.uvm_phase_jump = Some(target);
                }
                Ok(LogicVec::from_u64(0, 1))
            }
            // VERIF-05: skip phase ini — tandai nama phase saat ini supaya
            // run_phase_tree melewatinya bila belum dieksekusi.
            "skip" => {
                let current = self.uvm_current_phase.clone().unwrap_or_default();
                if !current.is_empty() {
                    self.uvm_phase_jump = Some(format!("skip:{}", current));
                }
                Ok(LogicVec::from_u64(0, 1))
            }
            "get_type_name" => Ok(string_to_logicvec("uvm_phase")),
            other => {
                self.emit_warning(
                    DiagCode::DpiError,
                    format!("uvm_phase: unknown method '{}'", other),
                );
                Ok(LogicVec::from_u64(0, 64))
            }
        }
    }
}
