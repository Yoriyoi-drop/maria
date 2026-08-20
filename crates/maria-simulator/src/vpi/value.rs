//! VPI Value Access — get_value / put_value with format conversion.
//!
//! Converts between VPI value formats (IntVal, VectorVal, BinStrVal, etc.)
//! and internal LogicVec representation.
#![allow(non_upper_case_globals)]

use super::handle::*;
use super::types::*;
use maria_ir::*;

/// vpi_get_value(handle, value_p) — read the current value of a signal/object.
pub fn vpi_get_value(handle: vpiHandle, value_p: &mut t_vpi_value) -> i32 {
    let obj = match vpi_lookup_object(handle) {
        Some(o) => o,
        None => return 0,
    };
    let (sig_id, sig_width) = match &obj.kind {
        VpiObjectKind::Signal(sig_id, _) => (*sig_id, {
            super::with_vpi_engine(|engine| {
                engine.design.top.signals.get(*sig_id).map(|s| s.width)
            }).flatten().unwrap_or(1)
        }),
        _ => return 0,
    };
    let _width = sig_width;
    let logic_val_opt = super::with_vpi_engine(|engine| {
        engine.state.read_signal(sig_id).clone()
    });
    let logic_val = match logic_val_opt {
        Some(v) => v,
        None => return 0,
    };
    match value_p.format {
        vpiIntVal => {
            let val = logic_val.to_u64() as i32;
            value_p.value = vpi_value_union { integer: val };
        }
        vpiScalarVal => {
            let scalar = match logic_val.bits.first().copied().unwrap_or(LogicVal::X) {
                LogicVal::Zero => 0,
                LogicVal::One => 1,
                LogicVal::X => 2,
                LogicVal::Z => 3,
            };
            value_p.value = vpi_value_union { scalar };
        }
        vpiVectorVal => {
            let val = logic_val.to_u64();
            let aval = val as u32;
            let bval = 0u32; // No Z/X for now
            value_p.value = vpi_value_union {
                vector: t_vpi_vector { aval, bval },
            };
        }
        vpiBinStrVal => {
            let s = bin_str(&logic_val);
            let ptr = cache_cstring(&s);
            value_p.value = vpi_value_union {
                string: ptr,
            };
        }
        vpiHexStrVal => {
            let s = hex_str(&logic_val);
            let ptr = cache_cstring(&s);
            value_p.value = vpi_value_union {
                string: ptr,
            };
        }
        vpiDecStrVal => {
            let val = logic_val.to_u64();
            let s = val.to_string();
            let ptr = cache_cstring(&s);
            value_p.value = vpi_value_union {
                string: ptr,
            };
        }
        vpiRealVal => {
            let val = logic_val.to_u64() as f64;
            value_p.value = vpi_value_union { real: val };
        }
        _ => {}
    }
    1
}

/// vpi_put_value(handle, value_p, time_p, flags) — assign a value to a signal.
/// Uses direct write (vpiNoDelay semantics by default). The VPI engine is
/// registered during the simulation run, so writes take effect immediately
/// and are visible to the event loop in subsequent delta cycles.
pub fn vpi_put_value(handle: vpiHandle, value_p: &t_vpi_value, _time_p: *mut t_vpi_time, _flags: i32) -> i32 {
    let obj = match vpi_lookup_object(handle) {
        Some(o) => o,
        None => return 0,
    };
    let sig_id = match &obj.kind {
        VpiObjectKind::Signal(sig_id, _) => *sig_id,
        _ => return 0,
    };
    let new_val = value_to_logicvec(value_p);
    let result = super::with_vpi_engine(|engine| {
        let sig = engine.design.top.signals.get(sig_id);
        if let Some(sig_info) = sig {
            let width = sig_info.width;
            let val = if new_val.width != width {
                let mut bits = new_val.bits.clone();
                if bits.len() < width {
                    bits.resize(width, LogicVal::Zero);
                } else {
                    bits.truncate(width);
                }
                LogicVec { width, bits }
            } else {
                new_val.clone()
            };
            // Direct write (vpiNoDelay semantics)
            engine.state.write_signal(sig_id, val);
            1
        } else {
            0
        }
    });
    result.unwrap_or(0)
}

