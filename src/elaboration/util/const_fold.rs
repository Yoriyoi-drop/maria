//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan util.rs (SRP Refactoring).
//! Tanggung jawab: Constant folding & value-to-LogicVec conversion.
//!
//! Fungsi:
//!   - const_eval_params()   — evaluasi konstanta dengan parameter
//!   - try_fold_const()      — coba fold expression ke IrExpr::Const
//!   - value_to_logicvec()   — konversi Value AST ke LogicVec IR
//!
//! ──────────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;

use crate::ast::types::const_eval_with_params;
use crate::ast::*;
use crate::intern::Symbol;
use crate::ir::*;

/// Evaluasi expression konstanta dengan parameter yang diberikan.
/// Delegasi ke const_eval_with_params dari ast/types.
pub fn const_eval_params(expr: &Expr, params: &HashMap<Symbol, i64>) -> Result<i64, String> {
    const_eval_with_params(expr, params)
}

/// Coba fold expression konstanta menjadi IrExpr::Const.
/// Mengembalikan Some(Const) jika konstanta bisa dievaluasi, None jika tidak.
pub fn try_fold_const(
    expr: &Expr,
    params: &HashMap<Symbol, i64>,
) -> Result<Option<IrExpr>, String> {
    match const_eval_with_params(expr, params) {
        Ok(val) => {
            let abs = val.unsigned_abs();
            let min_width = if val >= 0 {
                if val == 0 {
                    1
                } else {
                    64 - (abs.leading_zeros() as usize)
                }
            } else {
                64 - (abs.leading_zeros() as usize) + 1
            };
            let width = min_width.max(32);
            Ok(Some(IrExpr::Const(LogicVec::from_u64(val as u64, width))))
        }
        Err(_) => Ok(None),
    }
}

/// Konversi Value AST (Decimal, Binary, Hex, Octal, Real) ke LogicVec IR.
pub fn value_to_logicvec(val: &Value) -> LogicVec {
    match val {
        Value::Decimal(n) => {
            let abs = n.unsigned_abs();
            let min_width = if *n == 0 {
                1
            } else {
                64 - (abs.leading_zeros() as usize)
            };
            let width = min_width.max(32);
            let mut lv = LogicVec::from_u64(abs, width);
            if *n < 0 {
                for b in lv.bits.iter_mut() {
                    *b = match b {
                        LogicVal::Zero => LogicVal::One,
                        LogicVal::One => LogicVal::Zero,
                        _ => LogicVal::X,
                    };
                }
                let mut carry = true;
                for b in lv.bits.iter_mut() {
                    if carry {
                        match b {
                            LogicVal::Zero => {
                                *b = LogicVal::One;
                                carry = false;
                            }
                            LogicVal::One => {
                                *b = LogicVal::Zero;
                            }
                            _ => {}
                        }
                    }
                }
            }
            lv
        }
        Value::Binary { bits, width, .. } => {
            let w = width.unwrap_or(bits.len());
            let mut vec = LogicVec::new(w);
            for (i, c) in bits.chars().rev().enumerate() {
                if i >= w {
                    break;
                }
                vec.bits[i] = match c {
                    '0' => LogicVal::Zero,
                    '1' => LogicVal::One,
                    'x' | 'X' => LogicVal::X,
                    'z' | 'Z' => LogicVal::Z,
                    '_' => continue,
                    _ => LogicVal::X,
                };
            }
            vec
        }
        Value::Hex { bits, width, .. } => {
            let w = width.unwrap_or(bits.len() * 4);
            let mut vec = LogicVec::new(w);
            let digits: String = bits.chars().filter(|c| *c != '_').collect();
            for (i, c) in digits.chars().rev().enumerate() {
                let hex_val = c.to_digit(16).unwrap_or(0);
                for j in 0..4 {
                    let bit_idx = i * 4 + j;
                    if bit_idx >= w {
                        break;
                    }
                    vec.bits[bit_idx] = if (hex_val >> j) & 1 == 1 {
                        LogicVal::One
                    } else {
                        LogicVal::Zero
                    };
                }
            }
            vec
        }
        Value::Octal { bits, width, .. } => {
            let w = width.unwrap_or(bits.len() * 3);
            let mut vec = LogicVec::new(w);
            let digits: String = bits.chars().filter(|c| *c != '_').collect();
            for (i, c) in digits.chars().rev().enumerate() {
                let oct_val = c.to_digit(8).unwrap_or(0);
                for j in 0..3 {
                    let bit_idx = i * 3 + j;
                    if bit_idx >= w {
                        break;
                    }
                    vec.bits[bit_idx] = if (oct_val >> j) & 1 == 1 {
                        LogicVal::One
                    } else {
                        LogicVal::Zero
                    };
                }
            }
            vec
        }
        Value::Real(r) => LogicVec::from_u64(r.to_bits(), 64),
    }
}
