use crate::simulator::types::XPropagationMode;
use maria_ir::{BinaryIrOp, LogicVal, LogicVec, UnaryIrOp};
use std::cell::RefCell;

thread_local! {
    /// Thread-local X-propagation mode.
    static XPROP_MODE: RefCell<XPropagationMode> = const { RefCell::new(XPropagationMode::Pessimistic) };
}

/// Set X-propagation mode for current thread.
pub fn set_xprop_mode(mode: XPropagationMode) {
    XPROP_MODE.with(|cell| *cell.borrow_mut() = mode);
}

/// Get current X-propagation mode.
pub fn get_xprop_mode() -> XPropagationMode {
    XPROP_MODE.with(|cell| *cell.borrow())
}

/// Convert LogicVec to u64 bitmasks for SIMD-style operations.
/// Returns (known_mask, value_bits) where:
/// - known_mask: bit i = 1 if bit i is known (0/1), 0 if unknown (X/Z)
/// - value_bits: actual value of known bits (only meaningful where known_mask=1)
fn to_bitmasks(lv: &LogicVec) -> (u64, u64) {
    let mut known = 0u64;
    let mut value = 0u64;
    // Defensif: bit yang hilang (bits.len() < width) dianggap X (LRM:
    // bit tak terinisialisasi = unknown). Mencegah panic index-out-of-bounds
    // di jalur eval manapun bila nilai korup (width>0, bits kosong) bocor.
    for i in 0..lv.width.min(64) {
        match lv.bits.get(i).copied().unwrap_or(LogicVal::X) {
            LogicVal::Zero => {
                known |= 1 << i;
            }
            LogicVal::One => {
                known |= 1 << i;
                value |= 1 << i;
            }
            LogicVal::X | LogicVal::Z => {}
        }
    }
    (known, value)
}

/// Convert u64 bitmasks back to LogicVec.
fn from_bitmasks(known: u64, value: u64, width: usize) -> LogicVec {
    let _w = width.min(64);
    let mut bits = Vec::with_capacity(width);
    for i in 0..width {
        if i < 64 {
            if (known >> i) & 1 == 1 {
                bits.push(if (value >> i) & 1 == 1 {
                    LogicVal::One
                } else {
                    LogicVal::Zero
                });
            } else {
                bits.push(LogicVal::X);
            }
        } else {
            bits.push(LogicVal::X); // >64 bits default to X
        }
    }
    LogicVec { bits, width }
}

/// Optimized check for X/Z using u64 bitmasks (O(1) for width ≤ 64).
fn has_xz(val: &LogicVec) -> bool {
    if val.width <= 64 {
        let (known, _) = to_bitmasks(val);
        let mask = if val.width == 64 {
            !0u64
        } else {
            (1u64 << val.width) - 1
        };
        known != mask
    } else {
        val.bits
            .iter()
            .any(|b| *b == LogicVal::X || *b == LogicVal::Z)
    }
}

/// Optimized check for any X/Z in a (known, value) pair.
#[allow(dead_code)]
fn has_xz_u64(known: u64, width: usize) -> bool {
    let mask = if width == 64 {
        !0u64
    } else {
        (1u64 << width) - 1
    };
    known != mask
}

