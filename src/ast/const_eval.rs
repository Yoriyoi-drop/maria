use std::collections::HashMap;

use crate::ast::expr::{BinaryOp, Expr, UnaryOp, Value};

/// Encode a short string as i64 for parameter comparison purposes.
/// Strings up to 8 characters are encoded as little-endian bytes.
pub fn string_to_i64(s: &str) -> i64 {
    let bytes = s.as_bytes();
    let mut val: i64 = 0;
    for (i, &b) in bytes.iter().enumerate().take(8) {
        val |= (b as i64) << (i * 8);
    }
    val
}

pub fn const_eval_simple(expr: &Expr) -> Result<i64, String> {
    match expr {
        Expr::Value(Value::Decimal(n)) => Ok(*n),
        Expr::Value(Value::Binary { bits, .. }) => {
            i64::from_str_radix(&bits.replace('x', "0").replace('z', "0"), 2)
                .map_err(|_| "bad binary".to_string())
        }
        Expr::Value(Value::Hex { bits, .. }) => {
            i64::from_str_radix(&bits.replace('x', "0").replace('z', "0"), 16)
                .map_err(|_| "bad hex".to_string())
        }
        Expr::Value(Value::Octal { bits, .. }) => {
            i64::from_str_radix(&bits.replace('x', "0").replace('z', "0"), 8)
                .map_err(|_| "bad octal".to_string())
        }
        Expr::Ident(ref s) if s == "1" => Ok(1),
        Expr::MethodCall { .. } => Err("method calls are not simple constants".to_string()),
        Expr::MemberAccess { .. } => Err("member access is not a simple constant".to_string()),
        _ => Err("not a simple constant".to_string()),
    }
}

