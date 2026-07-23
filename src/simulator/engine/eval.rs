use super::{
    edge_matches_abbrev, evaluate_string_method, SimulationEngine, sym_char_matches,
};
use crate::simulator::util::*;
use crate::ast::*;
use crate::error::SimError;
use crate::ir::*;
use crate::Symbol;
use crate::simulator::state::SimulationState;
use crate::simulator::types::*;
use crate::simulator::value::*;
use rand::Rng;
use rand::SeedableRng;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;

impl SimulationEngine {
    pub(crate) fn eval_assign_rhs(&mut self, expr: &IrExpr, lhs: &IrLValue) -> Result<LogicVec, SimError> {
        if let IrExpr::FillLit(v) = expr {
            let w = self.get_lvalue_width(lhs);
            Ok(LogicVec::fill(*v, w))
        } else if let IrExpr::Signed(inner) = expr {
            let mut val = self.evaluate_expr(inner)?;
            let target_w = self.get_lvalue_width(lhs);
            if val.width < target_w {
                let msb = val.bits.last().copied().unwrap_or(LogicVal::Zero);
                val.bits.resize(target_w, msb);
                val.width = target_w;
            }
            Ok(val)
        } else if let IrExpr::NewCall { class_name, args } = expr {
            if class_name.is_empty() && args.len() == 1 {
                let size_val = self.evaluate_expr(&args[0])?;
                let size = size_val.to_u64() as usize;
                if let Some(sig_id) = self.signal_id_from_lvalue(lhs) {
                    let elem_width = self.design.top.signals[sig_id].elem_width;
                    Ok(LogicVec::fill(LogicVal::X, size * elem_width))
                } else {
                    self.evaluate_expr(expr)
                }
            } else {
                self.evaluate_expr(expr)
            }
        } else {
            self.evaluate_expr(expr)
        }
    }

