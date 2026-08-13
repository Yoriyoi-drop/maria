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

use maria_ast::types::const_eval_with_params;
use maria_ast::*;
use maria_core::intern::Symbol;
use maria_ir::*;

/// Evaluasi expression konstanta dengan parameter yang diberikan.
/// Delegasi ke const_eval_with_params dari ast/types.
pub fn const_eval_params(expr: &Expr, params: &HashMap<Symbol, i64>) -> Result<i64, String> {
    const_eval_with_params(expr, params)
}

/// Perkiraan lebar sebenarnya dari ekspresi konstanta murni (tanpa signal),
/// dihitung dari struktur AST — bukan dari nilai hasil eval.
///
/// Latar belakang: `try_fold_const` mengevaluasi nilai lalu memakai lebar
/// minimal `min_width.max(32)`. Untuk konstanta yang nilainya kecil/0 tapi
/// lebar aslinya lebar (mis. `{384{1'b0}}` → 0, atau `512'h0 ^ 512'h36`),
/// lebar itu salah menjadi 32 — memicu WR0102 palsu seperti
/// `i_pad_256 = {secret_key_i[1023:896], {(BlockSizeSHA256-128){1'b0}}}`
/// (rhs dihitung 160 = 128+32, harusnya 512). Dengan lebar dari AST, hasil
/// fold mempertahankan lebar sebenarnya.
pub(crate) fn const_fold_width(expr: &Expr, params: &HashMap<Symbol, i64>) -> Option<usize> {
    match expr {
        Expr::Value(v) => Some(match v {
            Value::Binary { bits, width, .. } => width.unwrap_or_else(|| bits.len()),
            Value::Hex { bits, width, .. } => width.unwrap_or_else(|| bits.len() * 4),
            Value::Octal { bits, width, .. } => width.unwrap_or_else(|| bits.len() * 3),
            Value::Decimal(n) => {
                let abs = n.unsigned_abs();
                let w = if *n == 0 { 1 } else { 64 - abs.leading_zeros() as usize };
                w.max(32)
            }
            Value::Real(_) => 64,
        }),
        Expr::FillLit(_) => Some(1),
        Expr::Replicate { count, expr: inner } => {
            let c = const_eval_with_params(count, params).ok()? as usize;
            Some(c.saturating_mul(const_fold_width(inner, params)?))
        }
        Expr::Concat(exprs) => {
            let mut total = 0usize;
            for e in exprs {
                total = total.saturating_add(const_fold_width(e, params)?);
            }
            Some(total)
        }
        Expr::Paren(inner) => const_fold_width(inner, params),
        Expr::CastWidth { width, expr: _ } => {
            let w = const_eval_with_params(width, params).ok()?;
            Some(w.max(1) as usize)
        }
        Expr::String(s) => Some(s.len() * 8),
        Expr::UnaryOp { op, expr: inner } => {
            let w = const_fold_width(inner, params)?;
            Some(match op {
                UnaryOp::Not
                | UnaryOp::ReductionAnd
                | UnaryOp::ReductionNand
                | UnaryOp::ReductionOr
                | UnaryOp::ReductionNor
                | UnaryOp::ReductionXor
                | UnaryOp::ReductionXnor => 1,
                _ => w,
            })
        }
        Expr::BinaryOp { op, lhs, rhs } => {
            let lw = const_fold_width(lhs, params)?;
            let rw = const_fold_width(rhs, params)?;
            let m = lw.max(rw);
            Some(match op {
                BinaryOp::Eq
                | BinaryOp::Neq
                | BinaryOp::CaseEq
                | BinaryOp::CaseNeq
                | BinaryOp::EqWild
                | BinaryOp::NeqWild
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge
                | BinaryOp::LogicalAnd
                | BinaryOp::LogicalOr => 1,
                _ => m,
            })
        }
        Expr::TernaryOp {
            cond: _,
            true_expr,
            false_expr,
        } => {
            let tw = const_fold_width(true_expr, params)?;
            let fw = const_fold_width(false_expr, params)?;
            Some(tw.max(fw))
        }
        _ => None,
    }
}