/// Convert a t_vpi_value to a LogicVec.
fn value_to_logicvec(value_p: &t_vpi_value) -> LogicVec {
    match value_p.format {
        vpiIntVal => {
            let val = unsafe { value_p.value.integer } as u64;
            LogicVec::from_u64(val, 32)
        }
        vpiScalarVal => {
            let scalar = unsafe { value_p.value.scalar };
            let bit = match scalar {
                0 => LogicVal::Zero,
                1 => LogicVal::One,
                2 => LogicVal::X,
                _ => LogicVal::Z,
            };
            LogicVec { bits: vec![bit], width: 1 }
        }
        vpiVectorVal => {
            let aval = unsafe { value_p.value.vector.aval };
            LogicVec::from_u64(aval as u64, 32)
        }
        vpiBinStrVal => {
            let s = unsafe { super::types::cstr_to_str(value_p.value.string) };
            let mut bits = Vec::with_capacity(s.len());
            for c in s.chars() {
                match c {
                    '0' => bits.push(LogicVal::Zero),
                    '1' => bits.push(LogicVal::One),
                    'x' | 'X' => bits.push(LogicVal::X),
                    'z' | 'Z' => bits.push(LogicVal::Z),
                    '_' => {} // skip underscores
                    _ => bits.push(LogicVal::X),
                }
            }
            bits.reverse();
            LogicVec { width: bits.len(), bits }
        }
        vpiHexStrVal => {
            let s = unsafe { super::types::cstr_to_str(value_p.value.string) };
            let hex = s.trim_start_matches("0x").trim_start_matches("0X");
            let val = u64::from_str_radix(hex, 16).unwrap_or(0);
            LogicVec::from_u64(val, hex.len() * 4)
        }
        vpiRealVal => {
            let val = unsafe { value_p.value.real };
            LogicVec::from_u64(val.to_bits(), 64)
        }
        _ => LogicVec::new(1),
    }
}

/// Format a LogicVec as a binary string (MSB-first, VPI style).
fn bin_str(lv: &LogicVec) -> String {
    let mut s = String::with_capacity(lv.width);
    for i in (0..lv.width).rev() {
        match lv.bits[i] {
            LogicVal::Zero => s.push('0'),
            LogicVal::One => s.push('1'),
            LogicVal::X => s.push('x'),
            LogicVal::Z => s.push('z'),
        }
    }
    s
}

