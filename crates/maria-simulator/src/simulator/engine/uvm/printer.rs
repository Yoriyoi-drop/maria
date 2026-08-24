//! Layer printer UVM (VERIF-12): `uvm_printer` / `uvm_table_printer` —
//! memformat object (nama, class, fields) menjadi string tabel. Method
//! `print_object(obj)` mengembalikan string (dipakai `$display("%s", s)`),
//! `uvm_object::print(printer)` mendelegasikan ke printer bila argumen
//! printer diberikan. 1 file = 1 tanggung jawab: hanya printer layer.
//! https://verificationacademy.com/verification-methodology-reference/uvm-methodology-reference/uvm_printer/

use super::super::SimulationEngine;
use crate::simulator::util::*;
use maria_core::error::SimError;
use maria_ir::*;

impl SimulationEngine {
    /// Execute uvm_printer / uvm_table_printer method.
    /// `new(name)` menyimpan nama object; `print_object(obj_handle)`
    /// memformat fields object menjadi string tabel (nama, class, width,
    /// nilai per field) — mirip uvm_table_printer::print_object.
    pub(crate) fn execute_uvm_printer_method(
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
                self.uvm_object_data.insert(
                    obj_id,
                    crate::simulator::types::UvmObjectData { name: name.clone() },
                );
                Ok(LogicVec::from_u64(1, 1))
            }
            // print_object(uvm_object obj) → String (format tabel).
            "print_object" => {
                let target = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let s = self.format_uvm_object_table(target);
                Ok(string_to_logicvec(&s))
            }
            _ => self.execute_uvm_object_method(obj_id, method, args),
        }
    }

    /// Format object menjadi string tabel (VERIF-12): baris header + satu
    /// baris per field (nama, width, nilai). Nilai di-print sebagai hex
    /// bila width > 1, biner bila 1-bit. Handle null (0) → string kosong.
    pub(crate) fn format_uvm_object_table(&self, obj_id: ObjId) -> String {
        let Some(obj) = self.state.get_object(obj_id) else {
            return String::new();
        };
        let class_name = obj.class_name.as_str();
        let name = self
            .uvm_object_data
            .get(&obj_id)
            .map(|d| d.name.as_str())
            .unwrap_or("unnamed");
        let mut out = String::new();
        out.push_str(&format!(
            "Name               Type                 Size          Value\n\
             ----------------------------------------------------------------\n"
        ));
        out.push_str(&format!(
            "{:<18} {:<20} {:<13} @{}\n",
            name, class_name, "-", obj_id
        ));
        // Fields: iterasi deterministik (sorted) agar output stabil.
        let mut fields: Vec<(String, &LogicVec)> = obj
            .fields
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v))
            .collect();
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        for (fname, val) in fields {
            let value_str = if val.width <= 1 {
                match val.bits.first() {
                    Some(LogicVal::Zero) => "0".to_string(),
                    Some(LogicVal::One) => "1".to_string(),
                    Some(LogicVal::X) => "x".to_string(),
                    Some(LogicVal::Z) => "z".to_string(),
                    None => "-".to_string(),
                }
            } else {
                format!("'h{:x}", val.to_u64())
            };
            out.push_str(&format!(
                "{:<18} {:<20} {:<13} {}\n",
                fname, "integral", val.width, value_str
            ));
        }
        out
    }
}