/// Evaluate a unary operation on a logic vector
pub fn eval_unary(op: UnaryIrOp, val: &LogicVec) -> LogicVec {
    match op {
        UnaryIrOp::Plus => val.clone(),
        UnaryIrOp::Minus => {
            if val.width <= 64 && !has_xz(val) {
                // Fast path: u64 two's complement negation
                let v = val.to_u64();
                let neg = (!v).wrapping_add(1); // two's complement: ~v + 1
                LogicVec::from_u64(neg, val.width)
            } else {
                // Slow path: per-bit
                let mut result = LogicVec::new(val.width);
                for (i, b) in val.bits.iter().enumerate() {
                    result.bits[i] = match b {
                        LogicVal::Zero => LogicVal::One,
                        LogicVal::One => LogicVal::Zero,
                        LogicVal::X => LogicVal::X,
                        LogicVal::Z => LogicVal::X,
                    };
                }
                let mut carry = true;
                for b in result.bits.iter_mut() {
                    if carry {
                        match b {
                            LogicVal::Zero => {
                                *b = LogicVal::One;
                                carry = false;
                            }
                            LogicVal::One => {
                                *b = LogicVal::Zero;
                            }
                            LogicVal::X => {
                                carry = false;
                            }
                            LogicVal::Z => {
                                *b = LogicVal::X;
                                carry = false;
                            }
                        }
                    }
                }
                result
            }
        }
        UnaryIrOp::Not => {
            // Logical not: result is 1-bit
            let truthy = val.to_bool().unwrap_or(false);
            LogicVec::from_u64(if truthy { 0 } else { 1 }, 1)
        }
        UnaryIrOp::BitNot => {
            if val.width <= 64 {
                // Fast path: u64 bitmasks
                let (known, value) = to_bitmasks(val);
                let _mask = if val.width == 64 {
                    !0u64
                } else {
                    (1u64 << val.width) - 1
                };
                // Known bits stay known, value is inverted
                // X/Z bits remain unknown
                from_bitmasks(known, !value, val.width)
            } else {
                // Slow path: per-bit
                let mut result = LogicVec::new(val.width);
                for (i, b) in val.bits.iter().enumerate() {
                    result.bits[i] = match b {
                        LogicVal::Zero => LogicVal::One,
                        LogicVal::One => LogicVal::Zero,
                        LogicVal::X => LogicVal::X,
                        LogicVal::Z => LogicVal::X,
                    };
                }
                result
            }
        }
        UnaryIrOp::RedAnd => {
            if val.width <= 64 {
                // Fast path: u64 bitmasks
                let (known, value) = to_bitmasks(val);
                let mask = if val.width == 64 {
                    !0u64
                } else {
                    (1u64 << val.width) - 1
                };
                if known != mask {
                    // Some bits are X/Z → result is X
                    LogicVec {
                        bits: vec![LogicVal::X],
                        width: 1,
                    }
                } else {
                    // All bits known: check if any is 0
                    let all_ones = value & mask == mask;
                    LogicVec::from_u64(if all_ones { 1 } else { 0 }, 1)
                }
            } else {
                // Slow path: per-bit
                let mut result = LogicVal::One;
                for b in &val.bits {
                    match b {
                        LogicVal::Zero => {
                            result = LogicVal::Zero;
                            break;
                        }
                        LogicVal::X | LogicVal::Z => {
                            result = LogicVal::X;
                        }
                        _ => {}
                    }
                }
                LogicVec {
                    bits: vec![result],
                    width: 1,
                }
            }
        }
        UnaryIrOp::RedNand => {
            let and = eval_unary(UnaryIrOp::RedAnd, val);
            eval_unary(UnaryIrOp::BitNot, &and)
        }
        UnaryIrOp::RedOr => {
            if val.width <= 64 {
                // Fast path: u64 bitmasks
                let (known, value) = to_bitmasks(val);
                let mask = if val.width == 64 {
                    !0u64
                } else {
                    (1u64 << val.width) - 1
                };
                if known != mask {
                    // Some bits are X/Z → result is X
                    LogicVec {
                        bits: vec![LogicVal::X],
                        width: 1,
                    }
                } else {
                    // All bits known: check if any is 1
                    let any_one = value & mask != 0;
                    LogicVec::from_u64(if any_one { 1 } else { 0 }, 1)
                }
            } else {
                // Slow path: per-bit
                let mut result = LogicVal::Zero;
                for b in &val.bits {
                    match b {
                        LogicVal::One => {
                            result = LogicVal::One;
                            break;
                        }
                        LogicVal::X | LogicVal::Z => {
                            result = LogicVal::X;
                        }
                        _ => {}
                    }
                }
                LogicVec {
                    bits: vec![result],
                    width: 1,
                }
            }
        }
        UnaryIrOp::RedNor => {
            let or = eval_unary(UnaryIrOp::RedOr, val);
            eval_unary(UnaryIrOp::BitNot, &or)
        }
        UnaryIrOp::RedXor => {
            if val.width <= 64 {
                // Fast path: u64 bitmasks
                let (known, value) = to_bitmasks(val);
                let mask = if val.width == 64 {
                    !0u64
                } else {
                    (1u64 << val.width) - 1
                };
                if known != mask {
                    // Some bits are X/Z → result is X
                    LogicVec {
                        bits: vec![LogicVal::X],
                        width: 1,
                    }
                } else {
                    // XOR of all bits: parity of 1s
                    let ones = (value & mask).count_ones();
                    LogicVec::from_u64(if ones % 2 == 1 { 1 } else { 0 }, 1)
                }
            } else {
                // Slow path: per-bit
                let mut result = LogicVal::Zero;
                for b in &val.bits {
                    match b {
                        LogicVal::One => {
                            result = match result {
                                LogicVal::Zero => LogicVal::One,
                                LogicVal::One => LogicVal::Zero,
                                LogicVal::X => LogicVal::X,
                                LogicVal::Z => LogicVal::X,
                            };
                        }
                        LogicVal::X | LogicVal::Z => {
                            result = LogicVal::X;
                        }
                        _ => {}
                    }
                }
                LogicVec {
                    bits: vec![result],
                    width: 1,
                }
            }
        }
        UnaryIrOp::RedXnor => {
            let xor = eval_unary(UnaryIrOp::RedXor, val);
            eval_unary(UnaryIrOp::BitNot, &xor)
        }
    }
}

/// Sign-extend `val` ke `width` (isi bit di atas lebar asli dengan msb).
/// `extend_to` selalu ZERO-extend — untuk operasi SIGNED (perbandingan,
/// div/mod) nilai 8-bit 0xFF harus jadi -1 (0xFFFFFFFF), bukan 255.
fn sign_extend_to(val: &LogicVec, width: usize) -> LogicVec {
    if val.width >= width {
        return val.clone();
    }
    let mut bits = val.bits.clone();
    let msb = val.bits.last().copied().unwrap_or(LogicVal::Zero);
    let fill = match msb {
        LogicVal::Zero => LogicVal::Zero,
        LogicVal::One => LogicVal::One,
        LogicVal::X | LogicVal::Z => LogicVal::X,
    };
    bits.resize(width, fill);
    LogicVec { bits, width }
}

/// Evaluate a binary operation on logic vectors
pub fn eval_binary_signed(op: BinaryIrOp, lhs: &LogicVec, rhs: &LogicVec) -> LogicVec {
    let max_width = lhs.width.max(rhs.width);
    // ROUND 36: operan SIGNED di-sign-extend dari lebar ASLI-nya (bukan
    // zero-extend) — `logic signed [7:0] s = -1; s < 0` harus -1 < 0 = true,
    // bukan 255 < 0.
    let lhs_ext = sign_extend_to(lhs, max_width);
    let rhs_ext = sign_extend_to(rhs, max_width);
    match op {
        BinaryIrOp::Lt => {
            let l = lhs_ext.to_i64();
            let r = rhs_ext.to_i64();
            LogicVec::from_u64(if l < r { 1 } else { 0 }, 1)
        }
        BinaryIrOp::Le => {
            let l = lhs_ext.to_i64();
            let r = rhs_ext.to_i64();
            LogicVec::from_u64(if l <= r { 1 } else { 0 }, 1)
        }
        BinaryIrOp::Gt => {
            let l = lhs_ext.to_i64();
            let r = rhs_ext.to_i64();
            LogicVec::from_u64(if l > r { 1 } else { 0 }, 1)
        }
        BinaryIrOp::Ge => {
            let l = lhs_ext.to_i64();
            let r = rhs_ext.to_i64();
            LogicVec::from_u64(if l >= r { 1 } else { 0 }, 1)
        }
        BinaryIrOp::Div => {
            let l = if max_width > 64 && max_width <= 128 {
                to_i128_signed(&lhs_ext)
            } else {
                lhs_ext.to_i64() as i128
            };
            let r = if max_width > 64 && max_width <= 128 {
                to_i128_signed(&rhs_ext)
            } else {
                rhs_ext.to_i64() as i128
            };
            if r == 0 {
                LogicVec {
                    bits: vec![LogicVal::X; max_width],
                    width: max_width,
                }
            } else {
                // i128 div truncates toward zero (semantik SV §11.4.3).
                let q = l.wrapping_div(r);
                if max_width > 64 && max_width <= 128 {
                    from_u128_wide(q as u128, max_width)
                } else {
                    LogicVec::from_u64(q as u64, max_width)
                }
            }
        }
        BinaryIrOp::Mod => {
            let l = if max_width > 64 && max_width <= 128 {
                to_i128_signed(&lhs_ext)
            } else {
                lhs_ext.to_i64() as i128
            };
            let r = if max_width > 64 && max_width <= 128 {
                to_i128_signed(&rhs_ext)
            } else {
                rhs_ext.to_i64() as i128
            };
            if r == 0 {
                LogicVec {
                    bits: vec![LogicVal::X; max_width],
                    width: max_width,
                }
            } else {
                // i128 rem truncates toward zero (tanda mengikuti dividen,
                // SV §11.4.3): -7 % 2 = -1.
                let m = l.wrapping_rem(r);
                if max_width > 64 && max_width <= 128 {
                    from_u128_wide(m as u128, max_width)
                } else {
                    LogicVec::from_u64(m as u64, max_width)
                }
            }
        }
        _ => eval_binary(op, lhs, rhs),
    }
}

