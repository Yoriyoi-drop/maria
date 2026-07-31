use crate::ir::{BinaryIrOp, LogicVal, LogicVec, UnaryIrOp};
use crate::simulator::types::XPropagationMode;
use std::cell::RefCell;
use std::fmt;

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
    for i in 0..lv.width.min(64) {
        match lv.bits[i] {
            LogicVal::Zero => { known |= 1 << i; }
            LogicVal::One => { known |= 1 << i; value |= 1 << i; }
            LogicVal::X | LogicVal::Z => {}
        }
    }
    (known, value)
}

/// Convert u64 bitmasks back to LogicVec.
fn from_bitmasks(known: u64, value: u64, width: usize) -> LogicVec {
    let w = width.min(64);
    let mut bits = Vec::with_capacity(width);
    for i in 0..width {
        if i < 64 {
            if (known >> i) & 1 == 1 {
                bits.push(if (value >> i) & 1 == 1 { LogicVal::One } else { LogicVal::Zero });
            } else {
                bits.push(LogicVal::X);
            }
        } else {
            bits.push(LogicVal::X);  // >64 bits default to X
        }
    }
    LogicVec { bits, width }
}

/// Optimized check for X/Z using u64 bitmasks (O(1) for width ≤ 64).
fn has_xz(val: &LogicVec) -> bool {
    if val.width <= 64 {
        let (known, _) = to_bitmasks(val);
        let mask = if val.width == 64 { !0u64 } else { (1u64 << val.width) - 1 };
        known != mask
    } else {
        val.bits.iter().any(|b| *b == LogicVal::X || *b == LogicVal::Z)
    }
}

/// Optimized check for any X/Z in a (known, value) pair.
fn has_xz_u64(known: u64, width: usize) -> bool {
    let mask = if width == 64 { !0u64 } else { (1u64 << width) - 1 };
    known != mask
}

impl fmt::Display for LogicVal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogicVal::Zero => write!(f, "0"),
            LogicVal::One => write!(f, "1"),
            LogicVal::X => write!(f, "x"),
            LogicVal::Z => write!(f, "z"),
        }
    }
}

impl fmt::Display for LogicVec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for bit in self.bits.iter().rev() {
            write!(f, "{}", bit)?;
        }
        Ok(())
    }
}

