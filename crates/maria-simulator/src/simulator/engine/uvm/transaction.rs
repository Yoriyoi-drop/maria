//! Transaction recording UVM (VERIF-17/18/19).
//!
//! - VERIF-17: `uvm_transaction`/`uvm_sequence_item` — `begin_tr(name)` /
//!   `end_tr()` mencatat transaksi dengan waktu sim (`state.time`) ke
//!   `engine.tr_records`. `set_stream(name)` menempelkan transaksi ke stream.
//! - VERIF-18: `uvm_tr_database` — singleton `get_db()` (id disimpan di
//!   `uvm_tr_db_id`), `get_stream(name)` create/reuse stream, `get_tr_count()`,
//!   `set_stream(name)` menetapkan stream default db.
//! - VERIF-19: `uvm_tr_stream` — objek stream per-nama; `record(tr)` menempel
//!   transaksi ke stream, `get_tr_count()` menghitung record stream.
//!
//! 1 file = 1 tanggung jawab: hanya transaction recording.

use super::super::SimulationEngine;
use crate::simulator::types::UvmTrRecord;
use crate::simulator::util::logicvec_to_string;
use maria_compiler::hir::{LogicVec, ObjId};
use maria_core::diagnostics::DiagCode;
use maria_core::error::SimError;
use maria_core::Symbol;

impl SimulationEngine {
    /// VERIF-18: stream obj id untuk nama (create kalau belum ada) + reverse map.
    pub(crate) fn tr_stream_get(&mut self, stream_name: &str) -> ObjId {
        if let Some(id) = self.uvm_tr_streams.get(stream_name) {
            return *id;
        }
        let id = self.state.alloc_object(Symbol::intern("uvm_tr_stream"));
        self.uvm_tr_streams.insert(stream_name.to_string(), id);
        self.tr_stream_names.insert(id, stream_name.to_string());
        id
    }

    /// VERIF-18: nama stream untuk obj transaksi — stream sendiri (set_stream)
    /// > stream default db > None.
    pub(crate) fn tr_stream_for_obj(&self, obj_id: ObjId) -> Option<String> {
        self.tr_obj_stream
            .get(&obj_id)
            .cloned()
            .or_else(|| self.tr_db_default_stream.clone())
    }