/// Konversi LogicVec → u128 (hingga 128 bit; bit di atas width = 0).
/// Dipakai jalur aritmetika lebar (>64 bit) — dulu `to_u64` memotong
/// MSB sehingga `num % -(b)` (divisor 65-bit `2^64+1` → terbaca 1)
/// salah total (ditemukan fuzzer seed=41920606, konfirmasi Icarus).
fn to_u128_wide(lv: &LogicVec) -> u128 {
    let mut v = lv.to_u64() as u128;
    for i in 64..lv.width.min(128) {
        if lv.bits.get(i).copied().unwrap_or(LogicVal::X) == LogicVal::One {
            v |= 1u128 << i;
        }
    }
    v
}

/// Konversi u128 → LogicVec selebar `width` (zero-extend / truncate).
fn from_u128_wide(val: u128, width: usize) -> LogicVec {
    let mut lv = LogicVec::new(width);
    for i in 0..width.min(128) {
        lv.bits[i] = if (val >> i) & 1 == 1 {
            LogicVal::One
        } else {
            LogicVal::Zero
        };
    }
    lv
}

/// Sign-extend LogicVec ke i128 (untuk Div/Mod signed >64-bit).
fn to_i128_signed(lv: &LogicVec) -> i128 {
    let v = to_u128_wide(lv);
    if lv.width > 0 && lv.width < 128 {
        let sign_bit = 1u128 << (lv.width - 1);
        if v & sign_bit != 0 {
            return (v | !((1u128 << lv.width) - 1)) as i128;
        }
    }
    v as i128
}