    pub(crate) fn evaluate_expr(&mut self, expr: &IrExpr) -> Result<LogicVec, SimError> {
        match expr {
            IrExpr::Const(val) => Ok(val.clone()),
            IrExpr::FillLit(val) => Ok(LogicVec::fill(*val, 1)),
            IrExpr::Signal(id, _) => {
                let mut val = self.state.read_signal(*id).clone();
                sanitize_for_2state(&self.design.top.signals, *id, &mut val);
                Ok(val)
            }
            IrExpr::RangeSelect(sig_id, msb, lsb) => {
                let val = self.state.read_signal(*sig_id);
                let (start, end) = if *msb > *lsb {
                    (*lsb, *msb)
                } else {
                    (*msb, *lsb)
                };
                let mut bits = val.bits[start..=end].to_vec();
                if *msb > *lsb {
                    bits.reverse();
                }
                Ok(LogicVec {
                    width: bits.len(),
                    bits,
                })
            }
            IrExpr::BitSelect(sig_id, idx) => {
                let val = self.state.read_signal(*sig_id);
                let bit = val.bits.get(*idx).copied().unwrap_or(LogicVal::X);
                Ok(LogicVec {
                    bits: vec![bit],
                    width: 1,
                })
            }
            IrExpr::ExprRangeSelect(inner, msb, lsb) => {
                let val = self.evaluate_expr(inner)?;
                let (start, end) = if *msb > *lsb {
                    (*lsb, *msb)
                } else {
                    (*msb, *lsb)
                };
                if end >= val.width {
                    return Err(SimError::runtime(format!(
                        "range select out of bounds: {}:{} on width {}",
                        msb, lsb, val.width
                    )));
                }
                let mut bits = val.bits[start..=end].to_vec();
                if *msb > *lsb {
                    bits.reverse();
                }
                Ok(LogicVec {
                    width: bits.len(),
                    bits,
                })
            }
            IrExpr::ExprBitSelect(inner, idx) => {
                let val = self.evaluate_expr(inner)?;
                let bit = val.bits.get(*idx).copied().unwrap_or(LogicVal::X);
                Ok(LogicVec {
                    bits: vec![bit],
                    width: 1,
                })
            }
            IrExpr::ExprPartSelect(inner, base_expr, width_expr) => {
                let val = self.evaluate_expr(inner)?;
                let base = self.evaluate_expr(base_expr)?;
                let width = self.evaluate_expr(width_expr)?;
                let base = base.to_u64() as usize;
                let width = width.to_u64() as usize;
                if width == 0 || base >= val.width {
                    return Ok(LogicVec::new(1));
                }
                let end = (base + width - 1).min(val.width - 1);
                let mut bits = val.bits[base..=end].to_vec();
                // PartSelect is always [high:low] with high >= low, so reverse
                bits.reverse();
                Ok(LogicVec {
                    width: bits.len(),
                    bits,
                })
            }
            IrExpr::ArrayIndex {
                sig_id,
                index,
                elem_width,
            } => {
                let key_val = self.evaluate_expr(index)?;
                // Check if this is an associative array
                let sig_info = self.design.top.signals.get(*sig_id);
                if sig_info.map(|s| s.is_associative).unwrap_or(false) {
                    let assoc_map = self.assoc_data.entry(*sig_id).or_insert_with(HashMap::new);
                    if let Some(val) = assoc_map.get(&key_val) {
                        return Ok(val.clone());
                    }
                    return Ok(LogicVec::new(*elem_width));
                }
                let array_val = self.state.read_signal(*sig_id).clone();
                let idx = key_val.to_u64() as usize;
                let start = idx * elem_width;
                let end = start + elem_width - 1;
                let mut bits = Vec::with_capacity(*elem_width);
                for i in start..=end {
                    bits.push(array_val.bits.get(i).copied().unwrap_or(LogicVal::X));
                }
                Ok(LogicVec {
                    width: *elem_width,
                    bits,
                })
            }
            IrExpr::Concat(exprs) => {
                let mut result = LogicVec::new(0);
                for e in exprs.iter().rev() {
                    let part = self.evaluate_expr(e)?;
                    result = result.extend(&part);
                }
                Ok(result)
            }
            IrExpr::Replicate(count, inner) => {
                let val = self.evaluate_expr(inner)?;
                let mut result = LogicVec::new(0);
                for _ in 0..*count {
                    result = result.extend(&val);
                }
                Ok(result)
            }
            IrExpr::UnaryOp(op, inner) => {
                let val = self.evaluate_expr(inner)?;
                let inner_is_real = matches!(inner.as_ref(), IrExpr::Signal(id, _) if self.design.top.signals.get(*id).map(|s| s.is_real).unwrap_or(false));
                if inner_is_real {
                    let a = f64::from_bits(val.to_u64());
                    let result = match op {
                        UnaryIrOp::Minus => -a,
                        UnaryIrOp::Plus => a,
                        _ => return Ok(eval_unary(op.clone(), &val)),
                    };
                    return Ok(LogicVec::from_u64(result.to_bits(), 64));
                }
                Ok(eval_unary(op.clone(), &val))
            }
            IrExpr::BinaryOp(op, lhs, rhs) => {
                let lval = self.evaluate_expr(lhs)?;
                let rval = self.evaluate_expr(rhs)?;
                let lhs_is_real = matches!(lhs.as_ref(), IrExpr::Signal(id, _) if self.design.top.signals.get(*id).map(|s| s.is_real).unwrap_or(false));
                let rhs_is_real = matches!(rhs.as_ref(), IrExpr::Signal(id, _) if self.design.top.signals.get(*id).map(|s| s.is_real).unwrap_or(false));
                if lhs_is_real || rhs_is_real {
                    let a = f64::from_bits(lval.to_u64());
                    let b = f64::from_bits(rval.to_u64());
                    let result = match op {
                        BinaryIrOp::Add => a + b,
                        BinaryIrOp::Sub => a - b,
                        BinaryIrOp::Mul => a * b,
                        BinaryIrOp::Div => a / b,
                        BinaryIrOp::Mod => a % b,
                        BinaryIrOp::Power => a.powf(b),
                        BinaryIrOp::Lt => {
                            return Ok(LogicVec::from_u64(if a < b { 1 } else { 0 }, 32))
                        }
                        BinaryIrOp::Le => {
                            return Ok(LogicVec::from_u64(if a <= b { 1 } else { 0 }, 32))
                        }
                        BinaryIrOp::Gt => {
                            return Ok(LogicVec::from_u64(if a > b { 1 } else { 0 }, 32))
                        }
                        BinaryIrOp::Ge => {
                            return Ok(LogicVec::from_u64(if a >= b { 1 } else { 0 }, 32))
                        }
                        BinaryIrOp::Eq => {
                            return Ok(LogicVec::from_u64(if a == b { 1 } else { 0 }, 32))
                        }
                        BinaryIrOp::Neq => {
                            return Ok(LogicVec::from_u64(if a != b { 1 } else { 0 }, 32))
                        }
                        _ => return Ok(eval_binary(op.clone(), &lval, &rval)),
                    };
                    Ok(LogicVec::from_u64(result.to_bits(), 64))
                } else if matches!(
                    op,
                    BinaryIrOp::Lt | BinaryIrOp::Le | BinaryIrOp::Gt | BinaryIrOp::Ge
                ) && (is_signed_expr(lhs.as_ref(), &self.design.top.signals)
                    || is_signed_expr(rhs.as_ref(), &self.design.top.signals))
                {
                    Ok(eval_binary_signed(op.clone(), &lval, &rval))
                } else {
                    Ok(eval_binary(op.clone(), &lval, &rval))
                }
            }
            IrExpr::Cond(cond, true_expr, false_expr) => {
                let cval = self.evaluate_expr(cond)?;
                if cval.to_bool().unwrap_or(false) {
                    self.evaluate_expr(true_expr)
                } else {
                    self.evaluate_expr(false_expr)
                }
            }
            IrExpr::Signed(inner) => self.evaluate_expr(inner),
            IrExpr::String(s) => {
                let mut bits = Vec::with_capacity(s.len() * 8);
                for c in s.chars() {
                    let byte = c as u8;
                    for i in 0..8 {
                        bits.push(if (byte >> i) & 1 == 1 {
                            LogicVal::One
                        } else {
                            LogicVal::Zero
                        });
                    }
                }
                Ok(LogicVec {
                    width: bits.len(),
                    bits,
                })
            }
            IrExpr::SysFunc { name, args } => {
                match name.as_str() {
                    "$random" => {
                        // If seed argument provided, reseed RNG for reproducibility
                        if let Some(seed_arg) = args.first() {
                            if let Ok(seed_val) = self.evaluate_expr(seed_arg) {
                                let seed = seed_val.to_u64();
                                self.rng = rand::rngs::StdRng::seed_from_u64(seed);
                            }
                        }
                        let val: i32 = self.rng.gen();
                        Ok(LogicVec::from_u64(val as u64, 32))
                    }
                    "$urandom" => {
                        let val: u32 = self.rng.gen();
                        Ok(LogicVec::from_u64(val as u64, 32))
                    }
                    "$urandom_range" => {
                        let args_eval: Vec<LogicVec> = args
                            .iter()
                            .map(|a| self.evaluate_expr(a))
                            .collect::<Result<_, _>>()?;
                        let maxval = args_eval.first().map(|v| v.to_u64()).unwrap_or(0);
                        let minval = args_eval.get(1).map(|v| v.to_u64()).unwrap_or(0);
                        if maxval <= minval {
                            Ok(LogicVec::from_u64(minval, 32))
                        } else {
                            let range = maxval - minval + 1;
                            let val: u64 = if range <= 1 {
                                minval
                            } else {
                                minval + (self.rng.gen::<u64>() % range)
                            };
                            Ok(LogicVec::from_u64(val, 32))
                        }
                    }
                    "$signed" => {
                        if let Some(arg) = args.first() {
                            let val = self.evaluate_expr(arg)?;
                            // Sign-extend: copy the MSB to all higher bits
                            if val.width > 0 {
                                let msb = val.bits.last().copied().unwrap_or(LogicVal::Zero);
                                let new_width = val.width.max(1);
                                let mut bits = val.bits.clone();
                                bits.resize(new_width, msb);
                                Ok(LogicVec {
                                    width: new_width,
                                    bits,
                                })
                            } else {
                                Ok(val)
                            }
                        } else {
                            Err(SimError::runtime("$signed expects 1 argument"))
                        }
                    }
                    "$unsigned" => {
                        if let Some(arg) = args.first() {
                            let val = self.evaluate_expr(arg)?;
                            // Unsigned: zero-extend (already the default)
                            Ok(val)
                        } else {
                            Err(SimError::runtime("$unsigned expects 1 argument"))
                        }
                    }
                    "$countones" => {
                        if let Some(arg) = args.first() {
                            let val = self.evaluate_expr(arg)?;
                            let count =
                                val.bits.iter().filter(|b| **b == LogicVal::One).count() as u64;
                            Ok(LogicVec::from_u64(count, 32))
                        } else {
                            Err(SimError::runtime("$countones expects 1 argument"))
                        }
                    }
                    "$onehot" => {
                        if let Some(arg) = args.first() {
                            let val = self.evaluate_expr(arg)?;
                            let ones = val.bits.iter().filter(|b| **b == LogicVal::One).count();
                            let is_onehot = ones == 1;
                            Ok(LogicVec::from_u64(if is_onehot { 1 } else { 0 }, 1))
                        } else {
                            Err(SimError::runtime("$onehot expects 1 argument"))
                        }
                    }
                    "$isunknown" => {
                        if let Some(arg) = args.first() {
                            let val = self.evaluate_expr(arg)?;
                            let has_x_or_z = val
                                .bits
                                .iter()
                                .any(|b| *b == LogicVal::X || *b == LogicVal::Z);
                            Ok(LogicVec::from_u64(if has_x_or_z { 1 } else { 0 }, 1))
                        } else {
                            Err(SimError::runtime("$isunknown expects 1 argument"))
                        }
                    }
                    "$fopen" => {
                        let fname = args.first().and_then(|a| {
                            if let IrExpr::String(s) = a {
                                Some(s.clone())
                            } else {
                                None
                            }
                        });
                        if let Some(fname) = fname {
                            let mode = args.get(1).and_then(|a| {
                                if let IrExpr::String(s) = a {
                                    Some(s.as_str())
                                } else {
                                    None
                                }
                            });
                            let open_result = match mode {
                                Some("r") | Some("rb") => std::fs::File::open(&fname),
                                _ => std::fs::OpenOptions::new()
                                    .read(true)
                                    .write(true)
                                    .create(true)
                                    .truncate(true)
                                    .open(&fname),
                            };
                            match open_result {
                                Ok(f) => {
                                    let handle = self.next_file_handle;
                                    self.next_file_handle += 1;
                                    self.file_handles.insert(handle, f);
                                    self.file_read_pos.insert(handle, 0);
                                    Ok(LogicVec::from_u64(handle as u64, 32))
                                }
                                Err(_) => Ok(LogicVec::from_u64(0, 32)),
                            }
                        } else {
                            Ok(LogicVec::from_u64(0, 32))
                        }
                    }
                    "$fdisplay" => {
                        let handle = args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let msg = format_display(
                                    &self.state,
                                    &self.design.top.signals,
                                    &self.design.hier_signal_map,
                                    &self.assoc_data,
                                    &args[1..],
                                );
                                let _ = write!(f, "{}", msg);
                            }
                        }
                        Ok(LogicVec::from_u64(0, 1))
                    }
                    "$fread" => {
                        let target = args.first().and_then(|a| {
                            if let IrExpr::Signal(id, _) = a {
                                Some(*id)
                            } else {
                                None
                            }
                        });
                        let src = args.get(1);
                        let data = if let Some(IrExpr::String(fname)) = src {
                            std::fs::read(fname).ok()
                        } else if let Some(arg) = src {
                            let handle = self
                                .evaluate_expr(arg)
                                .ok()
                                .map(|v| v.to_u64() as u32)
                                .unwrap_or(0);
                            if handle > 0 {
                                use std::io::Read;
                                self.file_handles.get_mut(&handle).and_then(|f| {
                                    let mut buf = Vec::new();
                                    f.read_to_end(&mut buf).ok().map(|_| buf)
                                })
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        if let (Some(sid), Some(bytes)) = (target, data) {
                            let mut bits = Vec::with_capacity(bytes.len() * 8);
                            for byte in bytes {
                                for i in 0..8 {
                                    bits.push(if (byte >> i) & 1 == 1 {
                                        LogicVal::One
                                    } else {
                                        LogicVal::Zero
                                    });
                                }
                            }
                            self.state.write_signal(
                                sid,
                                LogicVec {
                                    width: bits.len(),
                                    bits,
                                },
                            );
                        }
                        Ok(LogicVec::from_u64(0, 1))
                    }
                    "$fclose" => {
                        let handle = args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            self.file_handles.remove(&h);
                            self.file_read_pos.remove(&h);
                        }
                        Ok(LogicVec::from_u64(0, 1))
                    }
                    "$fflush" => {
                        let handle = args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                use std::io::Write;
                                let _ = f.flush();
                            }
                        }
                        Ok(LogicVec::from_u64(0, 1))
                    }
                    "$fseek" => {
                        let handle = args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        let offset = args
                            .get(1)
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as i64));
                        let op = args
                            .get(2)
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64()));
                        if let (Some(h), Some(off)) = (handle, offset) {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                use std::io::{Seek, SeekFrom};
                                let seek_from = match op {
                                    Some(1) => SeekFrom::Current(off),
                                    Some(2) => SeekFrom::End(off),
                                    _ => SeekFrom::Start(off as u64),
                                };
                                let _ = f.seek(seek_from);
                                if let Some(pos) = f.stream_position().ok() {
                                    self.file_read_pos.insert(h, pos);
                                }
                            }
                        }
                        Ok(LogicVec::from_u64(0, 1))
                    }
                    "$ftell" => {
                        let handle = args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                use std::io::Seek;
                                let pos = f.stream_position().unwrap_or(0);
                                return Ok(LogicVec::from_u64(pos, 32));
                            }
                        }
                        Ok(LogicVec::from_u64(0, 32))
                    }
                    "$feof" => {
                        let handle = args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                use std::io::{Read, Seek};
                                let pos = f.stream_position().unwrap_or(0);
                                let mut byte = [0u8; 1];
                                let n = f.read(&mut byte).unwrap_or(0);
                                f.seek(std::io::SeekFrom::Start(pos)).ok();
                                return Ok(LogicVec::from_u64(if n == 0 { 1 } else { 0 }, 1));
                            }
                        }
                        Ok(LogicVec::from_u64(1, 1))
                    }
                    "$rewind" => {
                        // $rewind(fd) — rewind file to beginning (same as $fseek(fd, 0, 0))
                        let handle = args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                use std::io::{Seek, SeekFrom};
                                let _ = f.seek(SeekFrom::Start(0));
                                self.file_read_pos.insert(h, 0);
                                self.file_ungetc_buf.remove(&h);
                            }
                        }
                        Ok(LogicVec::from_u64(0, 1))
                    }
                    "$fgets" => {
                        // $fgets(str_var, fd) — read a line from file handle into string var
                        let str_arg = args.first();
                        let handle = args
                            .get(1)
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                use std::io::{BufRead, BufReader};
                                let mut reader = BufReader::new(f.by_ref());
                                let mut line = String::new();
                                let bytes = reader.read_line(&mut line).unwrap_or(0);
                                if bytes > 0 {
                                    // Trim trailing newline for Verilog string compatibility
                                    if line.ends_with('\n') {
                                        line.pop();
                                    }
                                    if line.ends_with('\r') {
                                        line.pop();
                                    }
                                    // Convert string to LogicVec
                                    let mut bits = Vec::with_capacity(line.len() * 8);
                                    for c in line.chars() {
                                        let byte = c as u8;
                                        for i in 0..8 {
                                            bits.push(if (byte >> i) & 1 == 1 {
                                                LogicVal::One
                                            } else {
                                                LogicVal::Zero
                                            });
                                        }
                                    }
                                    // Write into the string variable
                                    if let Some(IrExpr::Signal(sid, _)) = str_arg {
                                        self.state.write_signal(
                                            *sid,
                                            LogicVec {
                                                width: bits.len(),
                                                bits,
                                            },
                                        );
                                    }
                                    return Ok(LogicVec::from_u64(bytes as u64, 32));
                                }
                            }
                        }
                        Ok(LogicVec::from_u64(0, 32))
                    }
                    "$fgetc" => {
                        // $fgetc(fd) — read a single character from file handle
                        let handle = args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            // Check ungetc buffer first
                            if let Some(buf) = self.file_ungetc_buf.get_mut(&h) {
                                if let Some(byte) = buf.pop() {
                                    return Ok(LogicVec::from_u64(byte as u64, 32));
                                }
                            }
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                use std::io::Read;
                                let mut byte = [0u8; 1];
                                let bytes = f.read(&mut byte).unwrap_or(0);
                                if bytes > 0 {
                                    return Ok(LogicVec::from_u64(byte[0] as u64, 32));
                                }
                            }
                        }
                        Ok(LogicVec::from_u64(!0u64, 32)) // EOF: returns 32'hFFFFFFFF
                    }
                    "$ungetc" => {
                        // $ungetc(char, fd) — push back a character to file handle
                        let char_val = args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u8));
                        let handle = args
                            .get(1)
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let (Some(c), Some(h)) = (char_val, handle) {
                            self.file_ungetc_buf.entry(h).or_default().push(c);
                        }
                        Ok(LogicVec::from_u64(0, 1))
                    }
                    "$fscanf" => {
                        let handle = args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                use std::io::{Read, Seek};
                                let read_pos = self.file_read_pos.entry(h).or_insert(0);
                                f.seek(std::io::SeekFrom::Start(*read_pos)).ok();
                                let mut content = String::new();
                                let _bytes_read = f.read_to_string(&mut content).unwrap_or(0);
                                *read_pos = f.stream_position().unwrap_or(0);
                                let fmt = args.get(1).and_then(|a| {
                                    if let IrExpr::String(s) = a {
                                        Some(s.clone())
                                    } else {
                                        None
                                    }
                                });
                                if let Some(ref fmt_str) = fmt {
                                    let tokens: Vec<&str> = content.split_whitespace().collect();
                                    let mut ti = 0;
                                    let mut ai = 0;
                                    let mut chars = fmt_str.chars().peekable();
                                    while let Some(c) = chars.next() {
                                        if c == '%' {
                                            if let Some(spec) = chars.next() {
                                                if spec == 'd' || spec == 'h' || spec == 'b' {
                                                    if let Some(tok) = tokens.get(ti) {
                                                        if let Ok(val) = if spec == 'h' {
                                                            i64::from_str_radix(tok, 16)
                                                        } else if spec == 'b' {
                                                            i64::from_str_radix(tok, 2)
                                                        } else {
                                                            tok.parse::<i64>()
                                                        } {
                                                            let out_idx = 2 + ai;
                                                            if let Some(arg) = args.get(out_idx) {
                                                                if let IrExpr::Signal(sid, _) = arg
                                                                {
                                                                    self.state.write_signal(
                                                                        *sid,
                                                                        LogicVec::from_u64(
                                                                            val as u64, 32,
                                                                        ),
                                                                    );
                                                                }
                                                            }
                                                            ai += 1;
                                                        }
                                                    }
                                                    ti += 1;
                                                }
                                            }
                                        }
                                    }
                                    // $fscanf returns number of items matched (or EOF)
                                    return Ok(LogicVec::from_u64(ai as u64, 32));
                                }
                            }
                        }
                        Ok(LogicVec::from_u64(0, 32))
                    }
                    "$sformatf" => {
                        if args.is_empty() {
                            return Ok(LogicVec::new(0));
                        }
                        let msg = format_display(
                            &self.state,
                            &self.design.top.signals,
                            &self.design.hier_signal_map,
                            &self.assoc_data,
                            args,
                        );
                        let mut bits = Vec::with_capacity(msg.len() * 8);
                        for c in msg.chars() {
                            let byte = c as u8;
                            for i in 0..8 {
                                bits.push(if (byte >> i) & 1 == 1 {
                                    LogicVal::One
                                } else {
                                    LogicVal::Zero
                                });
                            }
                        }
                        Ok(LogicVec {
                            width: bits.len(),
                            bits,
                        })
                    }
                    "$clog2" => {
                        if let Some(arg) = args.first() {
                            let val = self.evaluate_expr(arg)?;
                            let n = val.to_u64();
                            if n <= 1 {
                                Ok(LogicVec::from_u64(0, 32))
                            } else {
                                let bits = (64 - n.leading_zeros()) as u64;
                                if n.is_power_of_two() {
                                    Ok(LogicVec::from_u64(bits - 1, 32))
                                } else {
                                    Ok(LogicVec::from_u64(bits, 32))
                                }
                            }
                        } else {
                            Ok(LogicVec::from_u64(0, 32))
                        }
                    }
                    "$time" => Ok(LogicVec::from_u64(self.state.time as u64, 64)),
                    "$realtime" => {
                        let t = self.state.time as f64;
                        Ok(LogicVec::from_u64(t.to_bits(), 64))
                    }
                    "process::self" => {
                        let pid = self.current_process_id.unwrap_or(0);
                        if pid == 0 {
                            let pid = self.state.alloc_object("__process".into());
                            self.process_map.insert(
                                pid,
                                ProcessInfo {
                                    status: ProcessStatus::Running,
                                    await_continuations: Vec::new(),
                                },
                            );
                            self.current_process_id = Some(pid);
                        }
                        Ok(LogicVec::from_u64(
                            self.current_process_id.unwrap_or(0) as u64,
                            64,
                        ))
                    }
                    "uvm_config_db::set" => {
                        let arg_vals: Vec<LogicVec> = args
                            .iter()
                            .map(|a| self.evaluate_expr(a))
                            .collect::<Result<_, _>>()?;
                        let inst_name = if arg_vals.len() > 1 {
                            logicvec_to_string(&arg_vals[1])
                        } else {
                            String::new()
                        };
                        let field_name = if arg_vals.len() > 2 {
                            logicvec_to_string(&arg_vals[2])
                        } else {
                            String::new()
                        };
                        let value = if arg_vals.len() > 3 {
                            arg_vals[3].clone()
                        } else {
                            LogicVec::new(1)
                        };
                        self.uvm_config_db_data
                            .insert((Symbol::intern(&inst_name), Symbol::intern(&field_name)), value);
                        Ok(LogicVec::from_u64(1, 1))
                    }
                    "uvm_config_db::get" => {
                        let arg_vals: Vec<LogicVec> = args
                            .iter()
                            .map(|a| self.evaluate_expr(a))
                            .collect::<Result<_, _>>()?;
                        let inst_name = if arg_vals.len() > 1 {
                            logicvec_to_string(&arg_vals[1])
                        } else {
                            String::new()
                        };
                        let field_name = if arg_vals.len() > 2 {
                            logicvec_to_string(&arg_vals[2])
                        } else {
                            String::new()
                        };
                        let key = (inst_name, field_name);
                        let stored = self.uvm_config_db_data.get::<(Symbol, Symbol)>(&(Symbol::intern(&key.0), Symbol::intern(&key.1))).cloned();
                        if let Some(val) = stored {
                            if let Some(last_arg) = args.get(3) {
                                if let IrExpr::Signal(sig_id, _) = last_arg {
                                    self.state.write_signal(*sig_id, val);
                                }
                            }
                            Ok(LogicVec::from_u64(1, 1))
                        } else {
                            Ok(LogicVec::from_u64(0, 1))
                        }
                    }
                    "uvm_resource_db::set" => {
                        let arg_vals: Vec<LogicVec> = args
                            .iter()
                            .map(|a| self.evaluate_expr(a))
                            .collect::<Result<_, _>>()?;
                        let scope = if !arg_vals.is_empty() {
                            logicvec_to_string(&arg_vals[0])
                        } else {
                            String::new()
                        };
                        let name = if arg_vals.len() > 1 {
                            logicvec_to_string(&arg_vals[1])
                        } else {
                            String::new()
                        };
                        let value = if arg_vals.len() > 2 {
                            arg_vals[2].clone()
                        } else {
                            LogicVec::new(1)
                        };
                        self.uvm_resource_db_data.insert((Symbol::intern(&scope), Symbol::intern(&name)), value);
                        Ok(LogicVec::from_u64(1, 1))
                    }
                    "uvm_resource_db::get" => {
                        let arg_vals: Vec<LogicVec> = args
                            .iter()
                            .map(|a| self.evaluate_expr(a))
                            .collect::<Result<_, _>>()?;
                        let scope = if !arg_vals.is_empty() {
                            logicvec_to_string(&arg_vals[0])
                        } else {
                            String::new()
                        };
                        let rname = if arg_vals.len() > 1 {
                            logicvec_to_string(&arg_vals[1])
                        } else {
                            String::new()
                        };
                        let key = (scope, rname);
                        let stored = self.uvm_resource_db_data.get::<(Symbol, Symbol)>(&(Symbol::intern(&key.0), Symbol::intern(&key.1))).cloned();
                        if let Some(val) = stored {
                            if let Some(last_arg) = args.get(2) {
                                if let IrExpr::Signal(sig_id, _) = last_arg {
                                    self.state.write_signal(*sig_id, val);
                                }
                            }
                            Ok(LogicVec::from_u64(1, 1))
                        } else {
                            Ok(LogicVec::from_u64(0, 1))
                        }
                    }
                    "uvm_factory::set_type_override_by_type" => {
                        let arg_vals: Vec<LogicVec> = args
                            .iter()
                            .map(|a| self.evaluate_expr(a))
                            .collect::<Result<_, _>>()?;
                        let orig = if !arg_vals.is_empty() {
                            logicvec_to_string(&arg_vals[0])
                        } else {
                            String::new()
                        };
                        let override_type = if arg_vals.len() > 1 {
                            logicvec_to_string(&arg_vals[1])
                        } else {
                            String::new()
                        };
                        self.factory_type_overrides.insert(Symbol::intern(&orig), Symbol::intern(&override_type));
                        Ok(LogicVec::from_u64(1, 1))
                    }
                    "$test$plusargs" => {
                        if let Some(pattern) = args.first() {
                            if let Ok(pat_val) = self.evaluate_expr(pattern) {
                                let pat_str = logicvec_to_string(&pat_val);
                                for key in self.plusargs.keys() {
                                    if key.starts_with(&pat_str) {
                                        return Ok(LogicVec::from_u64(1, 32));
                                    }
                                }
                            }
                        }
                        Ok(LogicVec::from_u64(0, 32))
                    }
                    "$value$plusargs" => {
                        if let Some(pattern) = args.first() {
                            if let Ok(pat_val) = self.evaluate_expr(pattern) {
                                let pat_str = logicvec_to_string(&pat_val);
                                let plusarg_name = pat_str
                                    .split('%')
                                    .next()
                                    .unwrap_or(&pat_str)
                                    .trim_end_matches('=');
                                let plusargs = self.plusargs.clone();
                                for (key, val) in &plusargs {
                                    if key == plusarg_name {
                                        if let Some(var_arg) = args.get(1) {
                                            let num = if let Some(hex) = val
                                                .strip_prefix("0x")
                                                .or_else(|| val.strip_prefix("0X"))
                                            {
                                                u64::from_str_radix(hex, 16).unwrap_or(0)
                                            } else {
                                                val.parse::<u64>().unwrap_or(0)
                                            };
                                            let bits = LogicVec::from_u64(num, 32);
                                            if let IrExpr::Signal(id, _) = var_arg {
                                                self.state.write_signal(*id, bits);
                                            }
                                        }
                                        return Ok(LogicVec::from_u64(1, 32));
                                    }
                                }
                            }
                        }
                        Ok(LogicVec::from_u64(0, 32))
                    }
                    "$rose" => {
                        if let Some(arg) = args.first() {
                            let val = self.evaluate_expr(arg)?;
                            let key = format!("$rose({:?})", arg);
                            let prev = self
                                .sysfunc_prev
                                .entry(key)
                                .or_insert_with(|| LogicVec::fill(LogicVal::Zero, val.width));
                            let rose = prev.to_bool().unwrap_or(false) == false
                                && val.to_bool().unwrap_or(false) == true;
                            *prev = val;
                            Ok(LogicVec::from_u64(if rose { 1 } else { 0 }, 1))
                        } else {
                            Ok(LogicVec::from_u64(0, 1))
                        }
                    }
                    "$fell" => {
                        if let Some(arg) = args.first() {
                            let val = self.evaluate_expr(arg)?;
                            let key = format!("$fell({:?})", arg);
                            let prev = self
                                .sysfunc_prev
                                .entry(key)
                                .or_insert_with(|| LogicVec::fill(LogicVal::Zero, val.width));
                            let fell = prev.to_bool().unwrap_or(false) == true
                                && val.to_bool().unwrap_or(false) == false;
                            *prev = val;
                            Ok(LogicVec::from_u64(if fell { 1 } else { 0 }, 1))
                        } else {
                            Ok(LogicVec::from_u64(0, 1))
                        }
                    }
                    "$stable" => {
                        if let Some(arg) = args.first() {
                            let val = self.evaluate_expr(arg)?;
                            let key = format!("$stable({:?})", arg);
                            let prev = self
                                .sysfunc_prev
                                .entry(key)
                                .or_insert_with(|| LogicVec::fill(LogicVal::Zero, val.width));
                            let stable = *prev == val;
                            *prev = val;
                            Ok(LogicVec::from_u64(if stable { 1 } else { 0 }, 1))
                        } else {
                            Ok(LogicVec::from_u64(0, 1))
                        }
                    }
                    "$changed" => {
                        if let Some(arg) = args.first() {
                            let val = self.evaluate_expr(arg)?;
                            let key = format!("$changed({:?})", arg);
                            let prev = self
                                .sysfunc_prev
                                .entry(key)
                                .or_insert_with(|| LogicVec::fill(LogicVal::Zero, val.width));
                            let changed = *prev != val;
                            *prev = val;
                            Ok(LogicVec::from_u64(if changed { 1 } else { 0 }, 1))
                        } else {
                            Ok(LogicVec::from_u64(0, 1))
                        }
                    }
                    "$past" => {
                        if let Some(arg) = args.first() {
                            let val = self.evaluate_expr(arg)?;
                            let n = if args.len() > 1 {
                                if let Ok(nv) = self.evaluate_expr(&args[1]) {
                                    nv.to_u64().max(1) as usize
                                } else {
                                    1
                                }
                            } else {
                                1
                            };
                            let key = format!("$past({:?})", arg);
                            let hist = self
                                .sysfunc_history
                                .entry(key)
                                .or_insert_with(Vec::new);
                            hist.push(val);
                            if hist.len() > n {
                                let past = hist[hist.len() - 1 - n].clone();
                                Ok(past)
                            } else {
                                Ok(LogicVec::fill(LogicVal::Zero, hist[0].width))
                            }
                        } else {
                            Ok(LogicVec::from_u64(0, 32))
                        }
                    }
                    _ => {
                        eprintln!("warning: unsupported system function '{}'", name);
                        Ok(LogicVec::from_u64(0, 32))
                    }
                }
            }
            IrExpr::NewCall { class_name, args } => {
                let arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_expr(a))
                    .collect::<Result<_, _>>()?;
                // Check if this is a covergroup instantiation
                let is_cg = self
                    .design
                    .covergroups
                    .iter()
                    .any(|c| c.name == *class_name);
                let effective_name = if is_cg {
                    format!("__covergroup_{}", class_name)
                } else if let Some(override_type) = self.factory_type_overrides.get::<str>(class_name.as_str()) {
                    override_type.to_string()
                } else {
                    class_name.to_string()
                };
                let obj_id = self.state.alloc_object(Symbol::intern(&effective_name));
                if class_name == "__mailbox" {
                    self.mailbox_queues.insert(obj_id, VecDeque::new());
                } else if class_name == "__semaphore" {
                    let init = if !arg_vals.is_empty() {
                        arg_vals[0].to_u64() as u32
                    } else {
                        0
                    };
                    self.semaphore_counts.insert(obj_id, init);
                } else if is_cg {
                    // Auto-sample covergroup immediately on new()
                    self.sample_covergroup(class_name.as_str())?;
                } else if !class_name.is_empty() {
                    if let Some(cls) = self.design.classes.get(class_name.as_str()) {
                        if let Some(obj) = self.state.get_object_mut(obj_id) {
                            for field in &cls.fields {
                                obj.fields
                                    .entry(field.name.clone())
                                    .or_insert_with(|| LogicVec::from_u64(0, field.width));
                            }
                        }
                    }
                    if self.is_uvm_object_hierarchy(class_name.as_str()) {
                        self.uvm_object_data
                            .entry(obj_id)
                            .or_insert_with(|| UvmObjectData {
                                name: String::new(),
                            });
                    }
                    if self.is_uvm_analysis_port_hierarchy(class_name.as_str()) {
                        let pname = if !arg_vals.is_empty() {
                            logicvec_to_string(&arg_vals[0])
                        } else {
                            String::new()
                        };
                        self.uvm_analysis_port_data
                            .entry(obj_id)
                            .or_insert_with(|| UvmAnalysisPortData {
                                connections: Vec::new(),
                                name: pname.clone(),
                            });
                        self.uvm_object_data
                            .entry(obj_id)
                            .or_insert_with(|| UvmObjectData { name: pname });
                    }
                    if self.is_uvm_analysis_imp_hierarchy(class_name.as_str()) {
                        let pname = if !arg_vals.is_empty() {
                            logicvec_to_string(&arg_vals[0])
                        } else {
                            String::new()
                        };
                        let parent_obj = arg_vals.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                        self.uvm_analysis_imp_data.entry(obj_id).or_insert_with(|| {
                            UvmAnalysisImpData {
                                parent: if parent_obj != 0 {
                                    Some(parent_obj)
                                } else {
                                    None
                                },
                                name: pname.clone(),
                            }
                        });
                        self.uvm_object_data
                            .entry(obj_id)
                            .or_insert_with(|| UvmObjectData { name: pname });
                    }
                    if self.is_uvm_component_hierarchy(class_name.as_str()) {
                        let name = logicvec_to_string(&arg_vals[0]);
                        let parent_obj = arg_vals.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                        self.uvm_object_data
                            .insert(obj_id, UvmObjectData { name: name.clone() });
                        let mut cd = UvmComponentData {
                            parent: None,
                            children: Vec::new(),
                            report_verbosity: 2,
                        };
                        if parent_obj != 0 {
                            cd.parent = Some(parent_obj);
                            if let Some(pd) = self.uvm_component_data.get_mut(&parent_obj) {
                                pd.children.push(obj_id);
                            }
                        }
                        self.uvm_component_data.insert(obj_id, cd);
                    }
                    if self.find_method_in_hierarchy(class_name.as_str(), "new").is_ok() {
                        self.execute_method(obj_id, "new", &arg_vals)?;
                    }
                }
                Ok(LogicVec::from_u64(obj_id as u64, 64))
            }
            IrExpr::This => {
                if let Some(obj_id) = self.current_this {
                    Ok(LogicVec::from_u64(obj_id as u64, 64))
                } else {
                    Err(SimError::runtime("'this' used outside of class method"))
                }
            }
            IrExpr::MethodCall {
                obj,
                method,
                args,
                with_clause,
            } => {
                if let IrExpr::String(s) = obj.as_ref() {
                    let arg_vals: Vec<LogicVec> = args
                        .iter()
                        .map(|a| self.evaluate_expr(a))
                        .collect::<Result<_, _>>()?;
                    let result = evaluate_string_method(s.as_str(), method.as_str(), &arg_vals)?;
                    return Ok(result);
                }
                if let IrExpr::Signal(id, _) = obj.as_ref() {
                    if let Some(sig) = self.design.top.signals.get(*id) {
                        if sig.is_string {
                            let lv = self.state.read_signal(*id);
                            let s = logicvec_to_string(lv);
                            let arg_vals: Vec<LogicVec> = args
                                .iter()
                                .map(|a| self.evaluate_expr(a))
                                .collect::<Result<_, _>>()?;
                            let result = evaluate_string_method(&s, method.as_str(), &arg_vals)?;
                            return Ok(result);
                        }
                    }
                    if let Some(sig) = self.design.top.signals.get(*id) {
                        if let Some(ref cn) = sig.class_name {
                            let is_arr = sig.is_dynamic || sig.is_queue;
                            if !is_arr && !sig.is_string {
                                // Check if this class_name matches a covergroup or class
                                let is_cg = self.design.covergroups.iter().any(|c| c.name == *cn);
                                if is_cg || self.design.classes.contains_key(cn) {
                                    let obj_val = self.state.read_signal(*id);
                                    let obj_id = obj_val.to_u64() as ObjId;
                                    if obj_id == 0
                                        && self.state.objects.len() > 0
                                        && self.state.objects[0].class_name.is_empty()
                                    {
                                        let class_for_obj = if is_cg {
                                            format!("__covergroup_{}", cn)
                                        } else {
                                            cn.to_string()
                                        };
                                        let new_id = self.state.alloc_object(Symbol::intern(&class_for_obj));
                                        self.state.write_signal(
                                            *id,
                                            LogicVec::from_u64(new_id as u64, 64),
                                        );
                                        let arg_vals: Vec<LogicVec> = args
                                            .iter()
                                            .map(|a| self.evaluate_expr(a))
                                            .collect::<Result<_, _>>()?;
                                        return self.execute_method(new_id, method.as_str(), &arg_vals);
                                    }
                                }
                            }
                        }
                    }
                    let is_arr = self
                        .design
                        .top
                        .signals
                        .get(*id)
                        .map(|s| s.is_dynamic || s.is_queue)
                        .unwrap_or(false);
                    if is_arr {
                        let sig_info = self.design.top.signals[*id].clone();
                        return self.evaluate_array_method(
                            *id,
                            &sig_info,
                            method,
                            args,
                            with_clause.as_deref(),
                        );
                    }
                }
                let obj_val = self.evaluate_expr(obj)?;
                let obj_id = obj_val.to_u64() as ObjId;
                let arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_expr(a))
                    .collect::<Result<_, _>>()?;
                // Handle randomize() with inline constraint
                if method == "randomize" && with_clause.is_some() {
                    let class_name = self
                        .state
                        .get_object(obj_id)
                        .map(|o| o.class_name.clone())
                        .unwrap_or_default();
                    return self.execute_randomize_with(
                        obj_id,
                        &class_name,
                        with_clause.as_deref(),
                    );
                }
                let result = self.execute_method(obj_id, method, &arg_vals)?;
                Ok(result)
            }
            IrExpr::MemberAccess { obj, field } => {
                let obj_val = self.evaluate_expr(obj)?;
                let obj_id = obj_val.to_u64() as ObjId;
                let obj_data = self
                    .state
                    .get_object(obj_id)
                    .ok_or_else(|| format!("object {} not found", obj_id))?;
                let val = obj_data
                    .fields
                    .get(field)
                    .cloned()
                    .unwrap_or_else(|| LogicVec::new(1));
                Ok(val)
            }
            IrExpr::DpiCall {
                name,
                args,
                return_width,
            } => self.evaluate_dpi_call(name, args, *return_width),
            IrExpr::HierRef(name) => {
                if let Some(sig_id) = self.find_signal(name) {
                    let mut val = self.state.read_signal(sig_id).clone();
                    sanitize_for_2state(&self.design.top.signals, sig_id, &mut val);
                    Ok(val)
                } else {
                    Err(SimError::runtime(format!(
                        "hierarchical signal '{}' not found",
                        name
                    )))
                }
            }
            IrExpr::Inside { expr, list } => {
                let val = self.evaluate_expr(expr)?;
                for item in list {
                    let item_val = self.evaluate_expr(item)?;
                    let eq = val.case_eq(&item_val);
                    if eq == LogicVec::from_u64(1, 1) {
                        return Ok(LogicVec::from_u64(1, 1));
                    }
                }
                Ok(LogicVec::from_u64(0, 1))
            }
            IrExpr::Dist { expr: _expr, items } => {
                // Dist expression in randomize context: use weighted random selection
                if self.current_method == Some(Symbol::intern("randomize")) {
                    let total_weight: i64 = items
                        .iter()
                        .map(|item| {
                            let count = match (item.range_lo, item.range_hi) {
                                (Some(lo), Some(hi)) if hi >= lo => (hi - lo + 1).max(1),
                                _ => 1,
                            };
                            match item.weight_type {
                                DistWeightType::Item => item.weight * count,
                                DistWeightType::Range => item.weight,
                            }
                        })
                        .sum();
                    if total_weight > 0 {
                        let r = (self.rng.gen::<u64>() % total_weight as u64) as i64;
                        let mut cumulative = 0i64;
                        for item in items {
                            let count = match (item.range_lo, item.range_hi) {
                                (Some(lo), Some(hi)) if hi >= lo => (hi - lo + 1).max(1),
                                _ => 1,
                            };
                            let step = match item.weight_type {
                                DistWeightType::Item => item.weight * count,
                                DistWeightType::Range => item.weight,
                            };
                            cumulative += step;
                            if r < cumulative {
                                let v = match (item.range_lo, item.range_hi) {
                                    (Some(lo), Some(hi)) if hi >= lo => {
                                        lo + (self.rng.gen::<u64>() % ((hi - lo + 1) as u64)) as i64
                                    }
                                    (Some(v), _) | (_, Some(v)) => v,
                                    _ => 0i64,
                                };
                                return Ok(LogicVec::from_u64(v as u64, 32));
                            }
                        }
                    }
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            IrExpr::Cast { width, expr } => {
                let val = self.evaluate_expr(expr)?;
                Ok(val.resize(*width))
            }
            IrExpr::StreamingConcat {
                op,
                slice_size,
                slices,
            } => {
                let mut vals = Vec::new();
                for sl in slices {
                    vals.push(self.evaluate_expr(sl)?);
                }
                let all_bits: Vec<LogicVal> =
                    vals.iter().flat_map(|v| v.bits.iter().copied()).collect();
                let slen = slice_size.unwrap_or(1);
                if slen == 0 {
                    return Err(SimError::runtime("streaming slice size must be > 0"));
                }
                let mut result = Vec::new();
                if op == ">>" {
                    // reverse bits within each slice, then reverse slice order
                    for chunk in all_bits.chunks(slen).rev() {
                        result.extend(chunk.iter().rev());
                    }
                } else {
                    // reverse slice order only
                    for chunk in all_bits.chunks(slen).rev() {
                        result.extend(chunk.iter());
                    }
                }
                Ok(LogicVec {
                    width: result.len(),
                    bits: result,
                })
            }
            IrExpr::UdpLookup { udp_name, args } => {
                let udp = self
                    .design
                    .udp_defs
                    .iter()
                    .find(|u| u.name == *udp_name)
                    .cloned()
                    .ok_or_else(|| format!("UDP '{}' not found", udp_name))?;
                let arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_expr(a))
                    .collect::<Result<_, _>>()?;

                // Get previous arg values for edge detection
                let prev_vals = self.udp_prev_args.get(udp_name.as_str());
                let current_bits: Vec<LogicVal> = arg_vals
                    .iter()
                    .map(|v| v.bits.first().copied().unwrap_or(LogicVal::X))
                    .collect();
                let prev_bits: Option<Vec<LogicVal>> = prev_vals.map(|pv| {
                    pv.iter()
                        .map(|v| v.bits.first().copied().unwrap_or(LogicVal::X))
                        .collect()
                });

                // Scan table entries for first match
                'table: for entry in &udp.table {
                    for (i, sym) in entry.inputs.iter().enumerate() {
                        let bit = current_bits.get(i).copied().unwrap_or(LogicVal::X);
                        let matched = match sym {
                            UdpSymbol::Zero => bit == LogicVal::Zero,
                            UdpSymbol::One => bit == LogicVal::One,
                            UdpSymbol::X => bit == LogicVal::X,
                            UdpSymbol::DontCare => true,
                            UdpSymbol::Edge(edge_str) => {
                                // Edge detection: compare prev vs current
                                if let Some(ref pb) = prev_bits {
                                    let prev_bit = pb.get(i).copied().unwrap_or(LogicVal::X);
                                    let chars: Vec<char> = edge_str.chars().collect();
                                    if chars.len() == 2 {
                                        sym_char_matches(chars[0], prev_bit)
                                            && sym_char_matches(chars[1], bit)
                                    } else {
                                        // Abbreviated edge: r, f, p, n, *
                                        edge_matches_abbrev(edge_str, prev_bit, bit)
                                    }
                                } else {
                                    // No previous value — can't detect edge
                                    false
                                }
                            }
                            UdpSymbol::NoChange => true,
                        };
                        if !matched {
                            continue 'table;
                        }
                    }
                    // All inputs matched — determine output
                    let result = match &entry.output {
                        UdpSymbol::Zero => LogicVec::fill(LogicVal::Zero, 1),
                        UdpSymbol::One => LogicVec::fill(LogicVal::One, 1),
                        UdpSymbol::X => LogicVec::fill(LogicVal::X, 1),
                        UdpSymbol::DontCare => LogicVec::fill(LogicVal::X, 1),
                        UdpSymbol::NoChange => {
                            // For sequential UDP, return the current output value (last arg = state)
                            arg_vals
                                .last()
                                .cloned()
                                .unwrap_or(LogicVec::fill(LogicVal::X, 1))
                        }
                        UdpSymbol::Edge(s) => {
                            let v = s
                                .chars()
                                .last()
                                .map(|c| match c {
                                    '0' => LogicVal::Zero,
                                    '1' => LogicVal::One,
                                    _ => LogicVal::X,
                                })
                                .unwrap_or(LogicVal::X);
                            LogicVec::fill(v, 1)
                        }
                    };
                    // Store current arg values for next evaluation
                    self.udp_prev_args
                        .insert(udp_name.clone(), arg_vals.clone());
                    return Ok(result);
                }
                // No match — return X (or retain current value for sequential)
                let result = if udp.is_sequential {
                    arg_vals
                        .last()
                        .cloned()
                        .unwrap_or(LogicVec::fill(LogicVal::X, 1))
                } else {
                    LogicVec::fill(LogicVal::X, 1)
                };
                self.udp_prev_args
                    .insert(udp_name.clone(), arg_vals.clone());
                Ok(result)
            }
            IrExpr::FuncCall { func_name, args } => {
                let name = func_name;
                // Check recursion depth
                let depth = self
                    .recursion_depth
                    .get(name.as_str())
                    .copied()
                    .unwrap_or(0);
                if depth >= self.max_recursion_depth {
                    return Err(SimError::runtime(format!(
                        "recursion depth exceeded for function '{}' (max {})",
                        name, self.max_recursion_depth
                    )));
                }
                self.recursion_depth.insert(name.clone(), depth + 1);

                // Find the function declaration
                let func = self
                    .design
                    .module_functions
                    .get(name.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        SimError::runtime(format!("function '{}' not found for runtime call", name))
                    })?;

                // Compute return width from function declaration
                let ret_width = if let Some(er) = &func.range {
                    if let (Ok(msb), Ok(lsb)) = (
                        crate::ast::types::const_eval_simple(&er.msb),
                        crate::ast::types::const_eval_simple(&er.lsb),
                    ) {
                        let msb = msb as usize;
                        let lsb = lsb as usize;
                        if msb >= lsb {
                            msb - lsb + 1
                        } else {
                            lsb - msb + 1
                        }
                    } else {
                        1
                    }
                } else {
                    match &func.return_type {
                        Some(dt) => match dt.as_ref() {
                            crate::ast::types::DataType::Void => 0,
                            crate::ast::types::DataType::Byte => 8,
                            crate::ast::types::DataType::Shortint => 16,
                            crate::ast::types::DataType::Int
                            | crate::ast::types::DataType::Integer => 32,
                            crate::ast::types::DataType::Longint => 64,
                            crate::ast::types::DataType::Time => 64,
                            _ => 1,
                        },
                        None => 1,
                    }
                };

                // Evaluate all arguments
                let arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_expr(a))
                    .collect::<Result<_, _>>()?;

                // Create new local scope
                let depth_idx = self.method_locals.len();
                let mut locals = HashMap::new();

                // Initialize return value slot (for Stmt::Return to write into via current_method)
                locals.insert("__func_ret".to_string(), LogicVec::new(ret_width.max(1)));

                // Bind arguments to port names
                for (i, arg_val) in arg_vals.into_iter().enumerate() {
                    if let Some(port) = func.ports.get(i) {
                        locals.insert(port.name.clone(), arg_val);
                    }
                }

                // Initialize internal variables with X
                for decl in &func.decls {
                    for var in &decl.names {
                        if !locals.contains_key(var.name.as_str()) {
                            let width = if let Some(r) = &var.range {
                                r.width()
                            } else {
                                1
                            };
                            locals.insert(var.name.clone(), LogicVec::new(width));
                        }
                    }
                }

                self.method_locals.push(locals);

                // Save and set current_method so Stmt::Return stores into method_locals
                let saved_method = self.current_method.take();
                self.current_method = Some(Symbol::intern("__func_ret"));

                self.evaluate_ast_block_with_delay_fork(&func.stmts, None)?;

                // Restore current_method
                self.current_method = saved_method;

                // Read return value from method_locals
                let return_val = if ret_width > 0 {
                    self.get_local("__func_ret")
                        .unwrap_or_else(|| LogicVec::new(ret_width))
                } else {
                    LogicVec::new(0)
                };

                // Restore scope
                self.method_locals.truncate(depth_idx);
                self.recursion_depth.insert(name.clone(), depth);

                Ok(return_val)
            }
            IrExpr::VifBinding { instance_name } => {
                // Look up the instance in the signal hierarchy
                // Find the first signal belonging to this instance and return its SignalId as binding handle
                let mut binding_handle: Option<usize> = None;
                let prefix = format!("{instance_name}.");
                for (sid, sig) in self.design.top.signals.iter().enumerate() {
                    if sig.name.starts_with(&prefix) || sig.name == *instance_name {
                        binding_handle = Some(sid);
                        break;
                    }
                }
                if let Some(handle) = binding_handle {
                    return Ok(LogicVec::from_u64(handle as u64, 64));
                }
                // Fallback: match instance name as any path component: top.instance.sig
                let target = instance_name.as_str();
                for (sid, sig) in self.design.top.signals.iter().enumerate() {
                    let parts: Vec<&str> = sig.name.split('.').collect();
                    if parts.iter().any(|p| *p == target) {
                        binding_handle = Some(sid);
                        break;
                    }
                }
                match binding_handle {
                    Some(handle) => Ok(LogicVec::from_u64(handle as u64, 64)),
                    None => Ok(LogicVec::fill(LogicVal::X, 64)),
                }
            }
            IrExpr::VirtualIfaceAccess {
                vif_name,
                field,
                field_width,
            } => {
                // Find the vif signal and read its binding handle (SignalId of a signal in the bound instance)
                let mut result = LogicVec::fill(LogicVal::X, *field_width);
                for (sid, sig) in self.design.top.signals.iter().enumerate() {
                    if sig.iface_type.is_some() && sig.name == *vif_name {
                        let binding_val = self.state.read_signal(sid);
                        let handle = binding_val.to_u64() as usize;
                        if handle > 0 && handle < self.design.top.signals.len() {
                            // Bound — extract instance path from the bound signal's name
                            let bound_sig_name = self.design.top.signals[handle].name.as_str();
                            // Strip the signal name to get instance path: top.inst.sig -> top.inst
                            if let Some(dot_pos) = bound_sig_name.rfind('.') {
                                let inst_path = &bound_sig_name[..dot_pos];
                                let sig_key = format!("{}.{}", inst_path, field);
                                if let Some(&field_sid) = self.design.hier_signal_map.get::<str>(sig_key.as_str())
                                {
                                    result = self.state.read_signal(field_sid).clone();
                                }
                            }
                        }
                        break;
                    }
                }
                Ok(result)
            }
        }
    }

    pub(crate) fn write_lvalue(&mut self, lvalue: &IrLValue, mut val: LogicVec) -> Result<(), SimError> {
        // Check for const violation
        if let Some(id) = self.signal_id_from_lvalue(lvalue) {
            if let Some(sig) = self.design.top.signals.get(id) {
                if sig.is_const {
                    return Err(SimError::runtime(format!(
                        "cannot write to const signal '{}'",
                        sig.name
                    )));
                }
            }
        }
        match lvalue {
            IrLValue::Signal(id, _) => {
                sanitize_for_2state(&self.design.top.signals, *id, &mut val);
                let is_str = self
                    .design
                    .top
                    .signals
                    .get(*id)
                    .map(|s| s.is_string)
                    .unwrap_or(false);
                let sig_info = self.design.top.signals.get(*id).cloned();
                let is_dyn = sig_info
                    .as_ref()
                    .map(|s| s.is_dynamic || s.is_queue)
                    .unwrap_or(false);
                let resized = if is_str || is_dyn {
                    val
                } else {
                    let target_width = self.state.read_signal(*id).width;
                    if val.width != target_width {
                        val.resize(target_width)
                    } else {
                        val
                    }
                };
                // Apply resolution for multi-driver nets
                if let Some(ref info) = sig_info {
                    if info.multi_driver
                        && (info.kind == SignalKind::Wire || info.kind == SignalKind::Inout)
                    {
                        let current = self.state.read_signal(*id).clone();
                        let resolved = resolve_net_values(info.net_type, &current, &resized);
                        self.state.write_signal(*id, resolved);
                        return Ok(());
                    }
                }
                self.state.write_signal(*id, resized);
                self.signal_last_change.insert(*id, self.state.time);
            }
            IrLValue::RangeSelect(sig_id, msb, lsb) => {
                sanitize_for_2state(&self.design.top.signals, *sig_id, &mut val);
                let mut existing = self.state.read_signal(*sig_id).clone();
                let (start, end) = if *msb > *lsb {
                    (*lsb, *msb)
                } else {
                    (*msb, *lsb)
                };
                for (i, b) in val.bits.iter().enumerate() {
                    if start + i <= end {
                        existing.bits[start + i] = *b;
                    }
                }
                self.state.write_signal(*sig_id, existing);
                self.signal_last_change.insert(*sig_id, self.state.time);
            }
            IrLValue::BitSelect(sig_id, idx) => {
                sanitize_for_2state(&self.design.top.signals, *sig_id, &mut val);
                let mut existing = self.state.read_signal(*sig_id).clone();
                if let Some(b) = val.bits.first() {
                    if *idx < existing.bits.len() {
                        existing.bits[*idx] = *b;
                    }
                }
                self.state.write_signal(*sig_id, existing);
                self.signal_last_change.insert(*sig_id, self.state.time);
            }
            IrLValue::ArrayIndex {
                sig_id,
                index,
                elem_width,
            } => {
                let key_val = self.evaluate_expr(index)?;
                // Check if this is an associative array
                let sig_info = self.design.top.signals.get(*sig_id);
                if sig_info.map(|s| s.is_associative).unwrap_or(false) {
                    sanitize_for_2state(&self.design.top.signals, *sig_id, &mut val);
                    let assoc_map = self.assoc_data.entry(*sig_id).or_insert_with(HashMap::new);
                    assoc_map.insert(key_val, val);
                    return Ok(());
                }
                sanitize_for_2state(&self.design.top.signals, *sig_id, &mut val);
                let mut existing = self.state.read_signal(*sig_id).clone();
                let idx = key_val.to_u64() as usize;
                let start = idx * elem_width;
                let needed = start + elem_width;
                if needed > existing.width {
                    existing.bits.resize(needed, LogicVal::X);
                    existing.width = needed;
                }
                for (i, b) in val.bits.iter().enumerate() {
                    if start + i < needed {
                        existing.bits[start + i] = *b;
                    }
                }
                self.state.write_signal(*sig_id, existing);
                self.signal_last_change.insert(*sig_id, self.state.time);
            }
            IrLValue::ArrayRangeSelect {
                sig_id,
                index,
                elem_width,
                msb,
                lsb,
            } => {
                let mut existing = self.state.read_signal(*sig_id).clone();
                let idx_val = self.evaluate_expr(index)?;
                let idx = idx_val.to_u64() as usize;
                let base = idx * elem_width;
                let (start, end) = if *msb > *lsb {
                    (*lsb, *msb)
                } else {
                    (*msb, *lsb)
                };
                let abs_start = base + start;
                for (i, b) in val.bits.iter().enumerate() {
                    if abs_start + i <= base + end {
                        existing.bits[abs_start + i] = *b;
                    }
                }
                self.state.write_signal(*sig_id, existing);
                self.signal_last_change.insert(*sig_id, self.state.time);
            }
            IrLValue::ArrayBitSelect {
                sig_id,
                index,
                elem_width,
                bit,
            } => {
                let mut existing = self.state.read_signal(*sig_id).clone();
                let idx_val = self.evaluate_expr(index)?;
                let idx = idx_val.to_u64() as usize;
                let abs_idx = idx * elem_width + bit;
                if let Some(b) = val.bits.first() {
                    if abs_idx < existing.bits.len() {
                        existing.bits[abs_idx] = *b;
                    }
                }
                self.state.write_signal(*sig_id, existing);
                self.signal_last_change.insert(*sig_id, self.state.time);
            }
            IrLValue::Concat(parts) => {
                let mut offset = 0;
                for part in parts {
                    let w = self.get_lvalue_width(part);
                    let sub_val = if offset + w <= val.width {
                        LogicVec {
                            bits: val.bits[offset..offset + w].to_vec(),
                            width: w,
                        }
                    } else {
                        LogicVec::new(w)
                    };
                    self.write_lvalue(part, sub_val)?;
                    offset += w;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn check_timing_constraints(&mut self) -> Result<(), SimError> {
        let current_time = self.state.time;
        let signal_names: Vec<(String, SignalId)> = self
            .design
            .top
            .signals
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.to_string(), i))
            .collect();
        let items = self.design.specify_items.clone();
        for item in &items {
            match item {
                SpecifyItem::SetupCheck {
                    data,
                    ref_event: _ref_event,
                    limit,
                } => {
                    // _ref_event is parsed but runtime edge detection is simplified
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident(data_sig) = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta <= limit_val && delta > 0 {
                                    eprintln!("TIMING WARNING: $setup violation: data '{}' changed {}ns before ref (limit={}ns)",
                                        data_sig, delta, limit_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::HoldCheck {
                    ref_event: _ref_event,
                    data,
                    limit,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident(data_sig) = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta <= limit_val {
                                    eprintln!("TIMING WARNING: $hold violation: data '{}' changed {}ns before ref (limit={}ns)",
                                        data_sig, delta, limit_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::SetupHoldCheck {
                    ref_event: _ref_event,
                    data,
                    setup_limit,
                    hold_limit,
                } => {
                    let setup_val = const_eval_simple(setup_limit).unwrap_or(0) as u64;
                    let hold_val = const_eval_simple(hold_limit).unwrap_or(0) as u64;
                    if let Expr::Ident(data_sig) = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta <= setup_val {
                                    eprintln!("TIMING WARNING: $setuphold (setup) violation: data '{}' changed {}ns before ref (setup={}ns)",
                                        data_sig, delta, setup_val);
                                }
                                if delta > 0 && delta <= hold_val {
                                    eprintln!("TIMING WARNING: $setuphold (hold) violation: data '{}' changed {}ns before ref (hold={}ns)",
                                        data_sig, delta, hold_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::RecoveryCheck {
                    data,
                    ref_event: _ref_event,
                    limit,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident(data_sig) = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta <= limit_val {
                                    eprintln!("TIMING WARNING: $recovery violation: signal '{}' changed {}ns before ref (limit={}ns)", data_sig, delta, limit_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::RemovalCheck {
                    ref_event: _ref_event,
                    data,
                    limit,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident(data_sig) = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta <= limit_val {
                                    eprintln!("TIMING WARNING: $removal violation: signal '{}' changed {}ns before ref (limit={}ns)", data_sig, delta, limit_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::RecoveryRemovalCheck {
                    ref_event: _ref_event,
                    data,
                    recovery_limit,
                    removal_limit,
                } => {
                    let recov_val = const_eval_simple(recovery_limit).unwrap_or(0) as u64;
                    let remov_val = const_eval_simple(removal_limit).unwrap_or(0) as u64;
                    if let Expr::Ident(data_sig) = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta <= recov_val {
                                    eprintln!("TIMING WARNING: $recrem (recovery) violation: signal '{}' changed {}ns before ref (recov={}ns)", data_sig, delta, recov_val);
                                }
                                if delta > 0 && delta <= remov_val {
                                    eprintln!("TIMING WARNING: $recrem (removal) violation: signal '{}' changed {}ns before ref (remov={}ns)", data_sig, delta, remov_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::PeriodCheck { ref_event, limit } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident(ref_sig) = ref_event {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == ref_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta < limit_val {
                                    eprintln!("TIMING WARNING: $period violation: signal '{}' period {}ns < minimum {}ns", ref_sig, delta, limit_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::WidthCheck {
                    ref_event,
                    limit,
                    threshold: _threshold,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident(ref_sig) = ref_event {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == ref_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta < limit_val {
                                    eprintln!("TIMING WARNING: $width violation: signal '{}' pulse width {}ns < minimum {}ns", ref_sig, delta, limit_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::SkewCheck {
                    ref_event,
                    data,
                    limit,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident(data_sig) = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&data_change) = self.signal_last_change.get(sid) {
                                if let Expr::Ident(ref_sig) = &ref_event {
                                    if let Some((_, rsid)) =
                                        signal_names.iter().find(|(n, _)| n.as_str() == ref_sig.as_str())
                                    {
                                        if let Some(&ref_change) = self.signal_last_change.get(rsid)
                                        {
                                            let skew = if data_change > ref_change {
                                                data_change - ref_change
                                            } else {
                                                ref_change - data_change
                                            };
                                            if skew > limit_val {
                                                eprintln!("TIMING WARNING: $skew violation: skew {}ns > max {}ns between '{}' and '{}'", skew, limit_val, data_sig, ref_sig);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                SpecifyItem::TimeskewCheck {
                    ref_event,
                    data,
                    limit,
                    threshold: _threshold,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident(data_sig) = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&data_change) = self.signal_last_change.get(sid) {
                                if let Expr::Ident(ref_sig) = &ref_event {
                                    if let Some((_, rsid)) =
                                        signal_names.iter().find(|(n, _)| n.as_str() == ref_sig.as_str())
                                    {
                                        if let Some(&ref_change) = self.signal_last_change.get(rsid)
                                        {
                                            let skew = if data_change > ref_change {
                                                data_change - ref_change
                                            } else {
                                                ref_change - data_change
                                            };
                                            if skew > limit_val {
                                                eprintln!("TIMING WARNING: $timeskew violation: skew {}ns > max {}ns between '{}' and '{}'", skew, limit_val, data_sig, ref_sig);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                SpecifyItem::NochangeCheck {
                    ref_event: _ref_event,
                    data,
                    start_limit,
                    end_limit,
                } => {
                    let start_val = const_eval_simple(start_limit).unwrap_or(0) as u64;
                    let end_val = const_eval_simple(end_limit).unwrap_or(0) as u64;
                    if let Expr::Ident(data_sig) = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta >= start_val && delta <= end_val {
                                    eprintln!("TIMING WARNING: $nochange violation: signal '{}' changed within window [{}ns, {}ns] (delta={}ns)", data_sig, start_val, end_val, delta);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn evaluate_dpi_call(
        &mut self,
        name: &str,
        args: &[IrExpr],
        return_width: usize,
    ) -> Result<LogicVec, SimError> {
        // Check if we have a matching DPI import
        let dpi = self
            .design
            .dpi_imports
            .iter()
            .find(|d| d.name == name)
            .ok_or_else(|| format!("DPI function '{}' not found in imports", name))?;
        if dpi.is_task {
            return Ok(LogicVec::new(0));
        }
        let arg_vals: Vec<LogicVec> = args
            .iter()
            .map(|a| self.evaluate_expr(a))
            .collect::<Result<_, _>>()?;
        // Known DPI functions
        match name {
            "svBitToInt" | "svToInt" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(LogicVec::from_u64(val.to_u64(), return_width));
                }
                return Ok(LogicVec::from_u64(0, return_width));
            }
            "svBitToLong" | "svToLong" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(LogicVec::from_u64(val.to_u64(), return_width));
                }
                return Ok(LogicVec::from_u64(0, return_width));
            }
            "svToShortReal" | "svToReal" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(val.clone());
                }
                return Ok(LogicVec::from_u64(0, return_width));
            }
            "svIntToBit" | "svToBit" | "svToLogic" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(val.clone());
                }
                return Ok(LogicVec::from_u64(0, return_width));
            }
            "svBitToBitVal" | "svBitToLogicVal" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(val.clone());
                }
                return Ok(LogicVec::from_u64(0, return_width));
            }
            "svRandomize" | "sv$random" | "svUrandom" | "svUrandomRange" => {
                let r: u64 = self.rng.gen();
                Ok(LogicVec::from_u64(r, return_width))
            }
            "$test$plusargs" | "svTestPlusArgs" => {
                // Handled in SysFunc dispatch — fallback here
                Ok(LogicVec::from_u64(0, return_width))
            }
            "$value$plusargs" | "svValuePlusArgs" => Ok(LogicVec::from_u64(0, return_width)),
            _ => {
                // Unknown DPI — return 0
                Ok(LogicVec::from_u64(0, return_width))
            }
        }
    }

    fn get_lvalue_width(&self, lvalue: &IrLValue) -> usize {
        match lvalue {
            IrLValue::Signal(id, _) => self.state.read_signal(*id).width,
            IrLValue::RangeSelect(_, msb, lsb) => {
                if *msb > *lsb {
                    msb - lsb + 1
                } else {
                    lsb - msb + 1
                }
            }
            IrLValue::BitSelect(_, _) => 1,
            IrLValue::ArrayIndex { elem_width, .. } => *elem_width,
            IrLValue::ArrayRangeSelect { msb, lsb, .. } => {
                if *msb > *lsb {
                    msb - lsb + 1
                } else {
                    lsb - msb + 1
                }
            }
            IrLValue::ArrayBitSelect { .. } => 1,
            IrLValue::Concat(parts) => parts.iter().map(|p| self.get_lvalue_width(p)).sum(),
        }
    }

    pub(crate) fn get_local(&self, name: &str) -> Option<LogicVec> {
        for scope in self.method_locals.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v.clone());
            }
        }
        None
    }

    fn set_local(&mut self, name: &str, val: LogicVec) {
        if let Some(scope) = self.method_locals.last_mut() {
            scope.insert(name.to_string(), val);
        }
    }

    pub(crate) fn write_ast_lvalue(&mut self, lhs: &crate::ast::Expr, val: LogicVec) -> Result<(), SimError> {
        match lhs {
            crate::ast::Expr::Ident(name) => self.write_local_or_field(name, val),
            crate::ast::Expr::MemberAccess { obj, field } => {
                let obj_val = self.evaluate_ast_expr(obj)?;
                let obj_id = obj_val.to_u64() as ObjId;
                if let Some(obj_data) = self.state.get_object_mut(obj_id) {
                    obj_data.fields.insert(field.clone(), val);
                    Ok(())
                } else {
                    Err(SimError::runtime(format!(
                        "object {} not found for field '{}'",
                        obj_id, field
                    )))
                }
            }
            _ => Err(SimError::runtime(format!(
                "unsupported lvalue type in task method: {:?}",
                lhs
            ))),
        }
    }

    pub(crate) fn ast_lvalue_to_ir(&self, lhs: &crate::ast::Expr) -> Option<IrLValue> {
        match lhs {
            crate::ast::Expr::Ident(name) => {
                let sig_id = self.find_signal(name)?;
                Some(IrLValue::Signal(sig_id, 0))
            }
            _ => None,
        }
    }

    pub(crate) fn find_ast_signal_id(&self, expr: &crate::ast::Expr) -> Option<SignalId> {
        match expr {
            crate::ast::Expr::Ident(name) => self.find_signal(name),
            _ => None,
        }
    }

    pub(crate) fn handle_ast_syscall(
        &mut self,
        name: &str,
        args: &[crate::ast::Expr],
    ) -> Result<(), SimError> {
        if name == "display" || name == "write" {
            let ir_args: Vec<IrExpr> = args
                .iter()
                .map(|a| IrExpr::Const(self.evaluate_ast_expr(a).unwrap_or(LogicVec::new(32))))
                .collect();
            let msg = format_display(
                &self.state,
                &self.design.top.signals,
                &self.design.hier_signal_map,
                &self.assoc_data,
                &ir_args,
            );
            print!("{}", msg);
        } else if name == "finish" {
            self.running = false;
        }
        Ok(())
    }

    fn write_local_or_field(&mut self, name: &str, val: LogicVec) -> Result<(), SimError> {
        if self.get_local(name).is_some() {
            self.set_local(name, val);
            return Ok(());
        }
        if let Some(obj_id) = self.current_this {
            if let Some(obj) = self.state.get_object_mut(obj_id) {
                obj.fields.insert(name.to_string(), val);
                return Ok(());
            }
        }
        Err(SimError::runtime(format!(
            "cannot resolve '{}' in method context (not a local or field)",
            name
        )))
    }

    pub(crate) fn evaluate_ast_expr(&mut self, expr: &Expr) -> Result<LogicVec, SimError> {
        match expr {
            Expr::Value(v) => match v {
                Value::Decimal(i) => Ok(LogicVec::from_u64(*i as u64, 32)),
                Value::Binary { bits, .. } => {
                    LogicVec::from_bin(bits).map_err(|e| SimError::runtime(e))
                }
                Value::Hex { bits, .. } => {
                    LogicVec::from_hex(bits).map_err(|e| SimError::runtime(e))
                }
                Value::Octal { bits, .. } => {
                    LogicVec::from_hex(bits).map_err(|e| SimError::runtime(e))
                }
                Value::Real(r) => Ok(LogicVec::from_u64(r.to_bits(), 64)),
            },
            Expr::Ident(name) => {
                if name == "this" {
                    if let Some(obj_id) = self.current_this {
                        return Ok(LogicVec::from_u64(obj_id as u64, 64));
                    } else {
                        return Err(SimError::runtime("'this' used outside of class method"));
                    }
                }
                if let Some(local) = self.get_local(name) {
                    return Ok(local);
                }
                if let Some(obj_id) = self.current_this {
                    if let Some(obj) = self.state.get_object(obj_id) {
                        if let Some(val) = obj.fields.get(name) {
                            return Ok(val.clone());
                        }
                    }
                }
                if let Some(sig_id) = self.find_signal(name) {
                    return Ok(self.state.read_signal(sig_id).clone());
                }
                let ctx = self
                    .current_this
                    .map(|id| format!("obj_id={}", id))
                    .unwrap_or_else(|| "no current_this".to_string());
                Err(SimError::runtime(format!(
                    "cannot resolve identifier '{}' in method context ({})",
                    name, ctx
                )))
            }
            Expr::BinaryOp { op, lhs, rhs } => {
                let lval = self.evaluate_ast_expr(lhs)?;
                let rval = self.evaluate_ast_expr(rhs)?;
                let ir_op = map_ast_binary_op(op)?;
                Ok(eval_binary(ir_op, &lval, &rval))
            }
            Expr::UnaryOp { op, expr: inner } => {
                let val = self.evaluate_ast_expr(inner)?;
                let ir_op = map_ast_unary_op(op)?;
                Ok(eval_unary(ir_op, &val))
            }
            Expr::Concat(parts) => {
                let mut result = LogicVec::new(0);
                for p in parts.iter().rev() {
                    let part = self.evaluate_ast_expr(p)?;
                    result = result.extend(&part);
                }
                Ok(result)
            }
            Expr::Replicate { count, expr: inner } => {
                let count_val = self.evaluate_ast_expr(count)?;
                let n = count_val.to_u64() as usize;
                let val = self.evaluate_ast_expr(inner)?;
                let mut result = LogicVec::new(0);
                for _ in 0..n {
                    result = result.extend(&val);
                }
                Ok(result)
            }
            Expr::TernaryOp {
                cond,
                true_expr,
                false_expr,
            } => {
                let cval = self.evaluate_ast_expr(cond)?;
                if cval.to_bool().unwrap_or(false) {
                    self.evaluate_ast_expr(true_expr)
                } else {
                    self.evaluate_ast_expr(false_expr)
                }
            }
            Expr::FuncCall { name, args } if name == "new" => {
                let _arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                let obj_id = self.state.alloc_object("");
                Ok(LogicVec::from_u64(obj_id as u64, 64))
            }
            Expr::FuncCall { name, args } if name.ends_with("::new") => {
                let raw_name = name.strip_suffix("::new").unwrap().to_string();
                let is_builtin = matches!(
                    raw_name.as_str(),
                    "uvm_object"
                        | "uvm_component"
                        | "uvm_sequence_item"
                        | "uvm_sequence"
                        | "uvm_sequencer"
                        | "uvm_driver"
                        | "uvm_monitor"
                        | "uvm_scoreboard"
                        | "uvm_analysis_port"
                        | "uvm_analysis_imp"
                        | "uvm_test"
                        | "uvm_report_object"
                        | "uvm_factory"
                        | "uvm_resource_db"
                );
                let effective = if is_builtin {
                    format!("__{}", raw_name)
                } else {
                    raw_name.clone()
                };
                let effective = self
                    .factory_type_overrides
                    .get::<str>(effective.as_str())
                    .unwrap_or(&effective)
                    .clone();
                let obj_id = self.state.alloc_object(&effective);
                let arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                // Initialize built-in data
                if is_builtin {
                    if raw_name == "uvm_analysis_port" {
                        let pname = if !arg_vals.is_empty() {
                            logicvec_to_string(&arg_vals[0])
                        } else {
                            String::new()
                        };
                        self.uvm_analysis_port_data.insert(
                            obj_id,
                            UvmAnalysisPortData {
                                connections: Vec::new(),
                                name: pname.clone(),
                            },
                        );
                        self.uvm_object_data
                            .entry(obj_id)
                            .or_insert_with(|| UvmObjectData { name: pname });
                    } else if raw_name == "uvm_analysis_imp" {
                        let pname = if !arg_vals.is_empty() {
                            logicvec_to_string(&arg_vals[0])
                        } else {
                            String::new()
                        };
                        let parent_obj = arg_vals.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                        self.uvm_analysis_imp_data.insert(
                            obj_id,
                            UvmAnalysisImpData {
                                parent: if parent_obj != 0 {
                                    Some(parent_obj)
                                } else {
                                    None
                                },
                                name: pname.clone(),
                            },
                        );
                        self.uvm_object_data
                            .entry(obj_id)
                            .or_insert_with(|| UvmObjectData { name: pname });
                    }
                }
                if self.find_method_in_hierarchy(&effective, "new").is_ok() {
                    self.execute_method(obj_id, "new", &arg_vals)?;
                }
                Ok(LogicVec::from_u64(obj_id as u64, 64))
            }
            Expr::FuncCall { name, args } if name == "uvm_config_db::set" => {
                let arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                let inst_name = if arg_vals.len() > 1 {
                    logicvec_to_string(&arg_vals[1])
                } else {
                    String::new()
                };
                let field_name = if arg_vals.len() > 2 {
                    logicvec_to_string(&arg_vals[2])
                } else {
                    String::new()
                };
                let value = if arg_vals.len() > 3 {
                    arg_vals[3].clone()
                } else {
                    LogicVec::new(1)
                };
                self.uvm_config_db_data
                    .insert((inst_name, field_name), value);
                Ok(LogicVec::from_u64(1, 1))
            }
            Expr::FuncCall { name, args } if name == "uvm_config_db::get" => {
                let arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                let inst_name = if arg_vals.len() > 1 {
                    logicvec_to_string(&arg_vals[1])
                } else {
                    String::new()
                };
                let field_name = if arg_vals.len() > 2 {
                    logicvec_to_string(&arg_vals[2])
                } else {
                    String::new()
                };
                let key = (inst_name, field_name);
                let stored = self.uvm_config_db_data.get::<(Symbol, Symbol)>(&(Symbol::intern(&key.0), Symbol::intern(&key.1))).cloned();
                if let Some(val) = stored {
                    if let Some(last_arg) = args.get(3) {
                        match last_arg {
                            Expr::Ident(var) => {
                                self.write_local_or_field(var, val.clone())?;
                            }
                            Expr::MemberAccess { obj, field } => {
                                let obj_val = self.evaluate_ast_expr(obj)?;
                                let obj_id = obj_val.to_u64() as ObjId;
                                if let Some(obj) = self.state.get_object_mut(obj_id) {
                                    obj.fields.insert(field.clone(), val.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                    Ok(LogicVec::from_u64(1, 1))
                } else {
                    Ok(LogicVec::from_u64(0, 1))
                }
            }
            Expr::FuncCall { name, args } if name == "uvm_resource_db::set" => {
                let arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                let scope = if !arg_vals.is_empty() {
                    logicvec_to_string(&arg_vals[0])
                } else {
                    String::new()
                };
                let rname = if arg_vals.len() > 1 {
                    logicvec_to_string(&arg_vals[1])
                } else {
                    String::new()
                };
                let value = if arg_vals.len() > 2 {
                    arg_vals[2].clone()
                } else {
                    LogicVec::new(1)
                };
                self.uvm_resource_db_data.insert((scope, rname), value);
                Ok(LogicVec::from_u64(1, 1))
            }
            Expr::FuncCall { name, args } if name == "uvm_resource_db::get" => {
                let arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                let scope = if !arg_vals.is_empty() {
                    logicvec_to_string(&arg_vals[0])
                } else {
                    String::new()
                };
                let rname = if arg_vals.len() > 1 {
                    logicvec_to_string(&arg_vals[1])
                } else {
                    String::new()
                };
                let key = (scope, rname);
                let stored = self.uvm_resource_db_data.get::<(Symbol, Symbol)>(&(Symbol::intern(&key.0), Symbol::intern(&key.1))).cloned();
                if let Some(val) = stored {
                    if let Some(last_arg) = args.get(2) {
                        match last_arg {
                            Expr::Ident(var) => {
                                self.write_local_or_field(var, val.clone())?;
                            }
                            Expr::MemberAccess { obj, field } => {
                                let obj_val = self.evaluate_ast_expr(obj)?;
                                let obj_id = obj_val.to_u64() as ObjId;
                                if let Some(obj) = self.state.get_object_mut(obj_id) {
                                    obj.fields.insert(field.clone(), val.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                    Ok(LogicVec::from_u64(1, 1))
                } else {
                    Ok(LogicVec::from_u64(0, 1))
                }
            }
            Expr::FuncCall { name, args } if name == "uvm_factory::set_type_override_by_type" => {
                let arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                let orig = if !arg_vals.is_empty() {
                    logicvec_to_string(&arg_vals[0])
                } else {
                    String::new()
                };
                let override_type = if arg_vals.len() > 1 {
                    logicvec_to_string(&arg_vals[1])
                } else {
                    String::new()
                };
                self.factory_type_overrides.insert(Symbol::intern(&orig), Symbol::intern(&override_type));
                Ok(LogicVec::from_u64(1, 1))
            }
            Expr::FuncCall { name, args } => {
                let _arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                if name == "$clog2" {
                    if let Some(arg) = args.first() {
                        let val = self.evaluate_ast_expr(arg)?;
                        let n = val.to_u64();
                        if n <= 1 {
                            return Ok(LogicVec::from_u64(0, 32));
                        }
                        let msb = (64 - n.leading_zeros()) as u64;
                        let result = if n.is_power_of_two() { msb - 1 } else { msb };
                        return Ok(LogicVec::from_u64(result, 32));
                    }
                }
                Err(SimError::runtime(format!(
                    "unknown function '{}' in method context",
                    name
                )))
            }
            Expr::MethodCall {
                obj,
                method,
                args,
                with_clause: _,
            } => {
                if let Expr::Ident(s) = obj.as_ref() {
                    if s == "super" {
                        let arg_vals: Vec<LogicVec> = args
                            .iter()
                            .map(|a| self.evaluate_ast_expr(a))
                            .collect::<Result<_, _>>()?;
                        return self.execute_super_method(method, &arg_vals);
                    }
                }
                let obj_val = self.evaluate_ast_expr(obj)?;
                let obj_id = obj_val.to_u64() as ObjId;
                let arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                self.execute_method(obj_id, method, &arg_vals)
            }
            Expr::MemberAccess { obj, field } => {
                // Try hierarchical signal reference first
                let hier_name = Self::build_hier_name(obj, field);
                if let Some(sig_id) = self.find_signal(&hier_name) {
                    return Ok(self.state.read_signal(sig_id).clone());
                }
                // Fall back to object field access (class objects)
                let obj_val = self.evaluate_ast_expr(obj)?;
                let obj_id = obj_val.to_u64() as ObjId;
                let obj_data = self
                    .state
                    .get_object(obj_id)
                    .ok_or_else(|| format!("object {} not found", obj_id))?;
                Ok(obj_data
                    .fields
                    .get(field)
                    .cloned()
                    .unwrap_or_else(|| LogicVec::new(1)))
            }
            Expr::BitSelect { expr: inner, index } => {
                let val = self.evaluate_ast_expr(inner)?;
                let idx_val = self.evaluate_ast_expr(index)?;
                let i = idx_val.to_u64() as usize;
                // Check if this is an array field access (extract element, not bit)
                if let Some(elem_width) = self.get_field_elem_width(inner) {
                    let start = i * elem_width;
                    let end = (start + elem_width).min(val.width);
                    let mut bits = val.bits[start..end].to_vec();
                    if bits.len() < elem_width {
                        bits.resize(elem_width, LogicVal::X);
                    }
                    Ok(LogicVec {
                        width: bits.len(),
                        bits,
                    })
                } else {
                    let bit = val.bits.get(i).copied().unwrap_or(LogicVal::X);
                    Ok(LogicVec {
                        bits: vec![bit],
                        width: 1,
                    })
                }
            }
            Expr::RangeSelect {
                expr: inner,
                msb,
                lsb,
            } => {
                let val = self.evaluate_ast_expr(inner)?;
                let msb_val = self.evaluate_ast_expr(msb)?;
                let lsb_val = self.evaluate_ast_expr(lsb)?;
                let m = msb_val.to_u64() as usize;
                let l = lsb_val.to_u64() as usize;
                let (start, end) = if m > l { (l, m) } else { (m, l) };
                let mut bits = val.bits[start..=end].to_vec();
                if m > l {
                    bits.reverse();
                }
                Ok(LogicVec {
                    width: bits.len(),
                    bits,
                })
            }
            Expr::PartSelect {
                expr: inner,
                base,
                width,
            } => {
                let val = self.evaluate_ast_expr(inner)?;
                let base_val = self.evaluate_ast_expr(base)?;
                let width_val = self.evaluate_ast_expr(width)?;
                let b = base_val.to_u64() as usize;
                let w = width_val.to_u64() as usize;
                if b + w <= val.width && w > 0 {
                    let mut bits = val.bits[b..b + w].to_vec();
                    bits.reverse();
                    Ok(LogicVec { width: w, bits })
                } else if w == 0 {
                    Ok(LogicVec::from_u64(0, 1))
                } else {
                    Err(SimError::runtime(format!("part-select out of range")))
                }
            }
            Expr::Paren(inner) => self.evaluate_ast_expr(inner),
            Expr::String(s) => {
                let mut bits = Vec::with_capacity(s.len() * 8);
                for c in s.chars() {
                    let byte = c as u8;
                    for i in 0..8 {
                        bits.push(if (byte >> i) & 1 == 1 {
                            LogicVal::One
                        } else {
                            LogicVal::Zero
                        });
                    }
                }
                Ok(LogicVec {
                    width: bits.len(),
                    bits,
                })
            }
            Expr::Null => Ok(LogicVec::from_u64(0, 64)),
            Expr::FillLit(v) => Ok(LogicVec::fill(*v, 1)),
            Expr::Inside {
                expr: inner,
                range_list,
            } => {
                let val = self.evaluate_ast_expr(inner)?;
                for item in range_list {
                    let item_val = self.evaluate_ast_expr(item)?;
                    let eq = val.case_eq(&item_val);
                    if eq == LogicVec::from_u64(1, 1) {
                        return Ok(LogicVec::from_u64(1, 1));
                    }
                }
                Ok(LogicVec::from_u64(0, 1))
            }
            Expr::StreamingConcat {
                op,
                slice_size,
                slices,
            } => {
                let mut vals = Vec::new();
                for sl in slices {
                    vals.push(self.evaluate_ast_expr(sl)?);
                }
                let all_bits: Vec<LogicVal> =
                    vals.iter().flat_map(|v| v.bits.iter().copied()).collect();
                let slen = if let Some(ss_expr) = slice_size {
                    let ss_val = self.evaluate_ast_expr(ss_expr)?;
                    let n = ss_val.to_u64() as usize;
                    if n == 0 {
                        return Err(SimError::runtime("streaming slice size must be > 0"));
                    }
                    n
                } else {
                    1
                };
                let mut result = Vec::new();
                if op == ">>" {
                    for chunk in all_bits.chunks(slen).rev() {
                        result.extend(chunk.iter().rev());
                    }
                } else {
                    for chunk in all_bits.chunks(slen).rev() {
                        result.extend(chunk.iter());
                    }
                }
                Ok(LogicVec {
                    width: result.len(),
                    bits: result,
                })
            }
            Expr::Dist { expr: inner, items } => {
                let inner_val = self.evaluate_ast_expr(inner)?;
                let ir_items = items
                    .iter()
                    .map(|di| match di {
                        crate::ast::DistItem::Value(e, crate::ast::DistWeight::Item(w)) => {
                            let ev = self
                                .evaluate_ast_expr(e)
                                .unwrap_or(LogicVec::from_u64(0, 32));
                            crate::ir::IrDistItem {
                                range_lo: Some(ev.to_u64() as i64),
                                range_hi: Some(ev.to_u64() as i64),
                                weight_type: crate::ir::DistWeightType::Item,
                                weight: *w as i64,
                            }
                        }
                        crate::ast::DistItem::Value(e, crate::ast::DistWeight::Range(w)) => {
                            let ev = self
                                .evaluate_ast_expr(e)
                                .unwrap_or(LogicVec::from_u64(0, 32));
                            crate::ir::IrDistItem {
                                range_lo: Some(ev.to_u64() as i64),
                                range_hi: Some(ev.to_u64() as i64),
                                weight_type: crate::ir::DistWeightType::Range,
                                weight: *w as i64,
                            }
                        }
                        crate::ast::DistItem::Range(lo, hi, crate::ast::DistWeight::Item(w)) => {
                            let lo_v = self.evaluate_ast_expr(lo).ok().map(|v| v.to_u64() as i64);
                            let hi_v = self.evaluate_ast_expr(hi).ok().map(|v| v.to_u64() as i64);
                            crate::ir::IrDistItem {
                                range_lo: lo_v,
                                range_hi: hi_v,
                                weight_type: crate::ir::DistWeightType::Item,
                                weight: *w as i64,
                            }
                        }
                        crate::ast::DistItem::Range(lo, hi, crate::ast::DistWeight::Range(w)) => {
                            let lo_v = self.evaluate_ast_expr(lo).ok().map(|v| v.to_u64() as i64);
                            let hi_v = self.evaluate_ast_expr(hi).ok().map(|v| v.to_u64() as i64);
                            crate::ir::IrDistItem {
                                range_lo: lo_v,
                                range_hi: hi_v,
                                weight_type: crate::ir::DistWeightType::Range,
                                weight: *w as i64,
                            }
                        }
                    })
                    .collect::<Vec<_>>();
                Ok(self.evaluate_expr(&IrExpr::Dist {
                    expr: Box::new(IrExpr::Const(inner_val)),
                    items: ir_items,
                })?)
            }
            Expr::Cast { dtype, expr: inner } => {
                let val = self.evaluate_ast_expr(inner)?;
                let cast_width = match crate::elaboration::elaborator::parse_type_spec_str(dtype) {
                    Some(_) => {
                        // For AST path, compute width from type string
                        match dtype.as_str() {
                            "bit" | "logic" => 1,
                            "byte" => 8,
                            "shortint" => 16,
                            "int" | "integer" => 32,
                            "longint" | "time" => 64,
                            "real" | "realtime" => 64,
                            _ => val.width,
                        }
                    }
                    None => val.width,
                };
                Ok(val.resize(cast_width))
            }
            Expr::ScopedIdent { package, item } => Err(SimError::runtime(format!(
                "scoped identifier '{}.{}' not resolved at runtime",
                package, item
            ))),
        }
    }

    pub(crate) fn find_signal(&self, name: &str) -> Option<usize> {
        self.design
            .top
            .signals
            .iter()
            .position(|s| s.name == name)
            .or_else(|| self.design.hier_signal_map.get(name).copied())
    }

    fn build_hier_name(obj: &Expr, field: &str) -> String {
        match obj {
            Expr::Ident(prefix) => format!("{}.{}", prefix, field),
            Expr::MemberAccess {
                obj: inner,
                field: inner_field,
            } => {
                format!("{}.{}", Self::build_hier_name(inner, inner_field), field)
            }
            _ => String::new(),
        }
    }

    pub(crate) fn evaluate_ast_stmt(&mut self, stmt: &Stmt) -> Result<(), SimError> {
        match stmt {
            Stmt::Block { stmts } => {
                for s in stmts {
                    self.evaluate_ast_stmt(s)?;
                }
                Ok(())
            }
            Stmt::BlockingAssign { lhs, rhs, delay: _ } => {
                let val = self.evaluate_ast_expr(rhs)?;
                match lhs {
                    Expr::Ident(name) => self.write_local_or_field(name, val),
                    Expr::MemberAccess { obj, field } => {
                        let obj_val = self.evaluate_ast_expr(obj)?;
                        let obj_id = obj_val.to_u64() as ObjId;
                        if let Some(obj) = self.state.get_object_mut(obj_id) {
                            obj.fields.insert(field.clone(), val);
                            Ok(())
                        } else {
                            Err(SimError::runtime(format!(
                                "object {} not found for field write",
                                obj_id
                            )))
                        }
                    }
                    Expr::BitSelect { expr: inner, index } => {
                        let idx_val = self.evaluate_ast_expr(index)?;
                        let idx = idx_val.to_u64() as usize;
                        if let Some(elem_width) = self.get_field_elem_width(inner) {
                            let lhs_val = self.evaluate_ast_expr(inner)?;
                            let mut bits = lhs_val.bits.clone();
                            let start = idx * elem_width;
                            for (j, b) in val.bits.iter().enumerate() {
                                if start + j < bits.len() {
                                    bits[start + j] = *b;
                                }
                            }
                            let new_val = LogicVec {
                                width: bits.len(),
                                bits,
                            };
                            match inner.as_ref() {
                                Expr::Ident(name) => {
                                    self.write_local_or_field(name, new_val)?;
                                }
                                Expr::MemberAccess { obj, field } => {
                                    let ov = self.evaluate_ast_expr(obj)?;
                                    let oid = ov.to_u64() as ObjId;
                                    if let Some(o) = self.state.get_object_mut(oid) {
                                        o.fields.insert(field.clone(), new_val);
                                    }
                                }
                                _ => {}
                            }
                            Ok(())
                        } else {
                            let lhs_val = self.evaluate_ast_expr(inner)?;
                            let mut bits = lhs_val.bits.clone();
                            if idx < bits.len() {
                                let bit = val.bits.first().copied().unwrap_or(LogicVal::X);
                                bits[idx] = bit;
                            }
                            let width = bits.len();
                            let new_val = LogicVec { width, bits };
                            match inner.as_ref() {
                                Expr::Ident(name) => {
                                    self.write_local_or_field(name, new_val)?;
                                }
                                Expr::MemberAccess { obj, field } => {
                                    let ov = self.evaluate_ast_expr(obj)?;
                                    let oid = ov.to_u64() as ObjId;
                                    if let Some(o) = self.state.get_object_mut(oid) {
                                        o.fields.insert(field.clone(), new_val);
                                    }
                                }
                                _ => {}
                            }
                            Ok(())
                        }
                    }
                    Expr::RangeSelect {
                        expr: inner,
                        msb,
                        lsb,
                    } => {
                        let lhs_val = self.evaluate_ast_expr(inner)?;
                        let msb_val = self.evaluate_ast_expr(msb)?;
                        let lsb_val = self.evaluate_ast_expr(lsb)?;
                        let m = msb_val.to_u64() as usize;
                        let l = lsb_val.to_u64() as usize;
                        let (start, end) = if m > l { (l, m) } else { (m, l) };
                        let range_len = end - start + 1;
                        let mut bits = lhs_val.bits.clone();
                        for j in 0..val.width.min(range_len) {
                            if start + j < bits.len() {
                                bits[start + j] = val.bits[j];
                            }
                        }
                        let new_val = LogicVec {
                            width: bits.len(),
                            bits,
                        };
                        match inner.as_ref() {
                            Expr::Ident(name) => {
                                self.write_local_or_field(name, new_val)?;
                            }
                            Expr::MemberAccess { obj, field } => {
                                let ov = self.evaluate_ast_expr(obj)?;
                                let oid = ov.to_u64() as ObjId;
                                if let Some(o) = self.state.get_object_mut(oid) {
                                    o.fields.insert(field.clone(), new_val);
                                }
                            }
                            _ => {}
                        }
                        Ok(())
                    }
                    _ => Err(SimError::runtime(format!(
                        "unsupported LHS in method: {:?}",
                        lhs
                    ))),
                }
            }
            Stmt::NonBlockingAssign { lhs, rhs, delay: _ } => {
                let val = self.evaluate_ast_expr(rhs)?;
                match lhs {
                    Expr::Ident(name) => self.write_local_or_field(name, val),
                    Expr::MemberAccess { obj, field } => {
                        let obj_val = self.evaluate_ast_expr(obj)?;
                        let obj_id = obj_val.to_u64() as ObjId;
                        if let Some(obj) = self.state.get_object_mut(obj_id) {
                            obj.fields.insert(field.clone(), val);
                            Ok(())
                        } else {
                            Err(SimError::runtime(format!(
                                "object {} not found for field write",
                                obj_id
                            )))
                        }
                    }
                    Expr::BitSelect { expr: inner, index } => {
                        let idx_val = self.evaluate_ast_expr(index)?;
                        let idx = idx_val.to_u64() as usize;
                        if let Some(elem_width) = self.get_field_elem_width(inner) {
                            let lhs_val = self.evaluate_ast_expr(inner)?;
                            let mut bits = lhs_val.bits.clone();
                            let start = idx * elem_width;
                            for (j, b) in val.bits.iter().enumerate() {
                                if start + j < bits.len() {
                                    bits[start + j] = *b;
                                }
                            }
                            let new_val = LogicVec {
                                width: bits.len(),
                                bits: bits.clone(),
                            };
                            match inner.as_ref() {
                                Expr::Ident(name) => {
                                    self.write_local_or_field(name, new_val)?;
                                }
                                Expr::MemberAccess { obj, field } => {
                                    let ov = self.evaluate_ast_expr(obj)?;
                                    let oid = ov.to_u64() as ObjId;
                                    if let Some(o) = self.state.get_object_mut(oid) {
                                        o.fields.insert(field.clone(), new_val);
                                    }
                                }
                                _ => {}
                            }
                            Ok(())
                        } else {
                            let lhs_val = self.evaluate_ast_expr(inner)?;
                            let mut bits = lhs_val.bits.clone();
                            if idx < bits.len() {
                                let bit = val.bits.first().copied().unwrap_or(LogicVal::X);
                                bits[idx] = bit;
                            }
                            let width = bits.len();
                            let new_val = LogicVec { width, bits };
                            match inner.as_ref() {
                                Expr::Ident(name) => {
                                    self.write_local_or_field(name, new_val)?;
                                }
                                Expr::MemberAccess { obj, field } => {
                                    let ov = self.evaluate_ast_expr(obj)?;
                                    let oid = ov.to_u64() as ObjId;
                                    if let Some(o) = self.state.get_object_mut(oid) {
                                        o.fields.insert(field.clone(), new_val);
                                    }
                                }
                                _ => {}
                            }
                            Ok(())
                        }
                    }
                    Expr::RangeSelect {
                        expr: inner,
                        msb,
                        lsb,
                    } => {
                        let lhs_val = self.evaluate_ast_expr(inner)?;
                        let msb_val = self.evaluate_ast_expr(msb)?;
                        let lsb_val = self.evaluate_ast_expr(lsb)?;
                        let m = msb_val.to_u64() as usize;
                        let l = lsb_val.to_u64() as usize;
                        let (start, end) = if m > l { (l, m) } else { (m, l) };
                        let range_len = end - start + 1;
                        let mut bits = lhs_val.bits.clone();
                        for j in 0..val.width.min(range_len) {
                            if start + j < bits.len() {
                                bits[start + j] = val.bits[j];
                            }
                        }
                        let new_val = LogicVec {
                            width: bits.len(),
                            bits,
                        };
                        match inner.as_ref() {
                            Expr::Ident(name) => {
                                self.write_local_or_field(name, new_val)?;
                            }
                            Expr::MemberAccess { obj, field } => {
                                let ov = self.evaluate_ast_expr(obj)?;
                                let oid = ov.to_u64() as ObjId;
                                if let Some(o) = self.state.get_object_mut(oid) {
                                    o.fields.insert(field.clone(), new_val);
                                }
                            }
                            _ => {}
                        }
                        Ok(())
                    }
                    _ => Err(SimError::runtime(format!(
                        "unsupported LHS in method: {:?}",
                        lhs
                    ))),
                }
            }
            Stmt::IfElse {
                cond,
                true_branch,
                false_branch,
            } => {
                let cval = self.evaluate_ast_expr(cond)?;
                if cval.to_bool().unwrap_or(false) {
                    self.evaluate_ast_stmt(true_branch)
                } else if let Some(f) = false_branch {
                    self.evaluate_ast_stmt(f)
                } else {
                    Ok(())
                }
            }
            Stmt::Case {
                expr,
                items,
                default,
            } => {
                let case_val = self.evaluate_ast_expr(expr)?;
                let mut matched = false;
                for item in items {
                    for pat in &item.labels {
                        let pat_val = self.evaluate_ast_expr(pat)?;
                        if case_val.eq(&pat_val) {
                            self.evaluate_ast_stmt(&item.stmt)?;
                            matched = true;
                            break;
                        }
                    }
                    if matched {
                        break;
                    }
                }
                if !matched {
                    if let Some(default_body) = default {
                        self.evaluate_ast_stmt(default_body)?;
                    }
                }
                Ok(())
            }
            Stmt::CaseX {
                expr,
                items,
                default,
            } => {
                let case_val = self.evaluate_ast_expr(expr)?;
                let mut matched = false;
                for item in items {
                    for pat in &item.labels {
                        let pat_val = self.evaluate_ast_expr(pat)?;
                        if case_val.casex_eq(&pat_val) {
                            self.evaluate_ast_stmt(&item.stmt)?;
                            matched = true;
                            break;
                        }
                    }
                    if matched {
                        break;
                    }
                }
                if !matched {
                    if let Some(default_body) = default {
                        self.evaluate_ast_stmt(default_body)?;
                    }
                }
                Ok(())
            }
            Stmt::CaseZ {
                expr,
                items,
                default,
            } => {
                let case_val = self.evaluate_ast_expr(expr)?;
                let mut matched = false;
                for item in items {
                    for pat in &item.labels {
                        let pat_val = self.evaluate_ast_expr(pat)?;
                        if case_val.casez_eq(&pat_val) {
                            self.evaluate_ast_stmt(&item.stmt)?;
                            matched = true;
                            break;
                        }
                    }
                    if matched {
                        break;
                    }
                }
                if !matched {
                    if let Some(default_body) = default {
                        self.evaluate_ast_stmt(default_body)?;
                    }
                }
                Ok(())
            }
            Stmt::StmtCase {
                expr,
                items,
                default,
            } => self.evaluate_ast_stmt(&Stmt::Case {
                expr: expr.clone(),
                items: items.clone(),
                default: default.clone(),
            }),
            Stmt::LoopFor {
                init,
                cond,
                step,
                stmts,
            } => {
                if let Some(init_stmt) = init {
                    self.evaluate_ast_stmt(init_stmt)?;
                }
                while self.disable_pending.is_none()
                    && cond.as_ref().map_or(true, |c| {
                        self.evaluate_ast_expr(c)
                            .ok()
                            .map(|v| v.to_bool().unwrap_or(false))
                            .unwrap_or(false)
                    })
                {
                    for s in stmts {
                        self.evaluate_ast_stmt(s)?;
                        if self.disable_pending.is_some() {
                            break;
                        }
                    }
                    if self.disable_pending.is_some() {
                        break;
                    }
                    if let Some(step_stmt) = step {
                        self.evaluate_ast_stmt(step_stmt)?;
                    }
                }
                Ok(())
            }
            Stmt::LoopWhile { cond, stmts } => {
                while self.disable_pending.is_none()
                    && self
                        .evaluate_ast_expr(cond)
                        .ok()
                        .map(|v| v.to_bool().unwrap_or(false))
                        .unwrap_or(false)
                {
                    for s in stmts {
                        self.evaluate_ast_stmt(s)?;
                        if self.disable_pending.is_some() {
                            break;
                        }
                    }
                }
                Ok(())
            }
            Stmt::LoopForever { stmts } => {
                for _ in 0..1_000_000 {
                    if self.disable_pending.is_some() {
                        break;
                    }
                    for s in stmts {
                        self.evaluate_ast_stmt(s)?;
                        if self.disable_pending.is_some() {
                            break;
                        }
                    }
                }
                Ok(())
            }
            Stmt::Repeat { count, stmts } => {
                let count_val = self.evaluate_ast_expr(count)?;
                let n = count_val.to_u64();
                for _ in 0..n {
                    for s in stmts {
                        self.evaluate_ast_stmt(s)?;
                    }
                }
                Ok(())
            }
            Stmt::Expr { expr } => {
                self.evaluate_ast_expr(expr)?;
                Ok(())
            }
            Stmt::SysCall { name: _, args: _ } => Ok(()),
            Stmt::SysFinish => {
                self.running = false;
                Ok(())
            }
            Stmt::Delay { delay: _, stmt } => {
                // In immediate method context, execute delay body immediately
                self.evaluate_ast_stmt(stmt)
            }
            Stmt::Null => Ok(()),
            Stmt::Disable { name } => {
                self.disable_pending = Some(name.clone());
                Ok(())
            }
            Stmt::ForeachLoop {
                array_var,
                index_vars,
                stmts,
            } => {
                let count = self.get_foreach_count(array_var);
                let iv = index_vars
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "i".to_string());
                for i in 0..count {
                    let idx_val = LogicVec::from_u64(i as u64, 32);
                    let mut scope = HashMap::new();
                    scope.insert(iv.clone(), idx_val);
                    let depth = self.method_locals.len();
                    self.method_locals.push(scope);
                    for s in stmts {
                        self.evaluate_ast_stmt(s)?;
                    }
                    self.method_locals.truncate(depth);
                }
                Ok(())
            }
            Stmt::Return(Some(expr)) => {
                let val = self.evaluate_ast_expr(expr)?;
                if let Some(ref method) = self.current_method.clone() {
                    self.set_local(method, val);
                }
                Ok(())
            }
            Stmt::Return(None) => Ok(()),
            Stmt::StmtAssign { lhs, rhs } => {
                let val = self.evaluate_ast_expr(rhs)?;
                match lhs {
                    Expr::Ident(name) => self.write_local_or_field(name, val),
                    Expr::MemberAccess { obj, field } => {
                        let obj_val = self.evaluate_ast_expr(obj)?;
                        let obj_id = obj_val.to_u64() as ObjId;
                        if let Some(obj) = self.state.get_object_mut(obj_id) {
                            obj.fields.insert(field.clone(), val);
                            Ok(())
                        } else {
                            Err(SimError::runtime(format!(
                                "object {} not found for field write",
                                obj_id
                            )))
                        }
                    }
                    _ => Err(SimError::runtime(format!(
                        "unsupported LHS in StmtAssign: {:?}",
                        lhs
                    ))),
                }
            }
            _ => Err(SimError::runtime(format!(
                "unsupported statement in method context: {:?}",
                stmt
            ))),
        }
    }

}
