use crate::ir::{BinaryIrOp, LogicVal, LogicVec, UnaryIrOp};
use std::fmt;

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
            // Two's complement negation
            let mut result = LogicVec::new(val.width);
            // Bitwise not
            for (i, b) in val.bits.iter().enumerate() {
                result.bits[i] = match b {
                    LogicVal::Zero => LogicVal::One,
                    LogicVal::One => LogicVal::Zero,
                    LogicVal::X => LogicVal::X,
                    LogicVal::Z => LogicVal::X,
                };
            }
            // Add 1
            let mut carry = true;
            for b in result.bits.iter_mut() {
                if carry {
                    match b {
                        LogicVal::Zero => { *b = LogicVal::One; carry = false; }
                        LogicVal::One => { *b = LogicVal::Zero; }
                        LogicVal::X => { carry = false; }
                        LogicVal::Z => { *b = LogicVal::X; carry = false; }
                    }
                }
            }
            result
        }
        UnaryIrOp::Not => {
            // Logical not: result is 1-bit
            let truthy = val.to_bool().unwrap_or(false);
            LogicVec::from_u64(if truthy { 0 } else { 1 }, 1)
        }
        UnaryIrOp::BitNot => {
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
        UnaryIrOp::RedAnd => {
            let mut result = LogicVal::One;
            for b in &val.bits {
                match b {
                    LogicVal::Zero => { result = LogicVal::Zero; break; }
                    LogicVal::X | LogicVal::Z => { result = LogicVal::X; }
                    _ => {}
                }
            }
            LogicVec { bits: vec![result], width: 1 }
        }
        UnaryIrOp::RedNand => {
            let and = eval_unary(UnaryIrOp::RedAnd, val);
            eval_unary(UnaryIrOp::BitNot, &and)
        }
        UnaryIrOp::RedOr => {
            let mut result = LogicVal::Zero;
            for b in &val.bits {
                match b {
                    LogicVal::One => { result = LogicVal::One; break; }
                    LogicVal::X | LogicVal::Z => { result = LogicVal::X; }
                    _ => {}
                }
            }
            LogicVec { bits: vec![result], width: 1 }
        }
        UnaryIrOp::RedNor => {
            let or = eval_unary(UnaryIrOp::RedOr, val);
            eval_unary(UnaryIrOp::BitNot, &or)
        }
        UnaryIrOp::RedXor => {
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
                    LogicVal::X | LogicVal::Z => { result = LogicVal::X; }
                    _ => {}
                }
            }
            LogicVec { bits: vec![result], width: 1 }
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
            let l_has_x = lhs_ext.bits.iter().any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            let r_has_x = rhs_ext.bits.iter().any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            if l_has_x || r_has_x {
                LogicVec { bits: vec![LogicVal::X; max_width], width: max_width }
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
            let l_has_x = lhs_ext.bits.iter().any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            let r_has_x = rhs_ext.bits.iter().any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            if l_has_x || r_has_x {
                LogicVec { bits: vec![LogicVal::X; max_width], width: max_width }
            } else {
                LogicVec::from_u64(lhs_ext.to_u64().wrapping_mul(rhs_ext.to_u64()), max_width)
            }
        }
        BinaryIrOp::Div => {
            let l_has_x = lhs_ext.bits.iter().any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            let r_has_x = rhs_ext.bits.iter().any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            if l_has_x || r_has_x {
                LogicVec { bits: vec![LogicVal::X; max_width], width: max_width }
            } else {
                let l = lhs_ext.to_u64();
                let r = rhs_ext.to_u64();
                if r == 0 {
                    LogicVec { bits: vec![LogicVal::X; max_width], width: max_width }
                } else {
                    LogicVec::from_u64(l / r, max_width)
                }
            }
        }
        BinaryIrOp::Mod => {
            let l_has_x = lhs_ext.bits.iter().any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            let r_has_x = rhs_ext.bits.iter().any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            if l_has_x || r_has_x {
                LogicVec { bits: vec![LogicVal::X; max_width], width: max_width }
            } else {
                let l = lhs_ext.to_u64();
                let r = rhs_ext.to_u64();
                if r == 0 {
                    LogicVec { bits: vec![LogicVal::X; max_width], width: max_width }
                } else {
                    LogicVec::from_u64(l % r, max_width)
                }
            }
        }
        BinaryIrOp::Power => {
            let l_has_x = lhs_ext.bits.iter().any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            let r_has_x = rhs_ext.bits.iter().any(|b| *b == LogicVal::X || *b == LogicVal::Z);
            if l_has_x || r_has_x {
                LogicVec { bits: vec![LogicVal::X; max_width], width: max_width }
            } else {
                LogicVec::from_u64(lhs_ext.to_u64().wrapping_pow(rhs_ext.to_u64() as u32), max_width)
            }
        }
        BinaryIrOp::Eq | BinaryIrOp::CaseEq => {
            let eq = lhs_ext.bits == rhs_ext.bits;
            LogicVec::from_u64(if eq { 1 } else { 0 }, 1)
        }
        BinaryIrOp::Neq | BinaryIrOp::CaseNeq => {
            let eq = lhs_ext.bits == rhs_ext.bits;
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
            let l = lhs_ext.to_u64();
            let r = rhs_ext.to_u64();
            LogicVec::from_u64(if l < r { 1 } else { 0 }, 1)
        }
        BinaryIrOp::Le => {
            let l = lhs_ext.to_u64();
            let r = rhs_ext.to_u64();
            LogicVec::from_u64(if l <= r { 1 } else { 0 }, 1)
        }
        BinaryIrOp::Gt => {
            let l = lhs_ext.to_u64();
            let r = rhs_ext.to_u64();
            LogicVec::from_u64(if l > r { 1 } else { 0 }, 1)
        }
        BinaryIrOp::Ge => {
            let l = lhs_ext.to_u64();
            let r = rhs_ext.to_u64();
            LogicVec::from_u64(if l >= r { 1 } else { 0 }, 1)
        }
        BinaryIrOp::BitAnd => bitwise_op(&lhs_ext, &rhs_ext, |a, b| {
            match (a, b) {
                (LogicVal::One, LogicVal::One) => LogicVal::One,
                (LogicVal::Zero, _) | (_, LogicVal::Zero) => LogicVal::Zero,
                (LogicVal::X, _) | (_, LogicVal::X) => LogicVal::X,
                _ => LogicVal::X,
            }
        }),
        BinaryIrOp::BitOr => bitwise_op(&lhs_ext, &rhs_ext, |a, b| {
            match (a, b) {
                (LogicVal::Zero, LogicVal::Zero) => LogicVal::Zero,
                (LogicVal::One, _) | (_, LogicVal::One) => LogicVal::One,
                (LogicVal::X, _) | (_, LogicVal::X) => LogicVal::X,
                _ => LogicVal::X,
            }
        }),
        BinaryIrOp::BitXor => bitwise_op(&lhs_ext, &rhs_ext, |a, b| {
            match (a, b) {
                (LogicVal::Zero, LogicVal::Zero) => LogicVal::Zero,
                (LogicVal::One, LogicVal::One) => LogicVal::Zero,
                (LogicVal::Zero, LogicVal::One) => LogicVal::One,
                (LogicVal::One, LogicVal::Zero) => LogicVal::One,
                _ => LogicVal::X,
            }
        }),
        BinaryIrOp::BitXnor => {
            let xor = eval_binary(BinaryIrOp::BitXor, lhs, rhs);
            eval_unary(UnaryIrOp::BitNot, &xor)
        }
        BinaryIrOp::Shl => {
            let shift = rhs_ext.to_u64() as usize;
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
        BinaryIrOp::Shr => {
            let shift = rhs_ext.to_u64() as usize;
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
        BinaryIrOp::Sshl => {
            let shift = rhs_ext.to_u64() as usize;
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
        BinaryIrOp::Sshr => {
            let shift = rhs_ext.to_u64() as usize;
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

fn bitwise_op<F>(lhs: &LogicVec, rhs: &LogicVec, op: F) -> LogicVec
    where F: Fn(LogicVal, LogicVal) -> LogicVal
{
    let width = lhs.width.max(rhs.width);
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
