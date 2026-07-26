//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan util.rs (SRP Refactoring).
//! Tanggung jawab: Type checking utilities + type spec string parsing.
//!
//! Fungsi:
//!   - is_2state_type()      — cek apakah tipe data adalah 2-state
//!   - is_signed_type()      — cek apakah tipe data adalah signed
//!   - parse_type_spec_str() — parse string tipe spec ke DataType
//! ──────────────────────────────────────────────────────────────────────────────

use crate::ast::*;

/// Cek apakah tipe data termasuk 2-state (tidak memiliki X/Z).
/// Tipe 2-state: bit, byte, shortint, int, longint, time.
pub fn is_2state_type(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::Bit
            | DataType::Byte
            | DataType::Shortint
            | DataType::Int
            | DataType::Longint
            | DataType::Time
    )
}

/// Cek apakah tipe data adalah signed.
pub fn is_signed_type(dtype: &DataType) -> bool {
    matches!(dtype, DataType::Signed(_))
}

/// Parse string tipe spec (misal: "bit", "logic", "signed int") ke DataType.
/// Digunakan oleh width.rs dan eval.rs untuk resolusi tipe dari string literal.
pub fn parse_type_spec_str(s: &str) -> Option<DataType> {
    match s {
        "bit" => Some(DataType::Bit),
        "logic" => Some(DataType::Logic),
        "int" => Some(DataType::Int),
        "integer" => Some(DataType::Integer),
        "byte" => Some(DataType::Byte),
        "shortint" => Some(DataType::Shortint),
        "longint" => Some(DataType::Longint),
        "time" => Some(DataType::Time),
        "real" => Some(DataType::Real),
        "realtime" => Some(DataType::Realtime),
        "string" => Some(DataType::String),
        _ => {
            // Check for 'signed <type>' pattern
            if let Some(inner) = s.strip_prefix("signed ") {
                parse_type_spec_str(inner).map(|dt| DataType::Signed(Box::new(dt)))
            } else {
                None
            }
        }
    }
}