/// Coba fold expression konstanta menjadi IrExpr::Const.
/// Mengembalikan Some(Const) jika konstanta bisa dievaluasi, None jika tidak.
pub fn try_fold_const(
    expr: &Expr,
    params: &HashMap<Symbol, i64>,
) -> Result<Option<IrExpr>, String> {
    match const_eval_with_params(expr, params) {
        Ok(val) => {
            let width = const_fold_width(expr, params).unwrap_or_else(|| {
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
                min_width.max(32)
            });
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
            // F30 fix review: filter `_` dulu (konsisten dengan Hex/Octal) —
            // kalau di-loop langsung, enumerate menghitung index underscore
            // sehingga digit berikutnya bergeser (mis. `8'b1_010` → salah).
            let digits: String = bits.chars().filter(|c| *c != '_').collect();
            let w = width.unwrap_or(digits.len());
            // F30 fix: literal sized di-zero-extend — bit di atas digit bernilai
            // 0 (bukan X). Sebelumnya `LogicVec::new(w)` (fill X) membuat
            // `16'h6` = 16'hxxxx...0006 → perbandingan `sig == 16'h6` salah
            // (X di operand → hasil X → false). `from_u64` sudah zero-fill;
            // Binary/Hex/Octal harus konsisten.
            let mut vec = LogicVec::fill(LogicVal::Zero, w);
            for (i, c) in digits.chars().rev().enumerate() {
                if i >= w {
                    break;
                }
                vec.bits[i] = match c {
                    '0' => LogicVal::Zero,
                    '1' => LogicVal::One,
                    'x' | 'X' => LogicVal::X,
                    'z' | 'Z' => LogicVal::Z,
                    _ => LogicVal::X,
                };
            }
            vec
        }
        Value::Hex { bits, width, .. } => {
            let w = width.unwrap_or(bits.len() * 4);
            let mut vec = LogicVec::fill(LogicVal::Zero, w);
            let digits: String = bits.chars().filter(|c| *c != '_').collect();
            for (i, c) in digits.chars().rev().enumerate() {
                // F30 fix: digit x/z eksplisit (mis. `8'hx6`) → bit X/Z,
                // bukan 0 (sebelumnya `to_digit(16)` → None → unwrap_or(0)).
                let val = match c {
                    'x' | 'X' => LogicVal::X,
                    'z' | 'Z' => LogicVal::Z,
                    _ => {
                        let hv = c.to_digit(16).unwrap_or(0);
                        for j in 0..4 {
                            let bit_idx = i * 4 + j;
                            if bit_idx >= w {
                                break;
                            }
                            vec.bits[bit_idx] = if (hv >> j) & 1 == 1 {
                                LogicVal::One
                            } else {
                                LogicVal::Zero
                            };
                        }
                        continue;
                    }
                };
                for j in 0..4 {
                    let bit_idx = i * 4 + j;
                    if bit_idx >= w {
                        break;
                    }
                    vec.bits[bit_idx] = val;
                }
            }
            vec
        }
        Value::Octal { bits, width, .. } => {
            let w = width.unwrap_or(bits.len() * 3);
            let mut vec = LogicVec::fill(LogicVal::Zero, w);
            let digits: String = bits.chars().filter(|c| *c != '_').collect();
            for (i, c) in digits.chars().rev().enumerate() {
                // F30 fix: digit x/z eksplisit di octal → bit X/Z.
                let val = match c {
                    'x' | 'X' => LogicVal::X,
                    'z' | 'Z' => LogicVal::Z,
                    _ => {
                        let ov = c.to_digit(8).unwrap_or(0);
                        for j in 0..3 {
                            let bit_idx = i * 3 + j;
                            if bit_idx >= w {
                                break;
                            }
                            vec.bits[bit_idx] = if (ov >> j) & 1 == 1 {
                                LogicVal::One
                            } else {
                                LogicVal::Zero
                            };
                        }
                        continue;
                    }
                };
                for j in 0..3 {
                    let bit_idx = i * 3 + j;
                    if bit_idx >= w {
                        break;
                    }
                    vec.bits[bit_idx] = val;
                }
            }
            vec
        }
        Value::Real(r) => LogicVec::from_u64(r.to_bits(), 64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_ast::Value;

    /// F30 fix: literal sized di-zero-extend — bit di atas digit bernilai 0,
    /// BUKAN X. Sebelumnya `16'h6` = 16'hxxxx...0006 sehingga
    /// `signal == 16'h6` selalu false (X di operand → hasil X → display 0).
    #[test]
    fn value_to_logicvec_zero_extends_hex() {
        let lv = value_to_logicvec(&Value::Hex {
            bits: "6".into(),
            width: Some(16),
            is_signed: false,
        });
        assert_eq!(lv.width, 16);
        assert_eq!(lv.to_u64(), 6);
        // bit 4..=15 harus Zero (bukan X)
        assert!(!lv.bits[4..].iter().any(|b| matches!(b, LogicVal::X | LogicVal::Z)));
        assert_eq!(lv.bits[0], LogicVal::Zero);
        assert_eq!(lv.bits[1], LogicVal::One);
        assert_eq!(lv.bits[2], LogicVal::One);
        assert_eq!(lv.bits[3], LogicVal::Zero);
    }

    #[test]
    fn value_to_logicvec_zero_extends_binary_and_octal() {
        let bin = value_to_logicvec(&Value::Binary {
            bits: "110".into(),
            width: Some(8),
            is_signed: false,
        });
        assert_eq!(bin.width, 8);
        assert_eq!(bin.to_u64(), 6);
        assert!(!bin.bits[3..].iter().any(|b| matches!(b, LogicVal::X | LogicVal::Z)));

        let oct = value_to_logicvec(&Value::Octal {
            bits: "6".into(),
            width: Some(9),
            is_signed: false,
        });
        assert_eq!(oct.width, 9);
        assert_eq!(oct.to_u64(), 6);
        assert!(!oct.bits[3..].iter().any(|b| matches!(b, LogicVal::X | LogicVal::Z)));
    }

    /// F30 fix review: underscore di literal binary tidak boleh menggeser
    /// posisi digit (`8'b1_010` = 10, bukan bit[4] terisi karena underscore).
    #[test]
    fn value_to_logicvec_binary_underscore_no_shift() {
        let lv = value_to_logicvec(&Value::Binary {
            bits: "1_010".into(),
            width: Some(8),
            is_signed: false,
        });
        assert_eq!(lv.to_u64(), 0b1010, "underscore harus diabaikan: {:?}", lv.bits);
        assert_eq!(lv.bits[4], LogicVal::Zero); // posisi underscore = 0
    }

    /// F30: digit X/Z eksplisit tetap dipertahankan (tidak di-zero-kan).
    #[test]
    fn value_to_logicvec_keeps_explicit_x() {
        let lv = value_to_logicvec(&Value::Hex {
            bits: "x6".into(),
            width: Some(8),
            is_signed: false,
        });
        assert_eq!(lv.bits[4], LogicVal::X); // digit x eksplisit
        assert_eq!(lv.bits[1], LogicVal::One);
        assert_eq!(lv.bits[2], LogicVal::One);
    }
}
