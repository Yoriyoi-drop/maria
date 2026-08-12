//! Tipe nilai logika 4-state (X/Z/0/1) — fondasi seluruh simulator.
//!
//! Dipindah dari `src/ir/ir.rs` (migrasi monorepo: src/ → crates/) karena
//! dipakai lintas crate (ast `FillLit`, ir, simulator, waveform, hir) dan
//! memutus siklus dependensi ast ↔ ir: `ast` kini memakai `maria_core::LogicVal`,
//! `maria-ir` me-reexport dari sini agar `maria_ir::LogicVec` tetap valid.

use std::cell::RefCell;

/// Signature untuk allocator LogicVec custom (arena-backed).
pub type LogicVecCtor = fn(usize, LogicVal) -> Option<LogicVec>;

thread_local! {
    /// Custom constructor yang di-register oleh SimulationArena (atau None utk heap default).
    static LOGICVEC_CTOR: RefCell<Option<LogicVecCtor>> = const { RefCell::new(None) };
}

/// Register constructor LogicVec custom (mis. alokasi dari arena).
/// Berikan `None` untuk kembali ke heap allocation default.
pub fn set_logicvec_ctor(ctor: Option<LogicVecCtor>) {
    LOGICVEC_CTOR.with(|cell| *cell.borrow_mut() = ctor);
}

