//! uvm_comparator / uvm_in_order_comparator (VERIF-13).
//! Komponen pembanding transaksi TLM in-order: `analysis_imp` menerima
//! transaksi ACTUAL (`ap.write(actual)` → imp → comparator.write), antrian
//! expected diisi `write_expected` (atau imp `analysis_imp_expected`).
//! `write` membandingkan actual dgn head antrian expected (in-order) dan
//! meng-increment counter `matches`/`mismatches`. Perbandingan memakai
//! method `compare` user pada transaksi bila ada; fallback: kesetaraan
//! field object. `get_match_count()` / `get_mismatch_count()` membaca
//! counter. 1 file = 1 tanggung jawab: comparator TLM.

use super::super::SimulationEngine;
use maria_core::error::SimError;
use maria_compiler::hir::{LogicVec, ObjId};
use crate::simulator::types::*;
use crate::simulator::util::*;
use maria_core::Symbol;

impl SimulationEngine {
    /// Method builtin `uvm_comparator` / `uvm_in_order_comparator`.
    pub(crate) fn execute_uvm_comparator_method(
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
                    .unwrap_or_else(|| format!("comparator_{}", obj_id));
                self.uvm_object_data
                    .entry(obj_id)
                    .or_insert_with(|| UvmObjectData { name: name.clone() });
                // Analysis-imp internal (ACTUAL): `mon.ap.connect(comp.analysis_imp)`
                // → `ap.write(item)` → imp → parent.write (di sini).
                let imp_name = format!("{}_imp", if name.is_empty() { "comp" } else { &name });
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
                self.uvm_comparator_data.insert(
                    obj_id,
                    UvmComparatorData {
                        expected: std::collections::VecDeque::new(),
                        matches: 0,
                        mismatches: 0,
                    },
                );
                Ok(LogicVec::from_u64(1, 1))
            }
            // write(actual): pop expected head, bandingkan, increment counter.
            "write" => {
                let actual = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let mut cdata = self
                    .uvm_comparator_data
                    .get(&obj_id)
                    .cloned()
                    .unwrap_or(UvmComparatorData {
                        expected: std::collections::VecDeque::new(),
                        matches: 0,
                        mismatches: 0,
                    });
                match cdata.expected.pop_front() {
                    None => {
                        // Tidak ada expected → mismatch (seperti UVM asli:
                        // compare dgn transaksi kosong gagal).
                        cdata.mismatches += 1;
                        let msg = format!(
                            "uvm_comparator[{}] mismatch: no expected transaction for actual {}",
                            obj_id, actual
                        );
                        self.emit_severity("error", &format!("[UVM_COMPARATOR] {}", msg));
                    }
                    Some(expected) => {
                        if self.compare_transactions(actual, expected)? {
                            cdata.matches += 1;
                        } else {
                            cdata.mismatches += 1;
                            let msg = format!(
                                "uvm_comparator[{}] mismatch: actual {} vs expected {}",
                                obj_id, actual, expected
                            );
                            self.emit_severity("error", &format!("[UVM_COMPARATOR] {}", msg));
                        }
                    }
                }
                self.uvm_comparator_data.insert(obj_id, cdata);
                Ok(LogicVec::from_u64(1, 1))
            }
            // write_expected(exp): push ke antrian expected (in-order).
            "write_expected" => {
                let expected = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                if let Some(cdata) = self.uvm_comparator_data.get_mut(&obj_id) {
                    cdata.expected.push_back(expected);
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_match_count" => {
                let c = self
                    .uvm_comparator_data
                    .get(&obj_id)
                    .map(|d| d.matches)
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(c, 64))
            }
            "get_mismatch_count" => {
                let c = self
                    .uvm_comparator_data
                    .get(&obj_id)
                    .map(|d| d.mismatches)
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(c, 64))
            }
            _ => self.execute_uvm_component_method(obj_id, method, args),
        }
    }

    /// Bandingkan dua transaksi (obj id): pakai method `compare` user pada
    /// transaksi actual bila didefinisikan (UVM asli: `compare(T rhs)`
    /// virtual di transaksi); fallback: kesetaraan seluruh field object.
    fn compare_transactions(&mut self, actual: ObjId, expected: ObjId) -> Result<bool, SimError> {
        // compare() user di transaksi actual → hasil 1/0 (pola UVM asli).
        if let Some(obj) = self.state.get_object(actual) {
            let class = obj.class_name.as_str();
            if self.find_method_quiet(class, "compare").is_some() {
                let args = vec![LogicVec::from_u64(expected as u64, 64)];
                let r = self.execute_method(actual, "compare", &args)?;
                return Ok(r.to_u64() != 0);
            }
        }
        // Fallback: field object harus identik (nama + nilai).
        let a_fields = self
            .state
            .get_object(actual)
            .map(|o| o.fields.clone())
            .unwrap_or_default();
        let e_fields = self
            .state
            .get_object(expected)
            .map(|o| o.fields.clone())
            .unwrap_or_default();
        if a_fields.len() != e_fields.len() {
            return Ok(false);
        }
        for (k, v) in &a_fields {
            match e_fields.get(k) {
                Some(ev) if ev == v => {}
                _ => return Ok(false),
            }
        }
        Ok(true)
    }
}
