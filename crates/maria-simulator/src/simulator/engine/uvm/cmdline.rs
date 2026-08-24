//! `uvm_cmdline_processor` — singleton pembaca plusarg command line (VERIF-03).
//!
//! Method inti UVM yang didukung:
//! - `get()` / `get_uvm_cmdline_processor()` — singleton handle (obj_id yang
//!   sama untuk semua panggilan, disimpan di `uvm_cmdline_id`).
//! - `get_plusargs()` — string array semua plusarg (format `+key=val`).
//! - `get_args()` — string array argumen non-plusarg (biasanya kosong).
//! - `has_plusarg(string name)` — bit 1 bila ada plusarg yang namanya
//!   diawali `name` (pola `$test$plusargs`).
//! - `get_arg_value(string name, ref string value)` — bit 1 bila plusarg
//!   `name=...` ada; nilai disimpan di `uvm_cmdline_last_value` untuk dibaca
//!   `get_arg_value_out()` (ref out). Nilai di-strip awalan `+` dan opsi
//!   format `%s`/`%d` pada name (pola `$value$plusargs`).
//! - `get_arg_values(string name, ref string values[$])` — semua nilai yang
//!   cocok (biasanya 1; disimpan di `uvm_cmdline_values`).
//!
//! 1 file = 1 tanggung jawab: hanya uvm_cmdline_processor.

use super::super::SimulationEngine;
use crate::simulator::util::{logicvec_to_string, string_to_logicvec};
use maria_compiler::hir::{LogicVec, ObjId};
use maria_core::diagnostics::DiagCode;
use maria_core::error::SimError;

impl SimulationEngine {
    /// Method `uvm_cmdline_processor` — query plusarg command line (non-block).
    pub(crate) fn execute_uvm_cmdline_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            // Singleton: semua panggilan get() mengembalikan obj_id yang sama
            // (disimpan di uvm_cmdline_id agar state konsisten).
            "get" | "get_uvm_cmdline_processor" | "new" => {
                let id = if self.uvm_cmdline_id.is_none() {
                    self.uvm_cmdline_id = Some(obj_id);
                    obj_id
                } else {
                    self.uvm_cmdline_id.unwrap()
                };
                Ok(LogicVec::from_u64(id as u64, 64))
            }
            "get_plusargs" => {
                // String array: representasi ringkas sebagai LogicVec dengan
                // width = jumlah plusarg (0 bila tidak ada) — array string
                // penuh tidak didukung di return LogicVec; daftar disimpan di
                // uvm_cmdline_values untuk get_arg_values.
                let keys: Vec<String> = self.plusargs.keys().cloned().collect();
                self.uvm_cmdline_values = keys
                    .into_iter()
                    .map(|k| {
                        format!(
                            "+{}={}",
                            k,
                            self.plusargs.get(&k).cloned().unwrap_or_default()
                        )
                    })
                    .collect();
                Ok(LogicVec::from_u64(self.uvm_cmdline_values.len() as u64, 64))
            }
            "get_args" => {
                // Argumen non-plusarg tidak di-track terpisah — kosong.
                Ok(LogicVec::from_u64(0, 64))
            }
            "has_plusarg" | "has_arg" => {
                let Some(pat) = args.first() else {
                    return Ok(LogicVec::from_u64(0, 1));
                };
                let pat_str = logicvec_to_string(pat);
                let pat_str = pat_str.trim_start_matches('+');
                let hit = self.plusargs.keys().any(|k| k.starts_with(pat_str));
                Ok(LogicVec::from_u64(hit as u64, 1))
            }
            "get_arg_value" => {
                let Some(name) = args.first() else {
                    return Ok(LogicVec::from_u64(0, 1));
                };
                let name_str = logicvec_to_string(name);
                // `$value$plusargs` pattern: `name=%s` / `name=%d` / `name=`.
                let base = name_str
                    .split('%')
                    .next()
                    .unwrap_or(&name_str)
                    .trim_start_matches('+')
                    .trim_end_matches('=');
                let mut found = false;
                let mut val = String::new();
                for (key, v) in &self.plusargs {
                    if key == base {
                        found = true;
                        val = v.clone();
                        break;
                    }
                }
                if found {
                    self.uvm_cmdline_last_value = val;
                }
                Ok(LogicVec::from_u64(found as u64, 1))
            }
            "get_arg_values" => {
                let Some(name) = args.first() else {
                    return Ok(LogicVec::from_u64(0, 1));
                };
                let name_str = logicvec_to_string(name);
                let base = name_str
                    .split('%')
                    .next()
                    .unwrap_or(&name_str)
                    .trim_start_matches('+')
                    .trim_end_matches('=');
                let vals: Vec<String> = self
                    .plusargs
                    .iter()
                    .filter(|(k, _)| k.as_str() == base)
                    .map(|(_, v)| v.clone())
                    .collect();
                let n = vals.len();
                self.uvm_cmdline_values = vals;
                Ok(LogicVec::from_u64(n as u64, 32))
            }
            "get_arg_value_out" => {
                // Baca nilai yang disimpan get_arg_value (ref out di SV tidak
                // bisa langsung ditulis; test membaca via method ini).
                Ok(string_to_logicvec(&self.uvm_cmdline_last_value))
            }
            _ => Err(self.diag_error(
                DiagCode::NotImplemented,
                format!("unknown uvm_cmdline_processor method: {}", method),
            )),
        }
    }
}