pub fn eval_binary(op: BinaryIrOp, lhs: &LogicVec, rhs: &LogicVec) -> LogicVec {
    let max_width = lhs.width.max(rhs.width);
    let lhs_ext = extend_to(lhs, max_width);
    let rhs_ext = extend_to(rhs, max_width);

    match op {
        BinaryIrOp::Add | BinaryIrOp::Sub => {
            let l_has_x = lhs_ext
                .bits
                .iter()
                .any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            let r_has_x = rhs_ext
                .bits
                .iter()
                .any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            if l_has_x || r_has_x {
                LogicVec {
                    bits: vec![LogicVal::X; max_width],
                    width: max_width,
                }
            } else if max_width > 64 && max_width <= 128 {
                // Jalur lebar: u128 — to_u64 memotong MSB (bug fuzzer
                // seed=41920606).
                let l = to_u128_wide(&lhs_ext);
                let r = to_u128_wide(&rhs_ext);
                let result = match op {
                    BinaryIrOp::Add => l.wrapping_add(r),
                    BinaryIrOp::Sub => l.wrapping_sub(r),
                    _ => unreachable!(),
                };
                from_u128_wide(result, max_width)
            } else {
                let l = lhs_ext.to_u64();
                let r = rhs_ext.to_u64();
                let result = match op {
                    BinaryIrOp::Add => l.wrapping_add(r),
                    BinaryIrOp::Sub => l.wrapping_sub(r),
                    _ => unreachable!(),
                };
                LogicVec::from_u64(result, max_width)
            }
        }
        BinaryIrOp::Mul => {
            let l_has_x = lhs_ext
                .bits
                .iter()
                .any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            let r_has_x = rhs_ext
                .bits
                .iter()
                .any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            if l_has_x || r_has_x {
                LogicVec {
                    bits: vec![LogicVal::X; max_width],
                    width: max_width,
                }
            } else if max_width > 64 && max_width <= 128 {
                let l = to_u128_wide(&lhs_ext);
                let r = to_u128_wide(&rhs_ext);
                from_u128_wide(l.wrapping_mul(r), max_width)
            } else {
                LogicVec::from_u64(lhs_ext.to_u64().wrapping_mul(rhs_ext.to_u64()), max_width)
            }
        }
        BinaryIrOp::Div => {
            let l_has_x = lhs_ext
                .bits
                .iter()
                .any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            let r_has_x = rhs_ext
                .bits
                .iter()
                .any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            if l_has_x || r_has_x {
                LogicVec {
                    bits: vec![LogicVal::X; max_width],
                    width: max_width,
                }
            } else if max_width > 64 && max_width <= 128 {
                let l = to_u128_wide(&lhs_ext);
                let r = to_u128_wide(&rhs_ext);
                match l.checked_div(r) {
                    Some(q) => from_u128_wide(q, max_width),
                    None => LogicVec {
                        bits: vec![LogicVal::X; max_width],
                        width: max_width,
                    },
                }
            } else {
                let l = lhs_ext.to_u64();
                let r = rhs_ext.to_u64();
                match l.checked_div(r) {
                    Some(q) => LogicVec::from_u64(q, max_width),
                    None => LogicVec {
                        bits: vec![LogicVal::X; max_width],
                        width: max_width,
                    },
                }
            }
        }
        BinaryIrOp::Mod => {
            let l_has_x = lhs_ext
                .bits
                .iter()
                .any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            let r_has_x = rhs_ext
                .bits
                .iter()
                .any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            if l_has_x || r_has_x {
                LogicVec {
                    bits: vec![LogicVal::X; max_width],
                    width: max_width,
                }
            } else if max_width > 64 && max_width <= 128 {
                let l = to_u128_wide(&lhs_ext);
                let r = to_u128_wide(&rhs_ext);
                if r == 0 {
                    LogicVec {
                        bits: vec![LogicVal::X; max_width],
                        width: max_width,
                    }
                } else {
                    from_u128_wide(l.wrapping_rem(r), max_width)
                }
            } else {
                let l = lhs_ext.to_u64();
                let r = rhs_ext.to_u64();
                if r == 0 {
                    LogicVec {
                        bits: vec![LogicVal::X; max_width],
                        width: max_width,
                    }
                } else {
                    LogicVec::from_u64(l % r, max_width)
                }
            }
        }
        BinaryIrOp::Power => {
            let l_has_x = lhs_ext
                .bits
                .iter()
                .any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            let r_has_x = rhs_ext
                .bits
                .iter()
                .any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            if l_has_x || r_has_x {
                LogicVec {
                    bits: vec![LogicVal::X; max_width],
                    width: max_width,
                }
            } else {
                let exp = rhs_ext.to_u64();
                // Eksponensiasi modular biner mod 2^max_width (semantik SV:
                // hasil di-size ke max operand, aritmetika modular — divalidasi
                // differential vs Icarus). Dulu `exp >= 64 → 0` — salah total;
                // ditemukan fuzzer (seed 4900864: `b ** 33'h1B1CF1D8` ≠ 0).
                let m: u64 = if max_width >= 64 {
                    u64::MAX
                } else {
                    (1u64 << max_width) - 1
                };
                let mut base = lhs_ext.to_u64() & m;
                let mut acc: u64 = 1 & m;
                let mut e = exp;
                while e > 0 {
                    if e & 1 == 1 {
                        acc = acc.wrapping_mul(base) & m;
                    }
                    e >>= 1;
                    if e > 0 {
                        base = base.wrapping_mul(base) & m;
                    }
                }
                LogicVec::from_u64(acc, max_width)
            }
        }
        BinaryIrOp::Eq | BinaryIrOp::CaseEq => {
            // Pessimistic: X/Z in either operand → X result
            let mode = get_xprop_mode();
            if (mode == XPropagationMode::Pessimistic || mode == XPropagationMode::XAnywhere)
                && (has_xz(&lhs_ext) || has_xz(&rhs_ext))
            {
                return LogicVec {
                    bits: vec![LogicVal::X],
                    width: 1,
                };
            }
            let eq = if max_width <= 64 {
                // Fast path: O(1) u64 bitmask comparison
                let (lk, lv) = to_bitmasks(&lhs_ext);
                let (rk, rv) = to_bitmasks(&rhs_ext);
                lk == rk && lv == rv
            } else {
                lhs_ext.bits == rhs_ext.bits
            };
            LogicVec::from_u64(if eq { 1 } else { 0 }, 1)
        }
        BinaryIrOp::Neq | BinaryIrOp::CaseNeq => {
            let mode = get_xprop_mode();
            if (mode == XPropagationMode::Pessimistic || mode == XPropagationMode::XAnywhere)
                && (has_xz(&lhs_ext) || has_xz(&rhs_ext))
            {
                return LogicVec {
                    bits: vec![LogicVal::X],
                    width: 1,
                };
            }
            let eq = if max_width <= 64 {
                // Fast path: O(1) u64 bitmask comparison
                let (lk, lv) = to_bitmasks(&lhs_ext);
                let (rk, rv) = to_bitmasks(&rhs_ext);
                lk == rk && lv == rv
            } else {
                lhs_ext.bits == rhs_ext.bits
            };
            LogicVec::from_u64(if eq { 0 } else { 1 }, 1)
        }
        BinaryIrOp::EqWild => {
            let eq = lhs_ext.casex_eq(&rhs_ext);
            LogicVec::from_u64(if eq { 1 } else { 0 }, 1)
        }
        BinaryIrOp::NeqWild => {
            let eq = lhs_ext.casex_eq(&rhs_ext);
            LogicVec::from_u64(if eq { 0 } else { 1 }, 1)
        }
        BinaryIrOp::Lt => {
            let mode = get_xprop_mode();
            if (mode == XPropagationMode::Pessimistic || mode == XPropagationMode::XAnywhere)
                && (has_xz(&lhs_ext) || has_xz(&rhs_ext))
            {
                return LogicVec {
                    bits: vec![LogicVal::X],
                    width: 1,
                };
            }
            let l = lhs_ext.to_u64();
            let r = rhs_ext.to_u64();
            LogicVec::from_u64(if l < r { 1 } else { 0 }, 1)
        }
        BinaryIrOp::Le => {
            let mode = get_xprop_mode();
            if (mode == XPropagationMode::Pessimistic || mode == XPropagationMode::XAnywhere)
                && (has_xz(&lhs_ext) || has_xz(&rhs_ext))
            {
                return LogicVec {
                    bits: vec![LogicVal::X],
                    width: 1,
                };
            }
            let l = lhs_ext.to_u64();
            let r = rhs_ext.to_u64();
            LogicVec::from_u64(if l <= r { 1 } else { 0 }, 1)
        }
        BinaryIrOp::Gt => {
            let mode = get_xprop_mode();
            if (mode == XPropagationMode::Pessimistic || mode == XPropagationMode::XAnywhere)
                && (has_xz(&lhs_ext) || has_xz(&rhs_ext))
            {
                return LogicVec {
                    bits: vec![LogicVal::X],
                    width: 1,
                };
            }
            let l = lhs_ext.to_u64();
            let r = rhs_ext.to_u64();
            LogicVec::from_u64(if l > r { 1 } else { 0 }, 1)
        }
        BinaryIrOp::Ge => {
            let mode = get_xprop_mode();
            if (mode == XPropagationMode::Pessimistic || mode == XPropagationMode::XAnywhere)
                && (has_xz(&lhs_ext) || has_xz(&rhs_ext))
            {
                return LogicVec {
                    bits: vec![LogicVal::X],
                    width: 1,
                };
            }
            let l = lhs_ext.to_u64();
            let r = rhs_ext.to_u64();
            LogicVec::from_u64(if l >= r { 1 } else { 0 }, 1)
        }
        BinaryIrOp::BitAnd => {
            let mode = get_xprop_mode();
            if mode == XPropagationMode::Optimistic {
                // Optimistic: 0 & X = 0 (any known bit forces known output)
                bitwise_op(&lhs_ext, &rhs_ext, |a, b| match (a, b) {
                    (LogicVal::Zero, _) | (_, LogicVal::Zero) => LogicVal::Zero,
                    (LogicVal::One, LogicVal::One) => LogicVal::One,
                    _ => LogicVal::X,
                })
            } else {
                // Pessimistic: any X → X — BUG FIX (SIM-11): urutan match lama
                // `(Zero, _) | (_, Zero) => Zero` SEBELUM `_ => X` membuat
                // pessimistic 0 & X = 0 (identik dgn optimistic — mode tak
                // pernah berbeda utk BitAnd). X/Z harus dicek PERTAMA.
                bitwise_op(&lhs_ext, &rhs_ext, |a, b| match (a, b) {
                    (LogicVal::X, _) | (_, LogicVal::X) | (LogicVal::Z, _) | (_, LogicVal::Z) => {
                        LogicVal::X
                    }
                    (LogicVal::One, LogicVal::One) => LogicVal::One,
                    _ => LogicVal::Zero,
                })
            }
        }
        BinaryIrOp::BitOr => {
            let mode = get_xprop_mode();
            if mode == XPropagationMode::Optimistic {
                // Optimistic: 1 | X = 1
                bitwise_op(&lhs_ext, &rhs_ext, |a, b| match (a, b) {
                    (LogicVal::One, _) | (_, LogicVal::One) => LogicVal::One,
                    (LogicVal::Zero, LogicVal::Zero) => LogicVal::Zero,
                    _ => LogicVal::X,
                })
            } else {
                // Pessimistic: any X → X — BUG FIX (SIM-11): urutan match lama
                // `(One, _) | (_, One) => One` SEBELUM `_ => X` membuat
                // pessimistic 1 | X = 1 (identik dgn optimistic). X/Z harus
                // dicek PERTAMA.
                bitwise_op(&lhs_ext, &rhs_ext, |a, b| match (a, b) {
                    (LogicVal::X, _) | (_, LogicVal::X) | (LogicVal::Z, _) | (_, LogicVal::Z) => {
                        LogicVal::X
                    }
                    (LogicVal::One, _) | (_, LogicVal::One) => LogicVal::One,
                    _ => LogicVal::Zero,
                })
            }
        }
        BinaryIrOp::BitXor => bitwise_op(&lhs_ext, &rhs_ext, |a, b| match (a, b) {
            (LogicVal::Zero, LogicVal::Zero) => LogicVal::Zero,
            (LogicVal::One, LogicVal::One) => LogicVal::Zero,
            (LogicVal::Zero, LogicVal::One) => LogicVal::One,
            (LogicVal::One, LogicVal::Zero) => LogicVal::One,
            _ => LogicVal::X,
        }),
        BinaryIrOp::BitXnor => {
            let xor = eval_binary(BinaryIrOp::BitXor, lhs, rhs);
            eval_unary(UnaryIrOp::BitNot, &xor)
        }
        BinaryIrOp::Shl => {
            let shift = rhs_ext.to_u64() as usize;
            let result_width = lhs.width.max(rhs.width); // LRM §11.8.1: operand kiri shift context-determined — extend ke lebar operand kanan (unsized literal = 32). Dulu lhs.width saja → hasil shift dari comparison 1-bit tak ter-extend (ditemukan fuzzer seed=57554764, dikonfirmasi Icarus).
                                          // If shift amount has X/Z, result is unknown
            if has_xz(&rhs_ext) {
                return LogicVec {
                    bits: vec![LogicVal::X; result_width],
                    width: result_width,
                };
            }
            if result_width <= 64 && !has_xz(&lhs) {
                // Fast path: u64 shift
                let val = lhs.to_u64();
                let shifted = if shift >= result_width {
                    0
                } else {
                    val << shift
                };
                LogicVec::from_u64(shifted, result_width)
            } else {
                // Slow path: per-bit
                // If lhs has X/Z, shift result is X (unknown shifted by any amount = unknown)
                if has_xz(lhs) {
                    return LogicVec {
                        bits: vec![LogicVal::X; result_width],
                        width: result_width,
                    };
                }
                let mut result = lhs.clone();
                if result_width > lhs.width {
                    // Operan kiri di-zero-extend ke result_width (LRM
                    // §11.8.1 context-determined) — tanpa ini indexing
                    // result.bits OOB (ditemukan fuzzer seed=668917811772).
                    result.bits.extend(
                        std::iter::repeat(LogicVal::Zero).take(result_width - lhs.width),
                    );
                    result.width = result_width;
                }
                if shift > 0 && shift < result_width {
                    for i in (shift..result_width).rev() {
                        // Sumber dibatasi lebar asli lhs (zero-extension
                        // context-determined).
                        result.bits[i] = if i - shift < lhs.width {
                            lhs.bits[i - shift]
                        } else {
                            LogicVal::Zero
                        };
                    }
                    for i in 0..shift {
                        result.bits[i] = LogicVal::Zero;
                    }
                } else if shift >= result_width {
                    // Shift by >= width -> all zeros
                    for bit in result.bits.iter_mut() {
                        *bit = LogicVal::Zero;
                    }
                }
                result
            }
        }
        BinaryIrOp::Shr => {
            let shift = rhs_ext.to_u64() as usize;
            let result_width = lhs.width.max(rhs.width); // LRM §11.8.1: operand kiri shift context-determined — extend ke lebar operand kanan (unsized literal = 32). Dulu lhs.width saja → hasil shift dari comparison 1-bit tak ter-extend (ditemukan fuzzer seed=57554764, dikonfirmasi Icarus).
                                          // If shift amount has X/Z, result is unknown
            if has_xz(&rhs_ext) {
                return LogicVec {
                    bits: vec![LogicVal::X; result_width],
                    width: result_width,
                };
            }
            if result_width <= 64 && !has_xz(&lhs) {
                // Fast path: u64 shift
                let val = lhs.to_u64();
                let shifted = if shift >= result_width {
                    0
                } else {
                    val >> shift
                };
                LogicVec::from_u64(shifted, result_width)
            } else {
                // Slow path: per-bit
                // If lhs has X/Z, shift result is X
                if has_xz(lhs) {
                    return LogicVec {
                        bits: vec![LogicVal::X; result_width],
                        width: result_width,
                    };
                }
                let mut result = lhs.clone();
                if result_width > lhs.width {
                    // Operan kiri di-zero-extend ke result_width (LRM
                    // §11.8.1 context-determined) — tanpa ini indexing
                    // result.bits OOB (ditemukan fuzzer seed=668917811772).
                    result.bits.extend(
                        std::iter::repeat(LogicVal::Zero).take(result_width - lhs.width),
                    );
                    result.width = result_width;
                }
                if shift > 0 && shift < result_width {
                    for i in 0..(result_width - shift) {
                        // Sumber dibatasi lebar asli lhs; sisanya zero-fill
                        // (zero-extension context-determined).
                        result.bits[i] = if i + shift < lhs.width {
                            lhs.bits[i + shift]
                        } else {
                            LogicVal::Zero
                        };
                    }
                    for i in (result_width - shift)..result_width {
                        result.bits[i] = LogicVal::Zero;
                    }
                } else if shift >= result_width {
                    // Shift by >= width -> all zeros
                    for bit in result.bits.iter_mut() {
                        *bit = LogicVal::Zero;
                    }
                }
                result
            }
        }
        BinaryIrOp::Sshl => {
            let shift = rhs_ext.to_u64() as usize;
            let result_width = lhs.width.max(rhs.width); // LRM §11.8.1: operand kiri shift context-determined — extend ke lebar operand kanan (unsized literal = 32). Dulu lhs.width saja → hasil shift dari comparison 1-bit tak ter-extend (ditemukan fuzzer seed=57554764, dikonfirmasi Icarus).
                                          // If shift amount has X/Z, result is unknown
            if has_xz(&rhs_ext) {
                return LogicVec {
                    bits: vec![LogicVal::X; result_width],
                    width: result_width,
                };
            }
            if result_width <= 64 && !has_xz(&lhs) {
                // Arithmetic shift left is same as logical shift left
                let val = lhs.to_u64();
                let shifted = if shift >= result_width {
                    0
                } else {
                    val << shift
                };
                LogicVec::from_u64(shifted, result_width)
            } else {
                // Slow path: per-bit
                // If lhs has X/Z, shift result is X
                if has_xz(lhs) {
                    return LogicVec {
                        bits: vec![LogicVal::X; result_width],
                        width: result_width,
                    };
                }
                let _msb = lhs.bits.last().copied().unwrap_or(LogicVal::Zero);
                let mut result = lhs.clone();
                if result_width > lhs.width {
                    // Operan kiri di-zero-extend ke result_width (LRM
                    // §11.8.1 context-determined) — tanpa ini indexing
                    // result.bits OOB (ditemukan fuzzer seed=668917811772).
                    result.bits.extend(
                        std::iter::repeat(LogicVal::Zero).take(result_width - lhs.width),
                    );
                    result.width = result_width;
                }
                if shift > 0 && shift < result_width {
                    for _ in 0..shift {
                        for i in (1..result_width).rev() {
                            result.bits[i] = result.bits[i - 1];
                        }
                        result.bits[0] = LogicVal::Zero;
                    }
                } else if shift >= result_width {
                    // Shift by >= width -> all zeros
                    for bit in result.bits.iter_mut() {
                        *bit = LogicVal::Zero;
                    }
                }
                result
            }
        }
        BinaryIrOp::Sshr => {
            let shift = rhs_ext.to_u64() as usize;
            let result_width = lhs.width.max(rhs.width); // LRM §11.8.1: operand kiri shift context-determined — extend ke lebar operand kanan (unsized literal = 32). Dulu lhs.width saja → hasil shift dari comparison 1-bit tak ter-extend (ditemukan fuzzer seed=57554764, dikonfirmasi Icarus).
                                          // If shift amount has X/Z, result is unknown
            if has_xz(&rhs_ext) {
                return LogicVec {
                    bits: vec![LogicVal::X; result_width],
                    width: result_width,
                };
            }
            if result_width <= 64 && !has_xz(&lhs) {
                // Arithmetic shift right: extend sign bit
                let val = lhs.to_u64();
                let sign_bit = (val >> (result_width - 1)) & 1;
                let shifted = if shift >= result_width {
                    if sign_bit == 1 {
                        !0u64
                    } else {
                        0
                    }
                } else {
                    let shifted_val = val >> shift;
                    if sign_bit == 1 {
                        // Fill high bits with 1s for arithmetic shift
                        shifted_val | (!0u64 << (result_width - shift))
                    } else {
                        shifted_val
                    }
                };
                LogicVec::from_u64(shifted, result_width)
            } else {
                // Slow path: per-bit
                // If lhs has X/Z, shift result is X
                if has_xz(lhs) {
                    return LogicVec {
                        bits: vec![LogicVal::X; result_width],
                        width: result_width,
                    };
                }
                let msb = lhs.bits.last().copied().unwrap_or(LogicVal::Zero);
                let mut result = lhs.clone();
                if result_width > lhs.width {
                    // Operan kiri di-zero-extend ke result_width (LRM
                    // §11.8.1 context-determined) — tanpa ini indexing
                    // result.bits OOB (ditemukan fuzzer seed=668917811772).
                    result.bits.extend(
                        std::iter::repeat(LogicVal::Zero).take(result_width - lhs.width),
                    );
                    result.width = result_width;
                }
                if shift > 0 && shift < result_width {
                    for _ in 0..shift {
                        for i in 0..(result_width - 1) {
                            result.bits[i] = result.bits[i + 1];
                        }
                        *result.bits.last_mut().unwrap() = msb;
                    }
                } else if shift >= result_width {
                    // Shift by >= width -> fill with sign bit
                    for bit in result.bits.iter_mut() {
                        *bit = msb;
                    }
                }
                result
            }
        }
        BinaryIrOp::LogicalAnd => {
            let l = lhs.to_bool().unwrap_or(false);
            let r = rhs.to_bool().unwrap_or(false);
            LogicVec::from_u64(if l && r { 1 } else { 0 }, 1)
        }
        BinaryIrOp::LogicalOr => {
            let l = lhs.to_bool().unwrap_or(false);
            let r = rhs.to_bool().unwrap_or(false);
            LogicVec::from_u64(if l || r { 1 } else { 0 }, 1)
        }
    }
}