    /// Method `uvm_transaction`/`uvm_sequence_item` — rekam transaksi.
    pub(crate) fn execute_uvm_tr_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            // VERIF-17: mulai rekaman transaksi — catat waktu + stream saat ini.
            "begin_tr" => {
                let name = args
                    .first()
                    .map(logicvec_to_string)
                    .unwrap_or_else(|| "transaction".to_string());
                let stream = self.tr_stream_for_obj(obj_id);
                let idx = self.tr_records.len();
                self.tr_records.push(UvmTrRecord {
                    name: name.clone(),
                    obj_id,
                    stream,
                    start_time: self.state.time,
                    end_time: None,
                });
                self.tr_open.insert(obj_id, (idx, name));
                Ok(LogicVec::from_u64(0, 64))
            }
            // VERIF-17: tutup rekaman transaksi (set end_time = waktu sekarang).
            "end_tr" => {
                if let Some((idx, _)) = self.tr_open.remove(&obj_id) {
                    if let Some(rec) = self.tr_records.get_mut(idx) {
                        rec.end_time = Some(self.state.time);
                    }
                } else {
                    self.emit_warning(
                        DiagCode::DpiError,
                        format!("end_tr: no open begin_tr for obj_id={}", obj_id),
                    );
                }
                Ok(LogicVec::from_u64(0, 64))
            }
            // VERIF-19: tempelkan transaksi ini ke stream bernama.
            "set_stream" => {
                let Some(name) = args.first() else {
                    return Ok(LogicVec::from_u64(0, 64));
                };
                let stream_name = logicvec_to_string(name);
                self.tr_stream_get(&stream_name);
                self.tr_obj_stream.insert(obj_id, stream_name);
                Ok(LogicVec::from_u64(0, 64))
            }
            other => {
                self.emit_warning(
                    DiagCode::DpiError,
                    format!("uvm_transaction: unknown method '{}'", other),
                );
                Ok(LogicVec::from_u64(0, 64))
            }
        }
    }

    /// Method `uvm_tr_database` — singleton database + akses stream/record.
    pub(crate) fn execute_uvm_tr_db_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            // VERIF-18: singleton — semua get_db() → obj id sama.
            "get_db" | "new" => {
                let id = if self.uvm_tr_db_id.is_none() {
                    self.uvm_tr_db_id = Some(obj_id);
                    obj_id
                } else {
                    self.uvm_tr_db_id.unwrap()
                };
                Ok(LogicVec::from_u64(id as u64, 64))
            }
            // VERIF-18: get_stream(name) — create/reuse stream obj id.
            "get_stream" => {
                let stream_name = args.first().map(logicvec_to_string).unwrap_or_default();
                let id = self.tr_stream_get(&stream_name);
                Ok(LogicVec::from_u64(id as u64, 64))
            }
            // VERIF-18: get_tr_count() — jumlah record (semua stream).
            "get_tr_count" => Ok(LogicVec::from_u64(self.tr_records.len() as u64, 64)),
            // VERIF-18: set_stream(name) — stream default db utk begin_tr baru.
            "set_stream" => {
                let stream_name = args.first().map(logicvec_to_string).unwrap_or_default();
                self.tr_stream_get(&stream_name);
                self.tr_db_default_stream = Some(stream_name);
                Ok(LogicVec::from_u64(0, 64))
            }
            other => {
                self.emit_warning(
                    DiagCode::DpiError,
                    format!("uvm_tr_database: unknown method '{}'", other),
                );
                Ok(LogicVec::from_u64(0, 64))
            }
        }
    }

    /// Method `uvm_tr_stream` — stream transaksi (record + count).
    pub(crate) fn execute_uvm_tr_stream_method(
        &mut self,
        obj_id: ObjId,
        method: &str,
        args: &[LogicVec],
    ) -> Result<LogicVec, SimError> {
        match method {
            "new" => {
                // register stream by name (arg pertama) bila belum ada.
                if let Some(n) = args.first() {
                    let name = logicvec_to_string(n);
                    if !self.uvm_tr_streams.contains_key(&name) {
                        self.uvm_tr_streams.insert(name.clone(), obj_id);
                        self.tr_stream_names.insert(obj_id, name);
                    }
                }
                Ok(LogicVec::from_u64(obj_id as u64, 64))
            }
            // VERIF-19: record(tr_handle) — tempel transaksi ke stream ini.
            "record" => {
                let Some(tr) = args.first() else {
                    return Ok(LogicVec::from_u64(0, 64));
                };
                let tr_obj = tr.to_u64() as ObjId;
                let stream_name = self.tr_stream_names.get(&obj_id).cloned();
                if let Some(name) = stream_name {
                    self.tr_obj_stream.insert(tr_obj, name);
                }
                Ok(LogicVec::from_u64(0, 64))
            }
            // VERIF-19: get_tr_count() — record milik stream ini.
            "get_tr_count" => {
                let stream_name = self.tr_stream_names.get(&obj_id).cloned();
                let n = self
                    .tr_records
                    .iter()
                    .filter(|r| r.stream.is_some() && r.stream == stream_name)
                    .count();
                Ok(LogicVec::from_u64(n as u64, 64))
            }
            other => {
                self.emit_warning(
                    DiagCode::DpiError,
                    format!("uvm_tr_stream: unknown method '{}'", other),
                );
                Ok(LogicVec::from_u64(0, 64))
            }
        }
    }

    /// VERIF-17: ringkasan record transaksi (total + per stream + terbuka).
    pub(crate) fn report_tr_records(&self) -> String {
        let mut out = String::new();
        let total = self.tr_records.len();
        let open = self
            .tr_records
            .iter()
            .filter(|r| r.end_time.is_none())
            .count();
        out.push_str(&format!(
            "Transactions: {} recorded ({} open)\n",
            total, open
        ));
        let mut per_stream: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for r in &self.tr_records {
            let key = r.stream.clone().unwrap_or_else(|| "<none>".to_string());
            *per_stream.entry(key).or_insert(0) += 1;
        }
        for (stream, n) in per_stream {
            out.push_str(&format!("  stream '{}': {} records\n", stream, n));
        }
        out
    }
}
