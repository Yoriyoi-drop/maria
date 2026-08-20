//! VHPI Value System (IEEE 1076-2008 §C.5.9) — konversi nilai HDL.
//!
//! Representasi internal Maria tetap `LogicVec` (2-bit value/known implicit
//! per bit: 0/1/X/Z). VHPI format (`vhpiIntVal`, `vhpiLogicVal`,
//! `vhpiBinStrVal`, ...) dikonversi ke/dari `LogicVec` di sini — satu
//! representasi untuk semua foreign interface (arsitektur poin 7).
#![allow(non_upper_case_globals)]

use maria_ir::{LogicVal, LogicVec};
use std::os::raw::c_char;

// ─── Value Formats (IEEE 1076-2008) ───

pub const vhpiBinStrVal: i32 = 1;
pub const vhpiOctStrVal: i32 = 2;
pub const vhpiDecStrVal: i32 = 3;
pub const vhpiHexStrVal: i32 = 4;
pub const vhpiLogicVal: i32 = 5;
pub const vhpiIntVal: i32 = 6;
pub const vhpiRealVal: i32 = 7;
pub const vhpiEnumVal: i32 = 8;
pub const vhpiTimeVal: i32 = 9;
pub const vhpiStrVal: i32 = 10;
pub const vhpiVectorVal: i32 = 11;
pub const vhpiObjTypeVal: i32 = 12;

// ─── Struct nilai VHPI (C-compatible, pola vpi t_vpi_value) ───

#[repr(C)]
#[derive(Debug, Clone)]
pub struct t_vhpi_value {
    pub format: i32,
    pub value: vhpi_value_union,
}

#[repr(C)]
#[derive(Clone)]
pub union vhpi_value_union {
    pub logic: u8,
    pub int: i32,
    pub real: f64,
    pub str: *mut c_char,
    pub vector: t_vhpi_vector,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct t_vhpi_vector {
    pub length: i32,
    pub aval: *mut u32,
    pub bval: *mut u32,
}

impl std::fmt::Debug for vhpi_value_union {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "vhpi_value_union {{ ... }}")
    }
}
unsafe impl Send for vhpi_value_union {}
unsafe impl Sync for vhpi_value_union {}
impl Copy for vhpi_value_union {}
unsafe impl Send for t_vhpi_value {}
unsafe impl Sync for t_vhpi_value {}

impl Default for t_vhpi_value {
    fn default() -> Self {
        t_vhpi_value {
            format: vhpiIntVal,
            value: vhpi_value_union { int: 0 },
        }
    }
}

/// C-compatible struct waktu VHPI (IEEE 1076-2008 §C.5.10).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct t_vhpi_time {
    pub time_type: i32,
    pub low: u32,
    pub high: u32,
    pub real: f64,
}

impl t_vhpi_time {
    pub fn new_sim_time() -> Self {
        t_vhpi_time { time_type: 0, low: 0, high: 0, real: 0.0 }
    }
}

/// Konversi LogicVec → nilai VHPI sesuai format. (Pola vpi/value.rs.)
pub fn logicvec_to_vhpi(lv: &LogicVec, format: i32) -> t_vhpi_value {
    match format {
        vhpiLogicVal => t_vhpi_value {
            format,
            value: vhpi_value_union {
                logic: match lv.bits.first().copied().unwrap_or(LogicVal::X) {
                    LogicVal::Zero => b'0',
                    LogicVal::One => b'1',
                    LogicVal::X => b'X',
                    LogicVal::Z => b'Z',
                },
            },
        },
        vhpiIntVal => t_vhpi_value {
            format,
            value: vhpi_value_union { int: lv.to_u64() as i32 },
        },
        vhpiRealVal => t_vhpi_value {
            format,
            value: vhpi_value_union {
                real: if lv.width == 64 {
                    f64::from_bits(lv.to_u64())
                } else {
                    lv.to_u64() as f64
                },
            },
        },
        _ => t_vhpi_value {
            format,
            value: vhpi_value_union { int: 0 },
        },
    }
}