fn extend_to(val: &LogicVec, width: usize) -> LogicVec {
    if val.width >= width {
        val.clone()
    } else {
        let mut bits = val.bits.clone();
        let msb = val.bits.last().copied().unwrap_or(LogicVal::Zero);
        let fill = match msb {
            LogicVal::Zero | LogicVal::One => LogicVal::Zero,
            LogicVal::X | LogicVal::Z => LogicVal::X,
        };
        bits.resize(width, fill);
        LogicVec { bits, width }
    }
}

/// Arithmetic shift right pada nilai ber-LEBAR ASLINYA (bukan max_width
/// operand). ROUND 36: `a >>> b` (IEEE 1800 §11.4.10) arithmetic bila `a`
/// SIGNED, logical bila unsigned. `extend_to` selalu zero-extend — Sshr lama
/// menghitung sign_bit dari nilai yang sudah di-extend ke max_width sehingga
/// `logic signed [7:0] s = -128; s >>> 2` = 0x20 (harusnya 0xE0). Di sini
/// sign bit diambil dari msb lebar asli lhs; hasil berlebar lhs.width.
pub fn eval_sshr_signed(lhs: &LogicVec, rhs: &LogicVec) -> LogicVec {
    let shift = rhs.to_u64() as usize;
    let lw = lhs.width;
    if lw == 0 {
        return LogicVec::new(0);
    }
    let has_xz = lhs
        .bits
        .iter()
        .any(|b| matches!(b, LogicVal::X | LogicVal::Z));
    if lw <= 64 && !has_xz {
        let val = lhs.to_u64();
        let sign_bit = (val >> (lw - 1)) & 1;
        let shifted = if shift >= 64 {
            if sign_bit == 1 {
                !0u64
            } else {
                0
            }
        } else {
            let sv = val >> shift;
            if sign_bit == 1 {
                let fill_bits = (!0u64) << (lw.saturating_sub(shift).min(64));
                sv | fill_bits
            } else {
                sv
            }
        };
        LogicVec::from_u64(shifted, lw)
    } else {
        // Slow path per-bit dari lebar asli (sign bit = msb asli).
        let msb = lhs.bits.last().copied().unwrap_or(LogicVal::Zero);
        let mut result = lhs.clone();
        for _ in 0..shift.min(result.width) {
            for i in 0..(result.width - 1) {
                result.bits[i] = result.bits[i + 1];
            }
            *result.bits.last_mut().unwrap() = msb;
        }
        result
    }
}