pub fn const_eval_with_params(
    expr: &Expr,
    param_vals: &HashMap<String, i64>,
) -> Result<i64, String> {
    match expr {
        Expr::Value(Value::Decimal(n)) => Ok(*n),
        Expr::Value(Value::Binary { bits, .. }) => {
            i64::from_str_radix(&bits.replace('x', "0").replace('z', "0"), 2)
                .map_err(|_| "bad binary".to_string())
        }
        Expr::Value(Value::Hex { bits, .. }) => {
            i64::from_str_radix(&bits.replace('x', "0").replace('z', "0"), 16)
                .map_err(|_| "bad hex".to_string())
        }
        Expr::Value(Value::Octal { bits, .. }) => {
            i64::from_str_radix(&bits.replace('x', "0").replace('z', "0"), 8)
                .map_err(|_| "bad octal".to_string())
        }
        Expr::String(s) => Ok(string_to_i64(s)),
        Expr::Ident(name) => {
            if let Some(&val) = param_vals.get(name) {
                Ok(val)
            } else if name == "1" {
                Ok(1)
            } else if name.starts_with('$') {
                Err(format!(
                    "cannot evaluate system function '{}' in constant context",
                    name
                ))
            } else {
                Err(format!("'{}' not found in parameter context", name))
            }
        }
        Expr::UnaryOp {
            op: UnaryOp::Minus,
            expr: inner,
        } => Ok(-const_eval_with_params(inner, param_vals)?),
        Expr::UnaryOp {
            op: UnaryOp::Plus,
            expr: inner,
        } => Ok(const_eval_with_params(inner, param_vals)?),
        Expr::UnaryOp {
            op: UnaryOp::BitNot,
            expr: inner,
        } => Ok(!const_eval_with_params(inner, param_vals)?),
        Expr::BinaryOp {
            op: BinaryOp::Add,
            lhs,
            rhs,
        } => {
            Ok(const_eval_with_params(lhs, param_vals)? + const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp {
            op: BinaryOp::Sub,
            lhs,
            rhs,
        } => {
            Ok(const_eval_with_params(lhs, param_vals)? - const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp {
            op: BinaryOp::Mul,
            lhs,
            rhs,
        } => {
            Ok(const_eval_with_params(lhs, param_vals)? * const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp {
            op: BinaryOp::Div,
            lhs,
            rhs,
        } => {
            let r = const_eval_with_params(rhs, param_vals)?;
            if r == 0 {
                return Err("division by zero in constant expression".to_string());
            }
            Ok(const_eval_with_params(lhs, param_vals)? / r)
        }
        Expr::BinaryOp {
            op: BinaryOp::Power,
            lhs,
            rhs,
        } => {
            let base = const_eval_with_params(lhs, param_vals)?;
            let exp = const_eval_with_params(rhs, param_vals)? as u32;
            Ok(base.pow(exp))
        }
        Expr::BinaryOp {
            op: BinaryOp::Mod,
            lhs,
            rhs,
        } => {
            Ok(const_eval_with_params(lhs, param_vals)? % const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp {
            op: BinaryOp::Eq,
            lhs,
            rhs,
        } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l == r { 1 } else { 0 })
        }
        Expr::BinaryOp {
            op: BinaryOp::Neq,
            lhs,
            rhs,
        } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l != r { 1 } else { 0 })
        }
        Expr::BinaryOp {
            op: BinaryOp::Lt,
            lhs,
            rhs,
        } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l < r { 1 } else { 0 })
        }
        Expr::BinaryOp {
            op: BinaryOp::Le,
            lhs,
            rhs,
        } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l <= r { 1 } else { 0 })
        }
        Expr::BinaryOp {
            op: BinaryOp::Gt,
            lhs,
            rhs,
        } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l > r { 1 } else { 0 })
        }
        Expr::BinaryOp {
            op: BinaryOp::Ge,
            lhs,
            rhs,
        } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l >= r { 1 } else { 0 })
        }
        Expr::BinaryOp {
            op: BinaryOp::LogicalAnd,
            lhs,
            rhs,
        } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l != 0 && r != 0 { 1 } else { 0 })
        }
        Expr::BinaryOp {
            op: BinaryOp::LogicalOr,
            lhs,
            rhs,
        } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l != 0 || r != 0 { 1 } else { 0 })
        }
        Expr::BinaryOp {
            op: BinaryOp::BitAnd,
            lhs,
            rhs,
        } => {
            Ok(const_eval_with_params(lhs, param_vals)? & const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp {
            op: BinaryOp::BitOr,
            lhs,
            rhs,
        } => {
            Ok(const_eval_with_params(lhs, param_vals)? | const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp {
            op: BinaryOp::BitXor,
            lhs,
            rhs,
        } => {
            Ok(const_eval_with_params(lhs, param_vals)? ^ const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp {
            op: BinaryOp::BitXnor,
            lhs,
            rhs,
        } => {
            Ok(!(const_eval_with_params(lhs, param_vals)?
                ^ const_eval_with_params(rhs, param_vals)?))
        }
        Expr::BinaryOp {
            op: BinaryOp::Shl,
            lhs,
            rhs,
        } => Ok(
            const_eval_with_params(lhs, param_vals)? << const_eval_with_params(rhs, param_vals)?
        ),
        Expr::BinaryOp {
            op: BinaryOp::Shr,
            lhs,
            rhs,
        } => Ok(
            const_eval_with_params(lhs, param_vals)? >> const_eval_with_params(rhs, param_vals)?
        ),
        Expr::BinaryOp {
            op: BinaryOp::Sshl,
            lhs,
            rhs,
        } => Ok(
            const_eval_with_params(lhs, param_vals)? << const_eval_with_params(rhs, param_vals)?
        ),
        Expr::BinaryOp {
            op: BinaryOp::Sshr,
            lhs,
            rhs,
        } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(l >> r)
        }
        Expr::BinaryOp {
            op: BinaryOp::CaseEq,
            lhs,
            rhs,
        } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l == r { 1 } else { 0 })
        }
        Expr::BinaryOp {
            op: BinaryOp::CaseNeq,
            lhs,
            rhs,
        } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l != r { 1 } else { 0 })
        }
        Expr::BinaryOp {
            op: BinaryOp::EqWild,
            lhs,
            rhs,
        } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l == r { 1 } else { 0 })
        }
        Expr::BinaryOp {
            op: BinaryOp::NeqWild,
            lhs,
            rhs,
        } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l != r { 1 } else { 0 })
        }
        Expr::UnaryOp {
            op: UnaryOp::Not,
            expr: inner,
        } => {
            let v = const_eval_with_params(inner, param_vals)?;
            Ok(if v == 0 { 1 } else { 0 })
        }
        Expr::UnaryOp {
            op: UnaryOp::ReductionAnd,
            expr: inner,
        } => {
            let v = const_eval_with_params(inner, param_vals)?;
            Ok(if v != 0 && v != -1 { 0 } else { 1 })
        }
        Expr::UnaryOp {
            op: UnaryOp::ReductionNand,
            expr: inner,
        } => {
            let v = const_eval_with_params(inner, param_vals)?;
            Ok(if v != 0 && v != -1 { 1 } else { 0 })
        }
        Expr::UnaryOp {
            op: UnaryOp::ReductionOr,
            expr: inner,
        } => {
            let v = const_eval_with_params(inner, param_vals)?;
            Ok(if v == 0 { 0 } else { 1 })
        }
        Expr::UnaryOp {
            op: UnaryOp::ReductionNor,
            expr: inner,
        } => {
            let v = const_eval_with_params(inner, param_vals)?;
            Ok(if v == 0 { 1 } else { 0 })
        }
        Expr::UnaryOp {
            op: UnaryOp::ReductionXor,
            expr: inner,
        } => {
            let v = const_eval_with_params(inner, param_vals)?;
            Ok((v.count_ones() & 1) as i64)
        }
        Expr::UnaryOp {
            op: UnaryOp::ReductionXnor,
            expr: inner,
        } => {
            let v = const_eval_with_params(inner, param_vals)?;
            Ok(1 - (v.count_ones() & 1) as i64)
        }
        Expr::TernaryOp {
            cond,
            true_expr,
            false_expr,
        } => {
            let cond_val = const_eval_with_params(cond, param_vals)?;
            if cond_val != 0 {
                const_eval_with_params(true_expr, param_vals)
            } else {
                const_eval_with_params(false_expr, param_vals)
            }
        }
        Expr::Paren(inner) => const_eval_with_params(inner, param_vals),
        Expr::ScopedIdent { package, item } => {
            let qualified = format!("{}::{}", package, item);
            if let Some(&val) = param_vals.get(&qualified) {
                Ok(val)
            } else {
                Err(format!("cannot evaluate package parameter '{}'", qualified))
            }
        }
        Expr::MethodCall { .. } => {
            Err("method calls not allowed in constant expression".to_string())
        }
        Expr::MemberAccess { .. } => {
            Err("member access not allowed in constant expression".to_string())
        }
        Expr::Inside {
            expr: inner,
            range_list,
        } => {
            let val = const_eval_with_params(inner, param_vals)?;
            for item in range_list {
                if const_eval_with_params(item, param_vals)? == val {
                    return Ok(1);
                }
            }
            Ok(0)
        }
        Expr::BitSelect { expr, index } => {
            let base_val = const_eval_with_params(expr, param_vals)?;
            let idx = const_eval_with_params(index, param_vals)?;
            Ok((base_val >> idx) & 1)
        }
        Expr::RangeSelect { expr, msb, lsb } => {
            let base_val = const_eval_with_params(expr, param_vals)?;
            let m = const_eval_with_params(msb, param_vals)?;
            let l = const_eval_with_params(lsb, param_vals)?;
            let width = (m - l + 1) as usize;
            if width >= 64 {
                Ok(base_val >> l)
            } else {
                let mask = (1i64 << width) - 1;
                Ok((base_val >> l) & mask)
            }
        }
        Expr::FuncCall { name, args } if name == "$clog2" => {
            if let Some(arg) = args.first() {
                let v = const_eval_with_params(arg, param_vals)?;
                if v <= 1 {
                    Ok(0)
                } else {
                    let n = v as u64;
                    let msb = (64 - n.leading_zeros()) as i64;
                    if n.is_power_of_two() {
                        Ok(msb - 1)
                    } else {
                        Ok(msb)
                    }
                }
            } else {
                Ok(0)
            }
        }
        Expr::FuncCall { name, args } if name == "$bits" || name == "$size" => {
            if let Some(arg) = args.first() {
                const_eval_with_params(arg, param_vals)
            } else {
                Ok(0)
            }
        }
        Expr::FuncCall { name, .. } if name.starts_with('$') => Err(format!(
            "cannot evaluate system function '{}' in constant context",
            name
        )),
        _ => Err(format!(
            "non-constant expression in parameter context: {:?}",
            expr
        )),
    }
}
