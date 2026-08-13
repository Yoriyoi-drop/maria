use std::collections::HashMap;

use crate::expr::{BinaryOp, Expr, UnaryOp, Value};
use maria_core::intern::Symbol;

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

/// Nama fungsi dasar untuk fungsi yang dipanggil secara scoped
/// (`pkg::func(...)`). Mengembalikan bagian setelah `::` terakhir agar
/// dispatch fungsi konstan berlaku global untuk package mana pun.
fn base_func_name(name: &str) -> &str {
    name.rsplit_once("::").map(|(_, f)| f).unwrap_or(name)
}

/// Parse literal berbasis angka (`bits` = digit tanpa prefix) menjadi i64
/// dengan bit-pattern dipertahankan. Nilai ≥ 2^63 (mis. `64'hC0AC29B7C97C50DD`
/// yang dipakai konstanta kriptografi OpenTitan) tidak muat di i64 — parse
/// sebagai u64 lalu wrap. Operasi bit (BitSelect/RangeSelect/`& mask`) pada
/// nilai negatif tetap menghasilkan bit asli karena `>>`/`&` bekerja pada
/// representasi two's complement.
pub fn parse_literal(bits: &str, radix: u32) -> Result<i64, String> {
    u64::from_str_radix(&bits.replace(['x', 'z'], "0"), radix)
        .map(|v| v as i64)
        .map_err(|_| "bad literal".to_string())
}

pub fn const_eval_simple(expr: &Expr) -> Result<i64, String> {
    match expr {
        Expr::Value(Value::Decimal(n)) => Ok(*n),
        Expr::Value(Value::Binary { bits, .. }) => parse_literal(bits, 2),
        Expr::Value(Value::Hex { bits, .. }) => parse_literal(bits, 16),
        Expr::Value(Value::Octal { bits, .. }) => parse_literal(bits, 8),
        Expr::Ident { name: ref s, .. } if s == "1" => Ok(1),
        Expr::MethodCall { .. } => Err("method calls are not simple constants".to_string()),
        Expr::MemberAccess { .. } => Err("member access is not a simple constant".to_string()),
        Expr::StructLit { .. } => Err("struct literal is not a simple constant".to_string()),
        _ => Err("not a simple constant".to_string()),
    }
}

