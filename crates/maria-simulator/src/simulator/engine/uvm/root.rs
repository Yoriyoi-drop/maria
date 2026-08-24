//! `uvm_root` — kelas root UVM: singleton + top-level component (VERIF-04).
//!
//! Method inti UVM yang didukung:
//! - `get()` — singleton handle (obj_id yang sama untuk semua panggilan,
//!   disimpan di `uvm_root_id`; pola sama dengan uvm_cmdline_processor::get).
//! - `get_top()` — komponen top-level (obj id `uvm_test_top` dari
//!   run_test/execute_phases); 0 (null) bila tidak ada test berjalan.
//! - `run_test(string name)` — varian class-method: buat objek test & jalankan
//!   fase UVM (delegasi ke `run_uvm_test`, guard `uvm_phases_started`).
//!
//! 1 file = 1 tanggung jawab: hanya uvm_root.

use super::super::SimulationEngine;
use crate::simulator::util::logicvec_to_string;
use maria_compiler::hir::{LogicVec, ObjId};
use maria_core::diagnostics::DiagCode;
use maria_core::error::SimError;

impl SimulationEngine {
    /// Method `uvm_root` — singleton root + top-level component (non-block).
    pub(crate) fn execute_uvm_root_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            // Singleton: semua panggilan get() mengembalikan obj_id yang sama
            // (disimpan di uvm_root_id agar state konsisten).
            "get" | "new" => {
                let id = if self.uvm_root_id.is_none() {
                    self.uvm_root_id = Some(obj_id);
                    obj_id
                } else {
                    self.uvm_root_id.unwrap()
                };
                Ok(LogicVec::from_u64(id as u64, 64))
            }
            // Komponen top-level: obj id uvm_test_top (0/null bila tidak ada).
            "get_top" => Ok(LogicVec::from_u64(
                self.root_test_obj_id.unwrap_or(0) as u64,
                64,
            )),
            // Varian class-method run_test("name") — sama dgn bare run_test.
            "run_test" => {
                let test_name = args.first().map(logicvec_to_string).unwrap_or_default();
                self.run_uvm_test(&test_name)?;
                Ok(LogicVec::from_u64(1, 1))
            }
            other => {
                self.emit_warning(
                    DiagCode::DpiError,
                    format!("uvm_root: unknown method '{}'", other),
                );
                Ok(LogicVec::from_u64(0, 64))
            }
        }
    }
}
