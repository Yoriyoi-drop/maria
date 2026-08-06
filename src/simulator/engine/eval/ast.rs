use super::super::SimulationEngine;
use crate::error::SimError;
use crate::diagnostics::DiagCode;
use crate::ir::*;
use crate::ast::*;
use crate::Symbol;
use crate::simulator::types::*;
use crate::simulator::util::*;
use crate::simulator::value::*;
use std::collections::HashMap;

impl SimulationEngine {
    pub(crate) fn evaluate_ast_expr(&mut self, expr: &Expr) -> Result<LogicVec, SimError> {
        match expr {
            Expr::Value(v) => match v {
                Value::Decimal(i) => Ok(LogicVec::from_u64(*i as u64, 32)),
                Value::Binary { bits, .. } => {
                    LogicVec::from_bin(bits).map_err(|e| SimError::with_diag(DiagCode::DpiError, e))
                }
                Value::Hex { bits, .. } => {
                    LogicVec::from_hex(bits).map_err(|e| SimError::with_diag(DiagCode::DpiError, e))
                }
                Value::Octal { bits, .. } => {
                    LogicVec::from_hex(bits).map_err(|e| SimError::with_diag(DiagCode::DpiError, e))
                }
                Value::Real(r) => Ok(LogicVec::from_u64(r.to_bits(), 64)),
            },
            Expr::Ident { name, line, col } => {
                if name == "this" {
                    if let Some(obj_id) = self.current_this {
                        return Ok(LogicVec::from_u64(obj_id as u64, 64));
                    } else {
                        return Err(self.diag_error_at(DiagCode::NullHandle, "'this' used outside of class method", *line, *col));
                    }
                }
                if let Some(local) = self.get_local(name.as_str()) {
                    return Ok(local);
                }
                if let Some(obj_id) = self.current_this {
                    if let Some(obj) = self.state.get_object(obj_id) {
                        if let Some(val) = obj.fields.get(name) {
                            return Ok(val.clone());
                        }
                    }
                }
                if let Some(sig_id) = self.find_signal(name.as_str()) {
                    return Ok(self.state.read_signal(sig_id).clone());
                }
                let ctx = self
                    .current_this
                    .map(|id| format!("obj_id={}", id))
                    .unwrap_or_else(|| "no current_this".to_string());
                // Identifier method-context yang tak bisa di-resolve (mis. field
                // class yang belum di-assign / null handle pada kode UVM) di-
                // perlakukan sebagai warning + null default agar simulasi tetap
                // berjalan, bukan mematikan seluruh run.
                self.diag_warn_at(
                    DiagCode::NullHandle,
                    format!("cannot resolve identifier '{}' in method context ({}); using null default", name, ctx),
                    *line,
                    *col,
                );
                Ok(LogicVec::from_u64(0, 64))
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
            Expr::FuncCall { name, args, .. } if name == "new" => {
                let _arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                let obj_id = self.state.alloc_object(Symbol::intern(""));
                Ok(LogicVec::from_u64(obj_id as u64, 64))
            }
            Expr::FuncCall { name, args, .. } if name.ends_with("::new") => {
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
                let obj_id = self.state.alloc_object(Symbol::intern(&effective));
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
            Expr::FuncCall { name, args, .. } if name == "uvm_config_db::set" => {
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
            Expr::FuncCall { name, args, .. } if name == "uvm_config_db::get" => {
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
                let stored = self.uvm_config_db_data.get(&key).cloned();
                if let Some(val) = stored {
                    if let Some(last_arg) = args.get(3) {
                        match last_arg {
                            Expr::Ident { name: var, .. } => {
                                self.write_local_or_field(var.as_str(), val.clone())?;
                            }
                            Expr::MemberAccess { obj, field } => {
                                let obj_val = self.evaluate_ast_expr(obj)?;
                                let obj_id = obj_val.to_u64() as ObjId;
                                if let Some(obj) = self.state.get_object_mut(obj_id) {
                                    obj.fields.insert(*field, val.clone());
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
            Expr::FuncCall { name, args, .. } if name == "uvm_resource_db::set" => {
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
            Expr::FuncCall { name, args, .. } if name == "uvm_resource_db::get" => {
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
                let stored = self.uvm_resource_db_data.get(&key).cloned();
                if let Some(val) = stored {
                    if let Some(last_arg) = args.get(2) {
                        match last_arg {
                            Expr::Ident { name: var, .. } => {
                                self.write_local_or_field(var.as_str(), val.clone())?;
                            }
                            Expr::MemberAccess { obj, field } => {
                                let obj_val = self.evaluate_ast_expr(obj)?;
                                let obj_id = obj_val.to_u64() as ObjId;
                                if let Some(obj) = self.state.get_object_mut(obj_id) {
                                    obj.fields.insert(*field, val.clone());
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
            Expr::FuncCall { name, args, .. } if name == "uvm_factory::set_type_override_by_type" => {
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
                self.factory_type_overrides.insert(orig, override_type);
                Ok(LogicVec::from_u64(1, 1))
            }
            Expr::FuncCall { name, args, .. } => {
                let arg_vals: Vec<LogicVec> = args
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
                // Pemanggilan method class tanpa `this.` prefix — dispatch ke
                // object saat ini bila method ada di hierarki class-nya.
                if let Some(obj_id) = self.current_this {
                    if let Some(obj) = self.state.get_object(obj_id) {
                        if !obj.class_name.is_empty() {
                            if self
                                .find_method_in_hierarchy(obj.class_name.as_str(), name.as_str())
                                .is_ok()
                            {
                                return self.execute_method(obj_id, name.as_str(), &arg_vals);
                            }
                        }
                    }
                }
                self.diag_warn_at(
                    DiagCode::NotImplemented,
                    format!("unknown function '{}' in method context; using null default", name),
                    0,
                    0,
                );
                Ok(LogicVec::from_u64(0, 64))
            }
            Expr::MethodCall {
                obj,
                method,
                args,
                with_clause: _,
            } => {
                if let Expr::Ident { name: s, .. } = obj.as_ref() {
                    if s == "super" {
                        let arg_vals: Vec<LogicVec> = args
                            .iter()
                            .map(|a| self.evaluate_ast_expr(a))
                            .collect::<Result<_, _>>()?;
                        return self.execute_super_method(method.as_str(), &arg_vals);
                    }
                }
                let obj_val = self.evaluate_ast_expr(obj)?;
                let obj_id = obj_val.to_u64() as ObjId;
                let arg_vals: Vec<LogicVec> = args
                    .iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                self.execute_method(obj_id, method.as_str(), &arg_vals)
            }
            Expr::MemberAccess { obj, field } => {
                // Try hierarchical signal reference first
                let hier_name = Self::build_hier_name(obj, field.as_str());
                if let Some(sig_id) = self.find_signal(&hier_name) {
                    return Ok(self.state.read_signal(sig_id).clone());
                }
                // Fall back to object field access (class objects)
                let obj_val = self.evaluate_ast_expr(obj)?;
                let obj_id = obj_val.to_u64() as ObjId;
                let obj_data = self
                    .state
                    .get_object(obj_id)
                    .ok_or_else(|| SimError::with_diag(DiagCode::NullHandle, format!("object {} not found", obj_id)))?;
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
                let bits = val.bits[start..=end].to_vec();
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
                    let bits = val.bits[b..b + w].to_vec();
                    Ok(LogicVec { width: w, bits })
                } else if w == 0 {
                    Ok(LogicVec::from_u64(0, 1))
                } else {
                    Err(SimError::with_diag(DiagCode::MemoryOutOfBounds, "part-select out of range"))
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
                        return Err(SimError::with_diag(DiagCode::MemoryOutOfBounds, "streaming slice size must be > 0"));
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
                let cast_width = match crate::elaboration::util::parse_type_spec_str(dtype.as_str()) {
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
            Expr::CastWidth { width, expr: inner } => {
                let val = self.evaluate_ast_expr(inner)?;
                let w = self.evaluate_ast_expr(width)?.to_u64().max(1) as usize;
                Ok(val.resize(w))
            }
            Expr::ScopedIdent {
                package,
                item,
                line,
                col,
            } => {
                let qname = Symbol::intern(&format!("{}::{}", package.as_str(), item.as_str()));
                if let Some(&val) = self.design.pkg_scoped_consts.get(&qname) {
                    return Ok(LogicVec::from_u64(val as u64, 32));
                }
                self.diag_warn_at(
                    DiagCode::DpiError,
                    format!("scoped identifier '{}.{}' not resolved at runtime; using null default", package, item),
                    *line,
                    *col,
                );
                Ok(LogicVec::from_u64(0, 32))
            }
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
            Expr::Ident { name: prefix, .. } => format!("{}.{}", prefix, field),
            Expr::MemberAccess {
                obj: inner,
                field: inner_field,
            } => {
                format!("{}.{}", Self::build_hier_name(inner, inner_field.as_str()), field)
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
                    Expr::Ident { name, .. } => self.write_local_or_field(name.as_str(), val),
                    Expr::MemberAccess { obj, field } => {
                        let obj_val = self.evaluate_ast_expr(obj)?;
                        let obj_id = obj_val.to_u64() as ObjId;
                        if let Some(obj) = self.state.get_object_mut(obj_id) {
                            obj.fields.insert(*field, val);
                            Ok(())
                        } else {
                            Err(SimError::with_diag(
                                DiagCode::NullHandle,
                                format!("object {} not found for field write", obj_id),
                            ))
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
                                Expr::Ident { name, .. } => {
                                    self.write_local_or_field(name.as_str(), new_val)?;
                                }
                                Expr::MemberAccess { obj, field } => {
                                    let ov = self.evaluate_ast_expr(obj)?;
                                    let oid = ov.to_u64() as ObjId;
                                    if let Some(o) = self.state.get_object_mut(oid) {
                                        o.fields.insert(*field, new_val);
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
                                Expr::Ident { name, .. } => {
                                    self.write_local_or_field(name.as_str(), new_val)?;
                                }
                                Expr::MemberAccess { obj, field } => {
                                    let ov = self.evaluate_ast_expr(obj)?;
                                    let oid = ov.to_u64() as ObjId;
                                    if let Some(o) = self.state.get_object_mut(oid) {
                                        o.fields.insert(*field, new_val);
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
                            Expr::Ident { name, .. } => {
                                self.write_local_or_field(name.as_str(), new_val)?;
                            }
                            Expr::MemberAccess { obj, field } => {
                                let ov = self.evaluate_ast_expr(obj)?;
                                let oid = ov.to_u64() as ObjId;
                                if let Some(o) = self.state.get_object_mut(oid) {
                                    o.fields.insert(*field, new_val);
                                }
                            }
                            _ => {}
                        }
                        Ok(())
                    }
                    _ => Err(SimError::with_diag(
                        DiagCode::NotImplemented,
                        format!("unsupported LHS in method: {:?}", lhs),
                    )),
                }
            }
            Stmt::NonBlockingAssign { lhs, rhs, delay: _ } => {
                let val = self.evaluate_ast_expr(rhs)?;
                match lhs {
                    Expr::Ident { name, .. } => self.write_local_or_field(name.as_str(), val),
                    Expr::MemberAccess { obj, field } => {
                        let obj_val = self.evaluate_ast_expr(obj)?;
                        let obj_id = obj_val.to_u64() as ObjId;
                        if let Some(obj) = self.state.get_object_mut(obj_id) {
                            obj.fields.insert(*field, val);
                            Ok(())
                        } else {
                            Err(SimError::with_diag(
                                DiagCode::NullHandle,
                                format!("object {} not found for field write", obj_id),
                            ))
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
                                Expr::Ident { name, .. } => {
                                    self.write_local_or_field(name.as_str(), new_val)?;
                                }
                                Expr::MemberAccess { obj, field } => {
                                    let ov = self.evaluate_ast_expr(obj)?;
                                    let oid = ov.to_u64() as ObjId;
                                    if let Some(o) = self.state.get_object_mut(oid) {
                                        o.fields.insert(*field, new_val);
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
                                Expr::Ident { name, .. } => {
                                    self.write_local_or_field(name.as_str(), new_val)?;
                                }
                                Expr::MemberAccess { obj, field } => {
                                    let ov = self.evaluate_ast_expr(obj)?;
                                    let oid = ov.to_u64() as ObjId;
                                    if let Some(o) = self.state.get_object_mut(oid) {
                                        o.fields.insert(*field, new_val);
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
                            Expr::Ident { name, .. } => {
                                self.write_local_or_field(name.as_str(), new_val)?;
                            }
                            Expr::MemberAccess { obj, field } => {
                                let ov = self.evaluate_ast_expr(obj)?;
                                let oid = ov.to_u64() as ObjId;
                                if let Some(o) = self.state.get_object_mut(oid) {
                                    o.fields.insert(*field, new_val);
                                }
                            }
                            _ => {}
                        }
                        Ok(())
                    }
                    _ => Err(SimError::with_diag(
                        DiagCode::NotImplemented,
                        format!("unsupported LHS in method: {:?}", lhs),
                    )),
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
                    && cond.as_ref().is_none_or(|c| {
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
                self.disable_pending = Some(*name);
                Ok(())
            }
            Stmt::ForeachLoop {
                array_var,
                index_vars,
                stmts,
            } => {
                let count = self.get_foreach_count(array_var.as_str());
                let iv = index_vars
                    .first()
                    .cloned()
                    .unwrap_or_else(|| Symbol::intern("i"));
                for i in 0..count {
                    let idx_val = LogicVec::from_u64(i as u64, 32);
                    let mut scope = HashMap::new();
                    scope.insert(iv, idx_val);
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
                if let Some(ref method) = self.current_method {
                    self.set_local(method.as_str(), val);
                }
                Ok(())
            }
            Stmt::Return(None) => Ok(()),
            Stmt::StmtAssign { lhs, rhs } => {
                let val = self.evaluate_ast_expr(rhs)?;
                match lhs {
                    Expr::Ident { name, .. } => self.write_local_or_field(name.as_str(), val),
                    Expr::MemberAccess { obj, field } => {
                        let obj_val = self.evaluate_ast_expr(obj)?;
                        let obj_id = obj_val.to_u64() as ObjId;
                        if let Some(obj) = self.state.get_object_mut(obj_id) {
                            obj.fields.insert(*field, val);
                            Ok(())
                        } else {
                            Err(SimError::with_diag(
                                DiagCode::NullHandle,
                                format!("object {} not found for field write", obj_id),
                            ))
                        }
                    }
                    _ => Err(SimError::with_diag(
                        DiagCode::NotImplemented,
                        format!("unsupported LHS in StmtAssign: {:?}", lhs),
                    )),
                }
            }
            _ => Err(SimError::with_diag(
                DiagCode::NotImplemented,
                format!("unsupported statement in method context: {:?}", stmt),
            )),
        }
    }

}