pub fn const_eval_with_params(
    expr: &Expr,
    param_vals: &HashMap<Symbol, i64>,
) -> Result<i64, String> {
    match expr {
        Expr::Value(Value::Decimal(n)) => Ok(*n),
        Expr::Value(Value::Binary { bits, .. }) => parse_literal(bits, 2),
        Expr::Value(Value::Hex { bits, .. }) => parse_literal(bits, 16),
        Expr::Value(Value::Octal { bits, .. }) => parse_literal(bits, 8),
        Expr::String(s) => Ok(string_to_i64(s)),
        Expr::Ident { name, .. } => {
            if let Some(&val) = param_vals.get(name.as_str()) {
                Ok(val)
            } else if name == "1" {
                Ok(1)
            } else if name.starts_with("$") {
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
        Expr::Cast { expr: inner, .. } => const_eval_with_params(inner, param_vals),
        Expr::CastWidth { expr: inner, .. } => const_eval_with_params(inner, param_vals),
        // Replikasi `{N{expr}}` di constant context: pola 1-bit 1 → mask N
        // ones; pola 0 → 0. Pola lain di-approximate (ulang bit-pattern).
        Expr::Replicate { count, expr } => {
            let n = const_eval_with_params(count, param_vals)?;
            let v = const_eval_with_params(expr, param_vals)?;
            let n = n.max(0).min(63) as u32;
            if v == 0 {
                Ok(0)
            } else if v == 1 {
                Ok((1u64.wrapping_shl(n)).wrapping_sub(1) as i64)
            } else if n == 0 {
                Ok(0)
            } else {
                let mut acc: u64 = 0;
                let w = 63.min(n) as u32;
                let pattern = (v as u64) & ((1u64 << w).wrapping_sub(1));
                for _ in 0..n {
                    acc = (acc << w) | pattern;
                }
                Ok(acc as i64)
            }
        }
        // Struct literal utuh tidak bisa di-const-eval sebagai skalar tanpa
        // layout typedef. Nilai 0 (perilaku lama — pola bernama sebelumnya
        // di-discard jadi 0) supaya localparam struct tetap terdaftar. Member
        // access (`P.offset`) tetap benar via key `base.field` di atas.
        Expr::StructLit { .. } => Ok(0),
        // Fill literal `'0`/`'1` — nilai 0 untuk konstanta (localparam struct
        // seperti `mac_bignum_contrl_t ControlDefault = '0` di
        // otbn_mac_bignum_fsm, `sha_word64_t ZeroWord = '0` di prim_sha2).
        Expr::FillLit(_) => Ok(0),
        Expr::ScopedIdent { package, item, .. } => {
            let qualified = Symbol::intern(&format!("{}::{}", package, item));
            if let Some(&val) = param_vals.get(&qualified) {
                return Ok(val);
            }
            // Juga coba tanpa package prefix — enum member yang sudah di-flatten
            // ke param_vals tanpa qualified name (mis. dari `import pkg::*`).
            if let Some(&val) = param_vals.get(item) {
                return Ok(val);
            }
            // `pkg::TypeName` (typedef/enum type) dipakai sebagai type-cast argument
            // atau di ekspresi generate — ini bukan nilai integer yang bisa dievaluasi.
            // Kembalikan error yang informatif; generate.rs akan menangani kasus ini
            // sebagai warning (member access / type expression tidak bisa di-const-eval).
            Err(format!("cannot evaluate package parameter '{}'", qualified))
        }
        Expr::MethodCall { .. } => {
            Err("method calls not allowed in constant expression".to_string())
        }
        Expr::MemberAccess { obj, field } => {
            // Struct field lookup via flattened keys `base.field` (mis.
            // `PartInfo[k].offset`, `hw2reg.key.q`) — dipakai generate if /
            // konstanta dengan struct localparam array.
            if let Some(bk) = expr_base_key(obj, param_vals) {
                let key = format!("{}.{}", bk, field.as_str());
                if let Some(&v) = param_vals.get(key.as_str()) {
                    return Ok(v);
                }
                if std::env::var("DBG_MEMBER").is_ok() {
                    eprintln!(
                        "[DBG-MEMBER] key '{}' NOT FOUND (obj={:?} field={} bk={})",
                        key, obj, field.as_str(), bk
                    );
                }
            } else if std::env::var("DBG_MEMBER").is_ok() {
                eprintln!(
                    "[DBG-MEMBER-NO-BASE] obj={:?} field={}",
                    obj, field.as_str()
                );
            }
            Err("member access not allowed in constant expression".to_string())
        }
        Expr::Inside {
            expr: inner,
            range_list,
        } => {
            let val = const_eval_with_params(inner, param_vals)?;
            for item in range_list {
                // Range inside `{[a:b], c}` — parser menyisipkan RangeSelect
                // dengan base literal 0 sebagai penanda rentang [lsb, msb].
                // HANYA base literal 0 yang dimaknai rentang — slice ekspresi
                // user (`inside {y[3:0]}`) tetap dievaluasi sebagai bit-slice
                // agar tidak salah diartikan sebagai rentang.
                if let Expr::RangeSelect { expr: base, msb, lsb } = item {
                    if matches!(base.as_ref(), Expr::Value(Value::Decimal(0))) {
                        // `inside {[a:b]}`: msb=a adalah batas BAWAH, lsb=b batas atas.
                        let lo = const_eval_with_params(msb, param_vals)?;
                        let hi = const_eval_with_params(lsb, param_vals)?;
                        if val >= lo && val <= hi {
                            return Ok(1);
                        }
                    } else if const_eval_with_params(item, param_vals)? == val {
                        return Ok(1);
                    }
                } else if const_eval_with_params(item, param_vals)? == val {
                    return Ok(1);
                }
            }
            Ok(0)
        }
        Expr::BitSelect { expr, index } => {
            // Array element lookup via flattened keys `name[idx]` (array params)
            if let Expr::Ident { name, .. } = expr.as_ref() {
                let idx = const_eval_with_params(index, param_vals)?;
                let key = format!("{}[{}]", name.as_str(), idx);
                if let Some(&v) = param_vals.get(key.as_str()) {
                    return Ok(v);
                }
            }
            // 2D array lookup via flattened keys `name[r][c]` (mis. PiRotate [5][5]).
            // `PiRotate[r][c]` → BitSelect(BitSelect(Ident PiRotate, r), c); cari
            // key `PiRotate[r][c]` langsung.
            if let Expr::BitSelect { expr: inner, index: row_idx } = expr.as_ref() {
                if let Expr::Ident { name, .. } = inner.as_ref() {
                    let r = const_eval_with_params(row_idx, param_vals)?;
                    let c = const_eval_with_params(index, param_vals)?;
                    let key = format!("{}[{}][{}]", name.as_str(), r, c);
                    if let Some(&v) = param_vals.get(key.as_str()) {
                        return Ok(v);
                    }
                }
            }
            let base_val = const_eval_with_params(expr, param_vals)?;
            let idx = const_eval_with_params(index, param_vals)?;
            if idx < 0 || idx >= 64 {
                return Ok(0);
            }
            Ok((base_val >> idx) & 1)
        }
        Expr::RangeSelect { expr, msb, lsb } => {
            let base_val = const_eval_with_params(expr, param_vals)?;
            let m = const_eval_with_params(msb, param_vals)?;
            let l = const_eval_with_params(lsb, param_vals)?;
            if l < 0 || l >= 64 {
                return Ok(0);
            }
            let width = (m - l + 1) as usize;
            if width >= 64 {
                Ok(base_val >> l)
            } else {
                let mask = (1i64 << width) - 1;
                Ok((base_val >> l) & mask)
            }
        }
        // Indexed part-select `[base +: width]` (pola OpenTitan:
        // `localparam bit [AW-1:0] TopAddr = TopAddrInt[0 +: AW];`). Maria
        // mengasumsikan arah `+:`. Tanpa ini localparam semacam itu gagal
        // di-const-eval → tidak terdaftar → "signal not found" di pemakaian.
        Expr::PartSelect { expr, base, width } => {
            let src = const_eval_with_params(expr, param_vals)?;
            let b = const_eval_with_params(base, param_vals)?;
            let w = const_eval_with_params(width, param_vals)?;
            if w <= 0 {
                return Err("part-select width must be positive".to_string());
            }
            let width = w as usize;
            let lsb = b;
            if lsb < 0 || lsb >= 64 {
                return Ok(0);
            }
            if width >= 64 {
                Ok(src >> lsb)
            } else {
                let mask = (1i64 << width) - 1;
                Ok((src >> lsb) & mask)
            }
        }
        Expr::FuncCall { name, args, .. } if name == "$clog2" => {
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
        // OpenTitan prim_util_pkg::vbits(value) = (value == 1) ? 1 : $clog2(value)
        Expr::FuncCall { name, args, .. } if base_func_name(name.as_str()) == "vbits" => {
            let v = const_eval_with_params(args.first().ok_or("vbits needs 1 arg")?, param_vals)?;
            Ok(if v == 1 { 1 } else {
                let n = v as u64;
                let msb = (64 - n.leading_zeros()) as i64;
                if n.is_power_of_two() { msb - 1 } else { msb }
            })
        }
        // OpenTitan prim_util_pkg::ceil_div(a, b) = ceiling division
        Expr::FuncCall { name, args, .. } if base_func_name(name.as_str()) == "ceil_div" => {
            let a = const_eval_with_params(args.first().ok_or("ceil_div needs 2 args")?, param_vals)?;
            let b = const_eval_with_params(args.get(1).ok_or("ceil_div needs 2 args")?, param_vals)?;
            if b == 0 {
                return Err("division by zero in ceil_div".to_string());
            }
            Ok(if a % b != 0 { a / b + 1 } else { a / b })
        }
        // OpenTitan prim_secded_pkg::get_synd_width(sd_type, width) — lebar
        // syndrome ECC per tipe & lebar data (tabel konstanta; tipe enum:
        // SecdedHsiao=0, SecdedHamming=1, SecdedInvHsiao=2, SecdedInvHamming=3).
        Expr::FuncCall { name, args, .. } if base_func_name(name.as_str()) == "get_synd_width" => {
            let sd = const_eval_with_params(args.first().ok_or("get_synd_width needs 2 args")?, param_vals)?;
            let w = const_eval_with_params(args.get(1).ok_or("get_synd_width needs 2 args")?, param_vals)?;
            let synd = match (sd, w) {
                (0, 16) | (2, 16) => 6,
                (0, 22) | (2, 22) => 6,
                (0, 32) | (2, 32) => 7,
                (0, 57) | (2, 57) => 7,
                (0, 64) | (2, 64) => 8,
                (1, 16) | (3, 16) => 6,
                (1, 32) | (3, 32) => 7,
                (1, 64) | (3, 64) => 8,
                (1, 68) | (3, 68) => 8,
                _ => 0,
            };
            Ok(synd)
        }
        // OpenTitan prim_secded_pkg::is_width_valid(sd_type, width) — apakah
        // kombinasi tipe+lebar didukung (tabel konstanta).
        Expr::FuncCall { name, args, .. } if base_func_name(name.as_str()) == "is_width_valid" => {
            let sd = const_eval_with_params(args.first().ok_or("is_width_valid needs 2 args")?, param_vals)?;
            let w = const_eval_with_params(args.get(1).ok_or("is_width_valid needs 2 args")?, param_vals)?;
            let valid = match (sd, w) {
                (0, 16) | (0, 22) | (0, 32) | (0, 57) | (0, 64)
                | (2, 16) | (2, 22) | (2, 32) | (2, 57) | (2, 64)
                | (1, 16) | (1, 32) | (1, 64) | (1, 68)
                | (3, 16) | (3, 32) | (3, 64) | (3, 68) => 1,
                _ => 0,
            };
            Ok(valid)
        }
        // Replikasi `{N{expr}}` — pola umum `{W{1'b1}}` untuk mask / literal
        // konstanta di localparam/param (mis. `{32 - $bits(...) - 1{1'b0}}` di
        // ibex_cs_registers, `{BeWidth{1'b1}}` di dm_mem). Nilai = pola diulang
        // N kali; lebar pola dari literal eksplisit atau bit-length nilai.
        Expr::Replicate { count, expr } => {
            let n = const_eval_with_params(count, param_vals)?;
            let v = const_eval_with_params(expr, param_vals)?;
            let n = n.max(0).min(63) as u32;
            if v == 0 {
                Ok(0)
            } else {
                let w: u32 = match expr.as_ref() {
                    Expr::Value(Value::Hex { bits, width, .. }) => {
                        width.unwrap_or(bits.len() * 4).max(1) as u32
                    }
                    Expr::Value(Value::Binary { bits, width, .. }) => {
                        width.unwrap_or(bits.len()).max(1) as u32
                    }
                    Expr::Value(Value::Octal { bits, width, .. }) => {
                        width.unwrap_or(bits.len() * 3).max(1) as u32
                    }
                    _ => (64u32 - (v as u64).leading_zeros()).max(1),
                };
                let w = w.min(63);
                let pattern = (v as u64) & ((1u64 << w).wrapping_sub(1));
                let mut acc: u64 = 0;
                let mut total_w: u32 = 0;
                for _ in 0..n {
                    acc = (acc << w) | pattern;
                    total_w = total_w.saturating_add(w);
                    if total_w >= 63 {
                        break;
                    }
                }
                Ok(acc as i64)
            }
        }
        // Concat `{a, b, c}` — elemen MSB→LSB. Lebar elemen: eksplisit untuk
        // literal bertipe (`4'h0`), bit-length nilai untuk ident/ekspresi lain
        // (mis. `{4'h0, dm::DataCount}` = 8'h02, bukan 2<<4).
        Expr::Concat(elems) => {
            // String concat ({`"hello", `" `", `"world`"}) harus dievaluasi di
            // runtime (byte per char), bukan di-const-fold sebagai bit-pattern
            // (i64 + bit-width merusak urutan byte). Kembalikan Err agar
            // elaborator menurunkan concat biasa → simulator meng-eval dengan
            // benar.
            if elems.iter().any(|e| matches!(e, Expr::String(_))) {
                return Err("string concat is not a constant expression".to_string());
            }
            let mut acc: u64 = 0;
            let mut shift: u32 = 0;
            for elem in elems.iter().rev() {
                let v = const_eval_with_params(elem, param_vals)?;
                let w: u32 = match elem {
                    Expr::Value(Value::Hex { bits, width, .. }) => {
                        width.unwrap_or(bits.len() * 4).max(1) as u32
                    }
                    Expr::Value(Value::Binary { bits, width, .. }) => {
                        width.unwrap_or(bits.len()).max(1) as u32
                    }
                    Expr::Value(Value::Octal { bits, width, .. }) => {
                        width.unwrap_or(bits.len() * 3).max(1) as u32
                    }
                    _ => (64u32 - (v as u64).leading_zeros()).max(1),
                };
                let w = w.min(63);
                acc |= ((v as u64) & ((1u64 << w).wrapping_sub(1)))
                    .wrapping_shl(shift.min(63));
                shift = shift.saturating_add(w);
                if shift >= 63 {
                    break;
                }
            }
            Ok(acc as i64)
        }
        // OpenTitan entropy_src_pkg::bucket_ht_data_width(w) = min(w, 4)
        // (BucketHtDataMaxWidth = 4) — lebar data per bucket health-test.
        Expr::FuncCall { name, args, .. } if base_func_name(name.as_str()) == "bucket_ht_data_width" => {
            let w = const_eval_with_params(args.first().ok_or("bucket_ht_data_width needs 1 arg")?, param_vals)?;
            Ok(if w >= 4 { 4 } else { w })
        }
        // OpenTitan otbn_pkg::SecAddRandWidth(w) = 2 * ($clog2(w) * w + 1) —
        // lebar randomness untuk otbn_sec_add / otbn_mask_accelerator
        // (localparam `RandWidth = SecAddRandWidth(Width)`).
        Expr::FuncCall { name, args, .. } if base_func_name(name.as_str()) == "SecAddRandWidth" => {
            let w = const_eval_with_params(args.first().ok_or("SecAddRandWidth needs 1 arg")?, param_vals)?;
            let clog = if w <= 1 { 1 } else { 63 - (w as u64).leading_zeros() as i64 };
            Ok(2 * (clog * w + 1))
        }
        // OpenTitan entropy_src_pkg::num_bucket_ht_inst(w) =
        // ceil_div(w, bucket_ht_data_width(w)) — jumlah instance bucket
        // (dipakai generate for di entropy_src.sv).
        Expr::FuncCall { name, args, .. } if base_func_name(name.as_str()) == "num_bucket_ht_inst" => {
            let w = const_eval_with_params(args.first().ok_or("num_bucket_ht_inst needs 1 arg")?, param_vals)?;
            let b = if w >= 4 { 4 } else { w };
            if b == 0 {
                return Err("division by zero in num_bucket_ht_inst".to_string());
            }
            Ok(if w % b != 0 { w / b + 1 } else { w / b })
        }
        Expr::FuncCall { name, args, .. } if name == "$bits" || name == "$size" => {
            if let Some(arg) = args.first() {
                const_eval_with_params(arg, param_vals)
            } else {
                Ok(0)
            }
        }
        Expr::FuncCall { name, .. } if name.starts_with("$") => Err(format!(
            "cannot evaluate system function '{}' in constant context",
            name
        )),
        _ => Err(format!(
            "non-constant expression in parameter context: {:?}",
            expr
        )),
    }
}

/// Bangun key lookup untuk base sebuah member access: `name` untuk Ident,
/// `name[idx]` untuk BitSelect konstanta, `name[r][c]` untuk BitSelect 2D.
/// Dipakai `const_eval_with_params` pada `Expr::MemberAccess` untuk mencari
/// key ter-flatten `name[idx].field` di param_vals.
fn expr_base_key(expr: &Expr, param_vals: &std::collections::HashMap<Symbol, i64>) -> Option<String> {
    match expr {
        Expr::Ident { name, .. } => Some(name.as_str().to_string()),
        Expr::BitSelect { expr: inner, index } => {
            let base = expr_base_key(inner, param_vals)?;
            let idx = const_eval_with_params(index, param_vals).ok()?;
            Some(format!("{}[{}]", base, idx))
        }
        _ => None,
    }
}