fn get_logicvec_ctor() -> Option<LogicVecCtor> {
    LOGICVEC_CTOR.with(|cell| *cell.borrow())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LogicVec {
    pub bits: Vec<LogicVal>,
    pub width: usize,
}

impl Default for LogicVec {
    fn default() -> Self {
        LogicVec::new(1)
    }
}

impl LogicVec {
    pub fn new(width: usize) -> Self {
        let w = if width > 1_000_000 { 1 } else { width };
        // Try arena-backed allocation first (zero-deallocation path)
        if let Some(ctor) = get_logicvec_ctor() {
            if let Some(lv) = ctor(w, LogicVal::X) {
                return lv;
            }
        }
        // Fallback: standard heap allocation
        LogicVec {
            bits: vec![LogicVal::X; w],
            width: w,
        }
    }

    pub fn fill(val: LogicVal, width: usize) -> Self {
        // Try arena-backed allocation first (zero-deallocation path)
        if let Some(ctor) = get_logicvec_ctor() {
            if let Some(lv) = ctor(width, val) {
                return lv;
            }
        }
        // Fallback: standard heap allocation
        LogicVec {
            bits: vec![val; width],
            width,
        }
    }

    pub fn from_u64(val: u64, width: usize) -> Self {
        // Try arena-backed allocation first (zero-deallocation path)
        if let Some(ctor) = get_logicvec_ctor() {
            if let Some(mut lv) = ctor(width, LogicVal::Zero) {
                for i in 0..lv.width.min(64) {
                    if (val >> i) & 1 == 1 {
                        lv.bits[i] = LogicVal::One;
                    }
                }
                return lv;
            }
        }
        // Fallback: standard heap allocation
        let mut bits = Vec::with_capacity(width);
        for i in 0..width {
            if i < 64 && (val >> i) & 1 == 1 {
                bits.push(LogicVal::One);
            } else {
                bits.push(LogicVal::Zero);
            }
        }
        LogicVec { bits, width }
    }

    pub fn to_u64(&self) -> u64 {
        let mut result = 0u64;
        for i in 0..self.width.min(64) {
            if self.bits[i] == LogicVal::One {
                result |= 1 << i;
            }
        }
        result
    }

    pub fn to_i64(&self) -> i64 {
        let uval = self.to_u64();
        if self.width < 64 {
            let mask = 1u64 << (self.width - 1);
            if uval & mask != 0 {
                (uval | (!0u64 << self.width)) as i64
            } else {
                uval as i64
            }
        } else {
            uval as i64
        }
    }

    pub fn to_bool(&self) -> Option<bool> {
        if self.width == 0 {
            return Some(false);
        }
        let all_x_or_z = self
            .bits
            .iter()
            .all(|b| *b == LogicVal::X || *b == LogicVal::Z);
        if all_x_or_z {
            return None;
        }
        let any_one = self.bits.contains(&LogicVal::One);
        // In Verilog, X/Z in a conditional is treated as false
        let any_zero_or_x_or_z = self.bits.contains(&LogicVal::Zero);
        Some(any_one && (!any_zero_or_x_or_z || any_one))
    }

    pub fn resize(&self, new_width: usize) -> Self {
        if new_width <= self.width {
            // Try arena-backed allocation
            if let Some(ctor) = get_logicvec_ctor() {
                if let Some(mut lv) = ctor(new_width, LogicVal::Zero) {
                    lv.bits[..new_width].copy_from_slice(&self.bits[..new_width]);
                    return lv;
                }
            }
            let mut bits = self.bits.clone();
            bits.truncate(new_width);
            return LogicVec {
                bits,
                width: new_width,
            };
        }
        // Try arena-backed allocation
        if let Some(ctor) = get_logicvec_ctor() {
            if let Some(mut lv) = ctor(new_width, LogicVal::Zero) {
                lv.bits[..self.width].copy_from_slice(&self.bits);
                return lv;
            }
        }
        let mut bits = self.bits.clone();
        bits.resize(new_width, LogicVal::Zero);
        LogicVec {
            bits,
            width: new_width,
        }
    }

    pub fn extend(&self, other: &LogicVec) -> Self {
        let new_width = self.width + other.width;
        // Try arena-backed allocation
        if let Some(ctor) = get_logicvec_ctor() {
            if let Some(mut lv) = ctor(new_width, LogicVal::Zero) {
                lv.bits[..self.width].copy_from_slice(&self.bits);
                lv.bits[self.width..].copy_from_slice(&other.bits);
                return lv;
            }
        }
        let mut bits = self.bits.clone();
        bits.extend_from_slice(&other.bits);
        LogicVec {
            bits,
            width: new_width,
        }
    }

    pub fn from_hex(hex_str: &str) -> Result<Self, String> {
        let hex = hex_str.trim_start_matches("0x").trim_start_matches("0X");
        let num_bits = hex.len() * 4;
        let val = u64::from_str_radix(hex, 16)
            .map_err(|e| format!("invalid hex '{}': {}", hex_str, e))?;
        Ok(LogicVec::from_u64(val, num_bits.max(1)))
    }

    pub fn from_bin(bin_str: &str) -> Result<Self, String> {
        let bin = bin_str.trim_start_matches("0b").trim_start_matches("0B");
        let num_bits = bin.len();
        let val = u64::from_str_radix(bin, 2)
            .map_err(|e| format!("invalid binary '{}': {}", bin_str, e))?;
        Ok(LogicVec::from_u64(val, num_bits.max(1)))
    }

    pub fn all_x(&self) -> bool {
        self.bits.iter().all(|b| *b == LogicVal::X)
    }

    pub fn all_z(&self) -> bool {
        self.bits.iter().all(|b| *b == LogicVal::Z)
    }

    pub fn casex_eq(&self, other: &LogicVec) -> bool {
        for i in 0..self.width.max(other.width) {
            let val = self.bits.get(i).copied().unwrap_or(LogicVal::Zero);
            let pat = other.bits.get(i).copied().unwrap_or(LogicVal::Zero);
            // In casex: X or Z in the pattern are don't-care (match anything)
            if pat == LogicVal::X || pat == LogicVal::Z {
                continue;
            }
            if val != pat {
                return false;
            }
        }
        true
    }

    pub fn casez_eq(&self, other: &LogicVec) -> bool {
        for i in 0..self.width.max(other.width) {
            let val = self.bits.get(i).copied().unwrap_or(LogicVal::Zero);
            let pat = other.bits.get(i).copied().unwrap_or(LogicVal::Zero);
            // In casez: Z in the pattern is don't-care (match anything)
            if pat == LogicVal::Z {
                continue;
            }
            if val != pat {
                return false;
            }
        }
        true
    }

    /// LRM 1800-2017 §12.5.1: operands case `case` dibandingkan dengan lebar
    /// operand TERLEBAR — yang lebih sempit di-zero-extend dulu. `PartialEq`
    /// (derived) membandingkan `bits` DAN `width` sehingga `6'd1` (width 32
    /// bila lebar literal hilang) tidak pernah cocok dengan signal 6-bit.
    /// Method ini membandingkan NILAI dengan zero-extension ke max width.
    pub fn case_val_eq(&self, other: &LogicVec) -> bool {
        for i in 0..self.width.max(other.width) {
            let val = self.bits.get(i).copied().unwrap_or(LogicVal::Zero);
            let pat = other.bits.get(i).copied().unwrap_or(LogicVal::Zero);
            if val != pat {
                return false;
            }
        }
        true
    }

    pub fn case_eq(&self, other: &LogicVec) -> LogicVec {
        let eq = self.bits == other.bits;
        LogicVec::from_u64(if eq { 1 } else { 0 }, 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum LogicVal {
    Zero,
    One,
    X,
    Z,
}

impl std::fmt::Display for LogicVal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogicVal::Zero => write!(f, "0"),
            LogicVal::One => write!(f, "1"),
            LogicVal::X => write!(f, "x"),
            LogicVal::Z => write!(f, "z"),
        }
    }
}

impl std::fmt::Display for LogicVec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for bit in self.bits.iter().rev() {
            write!(f, "{}", bit)?;
        }
        Ok(())
    }
}