/// Optimized bitwise operation using u64 bitmasks for width ≤ 64.
/// Falls back to per-bit loop for larger widths.
fn bitwise_op<F>(lhs: &LogicVec, rhs: &LogicVec, op: F) -> LogicVec
where
    F: Fn(LogicVal, LogicVal) -> LogicVal,
{
    let width = lhs.width.max(rhs.width);

    // Fast path: use u64 bitmasks for ≤ 64 bits
    if width <= 64 {
        let (mut lk, lv) = to_bitmasks(lhs);
        let (mut rk, rv) = to_bitmasks(rhs);
        let mask = if width == 64 {
            !0u64
        } else {
            (1u64 << width) - 1
        };

        // Zero-extend shorter operand: bits beyond its width are known 0
        if lhs.width < width {
            let lhs_ext_mask = if lhs.width == 64 {
                !0u64
            } else {
                (1u64 << lhs.width) - 1
            };
            lk |= mask & !lhs_ext_mask; // set known=1, value=0 for extension bits
        }
        if rhs.width < width {
            let rhs_ext_mask = if rhs.width == 64 {
                !0u64
            } else {
                (1u64 << rhs.width) - 1
            };
            rk |= mask & !rhs_ext_mask; // set known=1, value=0 for extension bits
        }

        // Compute per-bit results using u64 ops
        let mut result_known = 0u64;
        let mut result_value = 0u64;

        for i in 0..width {
            let l_val = if (lk >> i) & 1 == 1 {
                if (lv >> i) & 1 == 1 {
                    LogicVal::One
                } else {
                    LogicVal::Zero
                }
            } else {
                LogicVal::X
            };
            let r_val = if (rk >> i) & 1 == 1 {
                if (rv >> i) & 1 == 1 {
                    LogicVal::One
                } else {
                    LogicVal::Zero
                }
            } else {
                LogicVal::X
            };
            match op(l_val, r_val) {
                LogicVal::Zero => {
                    result_known |= 1 << i;
                }
                LogicVal::One => {
                    result_known |= 1 << i;
                    result_value |= 1 << i;
                }
                LogicVal::X | LogicVal::Z => {}
            }
        }

        return from_bitmasks(result_known & mask, result_value & mask, width);
    }

    // Slow path: per-bit loop for > 64 bits
    let mut bits = Vec::with_capacity(width);
    for i in 0..width {
        let l = lhs.bits.get(i).copied().unwrap_or(LogicVal::Zero);
        let r = rhs.bits.get(i).copied().unwrap_or(LogicVal::Zero);
        bits.push(op(l, r));
    }
    LogicVec { bits, width }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logicvec_from_u64() {
        let v = LogicVec::from_u64(0b1010, 4);
        assert_eq!(v.to_u64(), 0b1010);
        assert_eq!(format!("{}", v), "1010");
    }

    #[test]
    fn test_bitwise_and() {
        let a = LogicVec::from_u64(0b1100, 4);
        let b = LogicVec::from_u64(0b1010, 4);
        let r = eval_binary(BinaryIrOp::BitAnd, &a, &b);
        assert_eq!(r.to_u64(), 0b1000);
    }

    #[test]
    fn test_bitwise_or() {
        let a = LogicVec::from_u64(0b1100, 4);
        let b = LogicVec::from_u64(0b1010, 4);
        let r = eval_binary(BinaryIrOp::BitOr, &a, &b);
        assert_eq!(r.to_u64(), 0b1110);
    }

    #[test]
    fn test_add() {
        let a = LogicVec::from_u64(5, 8);
        let b = LogicVec::from_u64(3, 8);
        let r = eval_binary(BinaryIrOp::Add, &a, &b);
        assert_eq!(r.to_u64(), 8);
    }

    #[test]
    fn test_bit_not() {
        let a = LogicVec::from_u64(0b1010, 4);
        let r = eval_unary(UnaryIrOp::BitNot, &a);
        assert_eq!(r.to_u64(), 0b0101);
    }

    #[test]
    fn test_logical_not() {
        let a = LogicVec::from_u64(0, 1);
        let r = eval_unary(UnaryIrOp::Not, &a);
        assert_eq!(r.to_u64(), 1);
    }

    // ─── SIM-11: X propagation mode ────────────────────────────────────
    // XPropagationMode menentukan perilaku operator thd X: Optimistic
    // (0 & X = 0, 1 | X = 1), Pessimistic (0 & X = X). Mode thread-local —
    // restore ke default agar test lain tidak terpengaruh.
    fn x_vec() -> LogicVec {
        LogicVec {
            bits: vec![LogicVal::X],
            width: 1,
        }
    }

    #[test]
    fn test_xprop_optimistic_bitand_masks_x() {
        let prev = get_xprop_mode();
        set_xprop_mode(XPropagationMode::Optimistic);
        let r = eval_binary(BinaryIrOp::BitAnd, &LogicVec::from_u64(0, 1), &x_vec());
        set_xprop_mode(prev);
        assert_eq!(r.bits[0], LogicVal::Zero, "optimistic: 0 & X harus 0");
    }

    #[test]
    fn test_xprop_pessimistic_bitand_propagates_x() {
        let prev = get_xprop_mode();
        set_xprop_mode(XPropagationMode::Pessimistic);
        let r = eval_binary(BinaryIrOp::BitAnd, &LogicVec::from_u64(0, 1), &x_vec());
        set_xprop_mode(prev);
        assert_eq!(r.bits[0], LogicVal::X, "pessimistic: 0 & X harus X");
    }

    #[test]
    fn test_xprop_optimistic_bitor_masks_x() {
        let prev = get_xprop_mode();
        set_xprop_mode(XPropagationMode::Optimistic);
        let r = eval_binary(BinaryIrOp::BitOr, &LogicVec::from_u64(1, 1), &x_vec());
        set_xprop_mode(prev);
        assert_eq!(r.bits[0], LogicVal::One, "optimistic: 1 | X harus 1");
    }

    #[test]
    fn test_xprop_pessimistic_bitor_propagates_x() {
        let prev = get_xprop_mode();
        set_xprop_mode(XPropagationMode::Pessimistic);
        let r = eval_binary(BinaryIrOp::BitOr, &LogicVec::from_u64(1, 1), &x_vec());
        set_xprop_mode(prev);
        assert_eq!(r.bits[0], LogicVal::X, "pessimistic: 1 | X harus X");
    }

    #[test]
    fn test_xprop_pessimistic_eq_x_returns_x() {
        let prev = get_xprop_mode();
        set_xprop_mode(XPropagationMode::Pessimistic);
        let r = eval_binary(BinaryIrOp::Eq, &LogicVec::from_u64(5, 8), &x_vec());
        set_xprop_mode(prev);
        assert_eq!(r.bits[0], LogicVal::X, "pessimistic: 5 == X harus X");
    }

    // ─── Defensif: LogicVec korup (width>0, bits kosong) — PANIC-13 ───
    // Index OOB pada array unpacked pernah menghasilkan `bits` kosong dengan
    // width>0 → panic index-out-of-bounds di to_bitmasks (value.rs:28).
    // Fast path harus memperlakukan bit yang hilang sebagai X.
    fn corrupt_vec(width: usize) -> LogicVec {
        LogicVec {
            bits: Vec::new(),
            width,
        }
    }

    #[test]
    fn test_to_bitmasks_empty_bits_does_not_panic() {
        let (known, _) = to_bitmasks(&corrupt_vec(8));
        assert_eq!(known, 0, "bit hilang = X → known mask kosong");
    }

    #[test]
    fn test_eval_unary_on_empty_bits_does_not_panic() {
        let v = corrupt_vec(8);
        let bn = eval_unary(UnaryIrOp::BitNot, &v);
        assert_eq!(bn.bits[0], LogicVal::X, "BitNot X = X");
        let ra = eval_unary(UnaryIrOp::RedAnd, &v);
        assert_eq!(ra.bits[0], LogicVal::X, "RedAnd X = X");
        let ro = eval_unary(UnaryIrOp::RedOr, &v);
        assert_eq!(ro.bits[0], LogicVal::X, "RedOr X = X");
    }

    #[test]
    fn test_eval_binary_eq_on_empty_bits_does_not_panic() {
        let v = corrupt_vec(4);
        let other = LogicVec::from_u64(0, 4);
        let eq = eval_binary(BinaryIrOp::Eq, &v, &other);
        assert_eq!(eq.bits[0], LogicVal::X, "X == 0 → X (pessimistic)");
        let neq = eval_binary(BinaryIrOp::Neq, &v, &other);
        assert_eq!(neq.bits[0], LogicVal::X, "X != 0 → X (pessimistic)");
    }
}