/// Konversi nilai VHPI (dibaca dari pointer) → LogicVec.
///
/// # Safety
/// `value` harus menunjuk t_vhpi_value valid (format sesuai union).
pub unsafe fn vhpi_to_logicvec(value: *const t_vhpi_value) -> LogicVec {
    if value.is_null() {
        return LogicVec::from_u64(0, 32);
    }
    let v = &*value;
    match v.format {
        vhpiIntVal => LogicVec::from_u64(v.value.int as u32 as u64, 32),
        vhpiLogicVal => {
            let l = v.value.logic as char;
            let b = match l {
                '0' => LogicVal::Zero,
                '1' => LogicVal::One,
                'Z' => LogicVal::Z,
                _ => LogicVal::X,
            };
            LogicVec { width: 1, bits: vec![b] }
        }
        vhpiRealVal => LogicVec::from_u64(v.value.real as u64, 64),
        vhpiStrVal | vhpiBinStrVal => {
            if v.value.str.is_null() {
                LogicVec::from_u64(0, 32)
            } else {
                let s = std::ffi::CStr::from_ptr(v.value.str).to_string_lossy().to_string();
                binstr_to_logicvec(&s)
            }
        }
        _ => LogicVec::from_u64(0, 32),
    }
}

/// Parse string biner ("1010xZ") → LogicVec. Bit pertama string = MSB.
pub fn binstr_to_logicvec(s: &str) -> LogicVec {
    let bits: Vec<LogicVal> = s
        .chars()
        .filter(|c| *c != '_')
        .map(|c| match c {
            '0' => LogicVal::Zero,
            '1' => LogicVal::One,
            'z' | 'Z' => LogicVal::Z,
            _ => LogicVal::X,
        })
        .collect();
    // bits[0] = MSB → balik agar bits[0] = LSB (konvensi LogicVec).
    let mut rev = bits.clone();
    rev.reverse();
    LogicVec { width: rev.len(), bits: rev }
}

/// LogicVec → string biner MSB-first ("1010").
pub fn logicvec_to_binstr(lv: &LogicVec) -> String {
    lv.bits
        .iter()
        .rev()
        .map(|b| match b {
            LogicVal::Zero => '0',
            LogicVal::One => '1',
            LogicVal::X => 'x',
            LogicVal::Z => 'z',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logicvec_to_vhpi_int() {
        let lv = LogicVec::from_u64(0x2A, 8);
        let v = logicvec_to_vhpi(&lv, vhpiIntVal);
        assert_eq!(v.format, vhpiIntVal);
        assert_eq!(unsafe { v.value.int }, 0x2A);
    }

    #[test]
    fn test_logicvec_to_vhpi_logic() {
        let lv = LogicVec { width: 1, bits: vec![LogicVal::One] };
        let v = logicvec_to_vhpi(&lv, vhpiLogicVal);
        assert_eq!(unsafe { v.value.logic } as char, '1');
        let lvz = LogicVec { width: 1, bits: vec![LogicVal::Z] };
        let vz = logicvec_to_vhpi(&lvz, vhpiLogicVal);
        assert_eq!(unsafe { vz.value.logic } as char, 'Z');
    }

    #[test]
    fn test_binstr_roundtrip() {
        // "1011" MSB-first = 0b1011 = 11. LogicVec bits[0] = LSB = 1.
        let lv = binstr_to_logicvec("1011");
        assert_eq!(lv.to_u64(), 11);
        assert_eq!(logicvec_to_binstr(&lv), "1011");
        // underscore diabaikan.
        let lv2 = binstr_to_logicvec("10_11");
        assert_eq!(lv2.to_u64(), 11);
        // x/z — "1x" MSB-first → setelah reverse bits[0]=LSB=x
        let lvx = binstr_to_logicvec("1x");
        assert_eq!(lvx.bits[0], LogicVal::X);
        assert_eq!(lvx.bits[1], LogicVal::One);
    }

    #[test]
    fn test_vhpi_to_logicvec_int_ptr() {
        let v = t_vhpi_value {
            format: vhpiIntVal,
            value: vhpi_value_union { int: 42 },
        };
        let lv = unsafe { vhpi_to_logicvec(&v) };
        assert_eq!(lv.to_u64(), 42);
        assert_eq!(lv.width, 32);
    }

    #[test]
    fn test_vhpi_to_logicvec_null() {
        let lv = unsafe { vhpi_to_logicvec(std::ptr::null()) };
        assert_eq!(lv.to_u64(), 0);
    }
}
