//! VPI Value Access — get_value / put_value with format conversion.
//!
//! Converts between VPI value formats (IntVal, VectorVal, BinStrVal, etc.)
//! and internal LogicVec representation.

use super::handle::*;
use super::types::*;
use crate::ir::*;

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
    let mut s = String::with_capacity(lv.width / 4 + 1);
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