/// Evaluate a unary operation on a logic vector
pub fn eval_unary(op: UnaryIrOp, val: &LogicVec) -> LogicVec {
    match op {
        UnaryIrOp::Plus => val.clone(),
        UnaryIrOp::Minus => {
            if val.width <= 64 && !has_xz(val) {
                // Fast path: u64 two's complement negation
                let v = val.to_u64();
                let neg = (!v).wrapping_add(1);  // two's complement: ~v + 1
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
                let mask = if val.width == 64 { !0u64 } else { (1u64 << val.width) - 1 };
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
                let mask = if val.width == 64 { !0u64 } else { (1u64 << val.width) - 1 };
                if known != mask {
                    // Some bits are X/Z → result is X
                    LogicVec { bits: vec![LogicVal::X], width: 1 }
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
                LogicVec { bits: vec![result], width: 1 }
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
                let mask = if val.width == 64 { !0u64 } else { (1u64 << val.width) - 1 };
                if known != mask {
                    // Some bits are X/Z → result is X
                    LogicVec { bits: vec![LogicVal::X], width: 1 }
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
                LogicVec { bits: vec![result], width: 1 }
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
                let mask = if val.width == 64 { !0u64 } else { (1u64 << val.width) - 1 };
                if known != mask {
                    // Some bits are X/Z → result is X
                    LogicVec { bits: vec![LogicVal::X], width: 1 }
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
                LogicVec { bits: vec![result], width: 1 }
            }
        }
        UnaryIrOp::RedXnor => {
            let xor = eval_unary(UnaryIrOp::RedXor, val);
            eval_unary(UnaryIrOp::BitNot, &xor)
        }
    }
}

/// Evaluate a binary operation on logic vectors
pub fn eval_binary_signed(op: BinaryIrOp, lhs: &LogicVec, rhs: &LogicVec) -> LogicVec {
    let max_width = lhs.width.max(rhs.width);
    let lhs_ext = extend_to(lhs, max_width);
    let rhs_ext = extend_to(rhs, max_width);
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
        _ => eval_binary(op, lhs, rhs),
    }
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
                LogicVec::from_u64(
                    lhs_ext.to_u64().wrapping_pow(rhs_ext.to_u64() as u32),
                    max_width,
                )
            }
        }
        BinaryIrOp::Eq | BinaryIrOp::CaseEq => {
            // Pessimistic: X/Z in either operand → X result
            let mode = get_xprop_mode();
            if (mode == XPropagationMode::Pessimistic || mode == XPropagationMode::XAnywhere)
                && (has_xz(&lhs_ext) || has_xz(&rhs_ext))
            {
                return LogicVec { bits: vec![LogicVal::X], width: 1 };
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
                return LogicVec { bits: vec![LogicVal::X], width: 1 };
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
                return LogicVec { bits: vec![LogicVal::X], width: 1 };
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
                return LogicVec { bits: vec![LogicVal::X], width: 1 };
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
                return LogicVec { bits: vec![LogicVal::X], width: 1 };
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
                return LogicVec { bits: vec![LogicVal::X], width: 1 };
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
                // Pessimistic: any X → X
                bitwise_op(&lhs_ext, &rhs_ext, |a, b| match (a, b) {
                    (LogicVal::One, LogicVal::One) => LogicVal::One,
                    (LogicVal::Zero, _) | (_, LogicVal::Zero) => LogicVal::Zero,
                    _ => LogicVal::X,
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
                // Pessimistic: any X → X
                bitwise_op(&lhs_ext, &rhs_ext, |a, b| match (a, b) {
                    (LogicVal::Zero, LogicVal::Zero) => LogicVal::Zero,
                    (LogicVal::One, _) | (_, LogicVal::One) => LogicVal::One,
                    _ => LogicVal::X,
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
            if max_width <= 64 && !has_xz(&lhs_ext) {
                // Fast path: u64 shift
                let val = lhs_ext.to_u64();
                let shifted = if shift >= 64 { 0 } else { val << shift };
                LogicVec::from_u64(shifted, max_width)
            } else {
                // Slow path: per-bit
                let mut result = lhs_ext.clone();
                if shift > 0 {
                    for i in (shift..max_width).rev() {
                        result.bits[i] = lhs_ext.bits[i - shift];
                    }
                    for i in 0..shift.min(max_width) {
                        result.bits[i] = LogicVal::Zero;
                    }
                }
                result
            }
        }
        BinaryIrOp::Shr => {
            let shift = rhs_ext.to_u64() as usize;
            if max_width <= 64 && !has_xz(&lhs_ext) {
                // Fast path: u64 shift
                let val = lhs_ext.to_u64();
                let shifted = if shift >= 64 { 0 } else { val >> shift };
                LogicVec::from_u64(shifted, max_width)
            } else {
                // Slow path: per-bit
                let mut result = lhs_ext.clone();
                if shift > 0 {
                    for i in 0..(max_width - shift) {
                        result.bits[i] = lhs_ext.bits[i + shift];
                    }
                    for i in (max_width - shift)..max_width {
                        result.bits[i] = LogicVal::Zero;
                    }
                }
                result
            }
        }
        BinaryIrOp::Sshl => {
            let shift = rhs_ext.to_u64() as usize;
            if max_width <= 64 && !has_xz(&lhs_ext) {
                // Arithmetic shift left is same as logical shift left
                let val = lhs_ext.to_u64();
                let shifted = if shift >= 64 { 0 } else { val << shift };
                LogicVec::from_u64(shifted, max_width)
            } else {
                // Slow path: per-bit
                let _msb = lhs_ext.bits.last().copied().unwrap_or(LogicVal::Zero);
                let mut result = lhs_ext;
                for _ in 0..shift {
                    for i in (1..result.width).rev() {
                        result.bits[i] = result.bits[i - 1];
                    }
                    result.bits[0] = LogicVal::Zero;
                }
                result
            }
        }
        BinaryIrOp::Sshr => {
            let shift = rhs_ext.to_u64() as usize;
            if max_width <= 64 && !has_xz(&lhs_ext) {
                // Arithmetic shift right: extend sign bit
                let val = lhs_ext.to_u64();
                let sign_bit = (val >> (max_width - 1)) & 1;
                let shifted = if shift >= 64 {
                    if sign_bit == 1 { !0u64 } else { 0 }
                } else {
                    let shifted_val = val >> shift;
                    if sign_bit == 1 {
                        // Fill high bits with 1s for arithmetic shift
                        shifted_val | (!0u64 << (max_width - shift))
                    } else {
                        shifted_val
                    }
                };
                LogicVec::from_u64(shifted, max_width)
            } else {
                // Slow path: per-bit
                let msb = lhs_ext.bits.last().copied().unwrap_or(LogicVal::Zero);
                let mut result = lhs_ext;
                for _ in 0..shift {
                    for i in 0..(result.width - 1) {
                        result.bits[i] = result.bits[i + 1];
                    }
                    *result.bits.last_mut().unwrap() = msb;
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
        let mask = if width == 64 { !0u64 } else { (1u64 << width) - 1 };
        
        // Zero-extend shorter operand: bits beyond its width are known 0
        if lhs.width < width {
            let lhs_ext_mask = if lhs.width == 64 { !0u64 } else { (1u64 << lhs.width) - 1 };
            lk |= mask & !lhs_ext_mask;  // set known=1, value=0 for extension bits
        }
        if rhs.width < width {
            let rhs_ext_mask = if rhs.width == 64 { !0u64 } else { (1u64 << rhs.width) - 1 };
            rk |= mask & !rhs_ext_mask;  // set known=1, value=0 for extension bits
        }
        
        // Compute per-bit results using u64 ops
        let mut result_known = 0u64;
        let mut result_value = 0u64;
        
        for i in 0..width {
            let l_val = if (lk >> i) & 1 == 1 {
                if (lv >> i) & 1 == 1 { LogicVal::One } else { LogicVal::Zero }
            } else {
                LogicVal::X
            };
            let r_val = if (rk >> i) & 1 == 1 {
                if (rv >> i) & 1 == 1 { LogicVal::One } else { LogicVal::Zero }
            } else {
                LogicVal::X
            };
            match op(l_val, r_val) {
                LogicVal::Zero => { result_known |= 1 << i; }
                LogicVal::One => { result_known |= 1 << i; result_value |= 1 << i; }
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
}