/// Format a LogicVec as a hex string.
fn hex_str(lv: &LogicVec) -> String {
    let val = lv.to_u64();
    if lv.width <= 4 {
        format!("{:x}", val as u8)
    } else if lv.width <= 8 {
        format!("{:02x}", val as u8)
    } else if lv.width <= 16 {
        format!("{:04x}", val as u16)
    } else if lv.width <= 32 {
        format!("{:08x}", val as u32)
    } else {
        format!("{:016x}", val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::os::raw::c_char;

    #[test]
    fn test_vpi_value_bin_str() {
        // MSB-first, VPI style
        let lv = LogicVec::from_u64(0b1011, 4);
        assert_eq!(bin_str(&lv), "1011");
        let lv = LogicVec::from_u64(0xA5, 8);
        assert_eq!(bin_str(&lv), "10100101");
        let lv = LogicVec::from_u64(0, 1);
        assert_eq!(bin_str(&lv), "0");
    }

    #[test]
    fn test_vpi_value_hex_str() {
        // padding sesuai lebar (2 digit utk ≤8 bit, 4 utk ≤16, dst.)
        let lv = LogicVec::from_u64(0x2A, 8);
        assert_eq!(hex_str(&lv), "2a");
        let lv = LogicVec::from_u64(0x1A2B, 16);
        assert_eq!(hex_str(&lv), "1a2b");
        let lv = LogicVec::from_u64(0xB, 4);
        assert_eq!(hex_str(&lv), "b");
    }

    #[test]
    fn test_vpi_value_to_logicvec_int() {
        let mut value = t_vpi_value::default();
        value.format = vpiIntVal;
        value.value = vpi_value_union { integer: -5 };
        let lv = value_to_logicvec(&value);
        assert_eq!(lv.width, 32);
        assert_eq!(lv.to_i64(), -5, "int -5 harus jadi i64 -5");
    }

    #[test]
    fn test_vpi_value_to_logicvec_scalar() {
        let mut value = t_vpi_value::default();
        value.format = vpiScalarVal;
        value.value = vpi_value_union { scalar: 1 };
        let lv = value_to_logicvec(&value);
        assert_eq!(lv.width, 1);
        assert_eq!(lv.bits[0], LogicVal::One);

        value.value = vpi_value_union { scalar: 0 };
        let lv = value_to_logicvec(&value);
        assert_eq!(lv.bits[0], LogicVal::Zero);

        value.value = vpi_value_union { scalar: 2 };
        let lv = value_to_logicvec(&value);
        assert_eq!(lv.bits[0], LogicVal::X);

        value.value = vpi_value_union { scalar: 3 };
        let lv = value_to_logicvec(&value);
        assert_eq!(lv.bits[0], LogicVal::Z);
    }

    #[test]
    fn test_vpi_value_to_logicvec_vector() {
        let mut value = t_vpi_value::default();
        value.format = vpiVectorVal;
        value.value = vpi_value_union {
            vector: t_vpi_vector { aval: 0xDEAD, bval: 0 },
        };
        let lv = value_to_logicvec(&value);
        assert_eq!(lv.to_u64(), 0xDEAD);
        assert_eq!(lv.width, 32);
    }

    #[test]
    fn test_vpi_value_to_logicvec_bin_str() {
        // BinStrVal: MSB-first (karakter pertama = bit paling signifikan).
        // internal bits[0] = LSB → bits di-reverse.
        let cname = CString::new("1101").unwrap();
        let mut value = t_vpi_value::default();
        value.format = vpiBinStrVal;
        value.value = vpi_value_union {
            string: cname.as_ptr() as *mut c_char,
        };
        let lv = value_to_logicvec(&value);
        assert_eq!(lv.width, 4);
        // '1'(MSB) '1' '0' '1'(LSB) → bit3=1, bit2=1, bit1=0, bit0=1 = 13
        assert_eq!(lv.to_u64(), 0b1101, "MSB-first '1101' = 0b1101 = 13");

        // underscore di-skip: '1' '0' '0' '1' → bit3=1, bit0=1 = 9
        let cname = CString::new("10_01").unwrap();
        value.value = vpi_value_union {
            string: cname.as_ptr() as *mut c_char,
        };
        let lv = value_to_logicvec(&value);
        assert_eq!(lv.to_u64(), 0b1001);
    }

    #[test]
    fn test_vpi_value_to_logicvec_hex_str() {
        let cname = CString::new("0xFF").unwrap();
        let mut value = t_vpi_value::default();
        value.format = vpiHexStrVal;
        value.value = vpi_value_union {
            string: cname.as_ptr() as *mut c_char,
        };
        let lv = value_to_logicvec(&value);
        assert_eq!(lv.width, 8);
        assert_eq!(lv.to_u64(), 0xFF);
    }

    #[test]
    fn test_vpi_value_to_logicvec_real() {
        let mut value = t_vpi_value::default();
        value.format = vpiRealVal;
        value.value = vpi_value_union { real: 1.5 };
        let lv = value_to_logicvec(&value);
        assert_eq!(lv.width, 64);
        assert_eq!(lv.to_u64(), 1.5f64.to_bits());
    }

    #[test]
    fn test_vpi_value_roundtrip_via_cache_cstring() {
        // LANG-46: bin_str/hex_str hasilnya harus bisa di-cache sebagai
        // CString (dipakai vpi_get_value → vpiBinStrVal/vpiHexStrVal) dan
        // di-decode ulang value_to_logicvec tanpa kehilangan nilai.
        let lv = LogicVec::from_u64(0xA5, 8);
        let s = bin_str(&lv);
        let ptr = super::super::handle::cache_cstring(&s);
        let mut value = t_vpi_value::default();
        value.format = vpiBinStrVal;
        value.value = vpi_value_union { string: ptr };
        let back = value_to_logicvec(&value);
        assert_eq!(back.to_u64(), lv.to_u64(), "bin_str roundtrip");
    }
}
