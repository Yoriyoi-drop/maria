use super::super::SimulationEngine;
use crate::error::SimError;
use crate::ir::*;
use crate::simulator::util::*;
use crate::Symbol;
use std::collections::HashMap;

impl SimulationEngine {
    pub(crate) fn write_lvalue(&mut self, lvalue: &IrLValue, mut val: LogicVec) -> Result<(), SimError> {
        // Check for const violation
        if let Some(id) = self.signal_id_from_lvalue(lvalue) {
            if let Some(sig) = self.design.top.signals.get(id) {
                if sig.is_const {
                    return Err(self.diag_error(crate::diagnostics::DiagCode::DpiError, format!(
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
                let is_init = self.state.read_signal(*sig_id).all_x() || self.state.read_signal(*sig_id).all_z();
                self.state.write_signal(*sig_id, existing);
                if !is_init {
                    self.signal_last_change.insert(*sig_id, self.state.time);
                }
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


    pub(crate) fn get_lvalue_width(&self, lvalue: &IrLValue) -> usize {
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
            if let Some(v) = scope.get::<str>(name) {
                return Some(v.clone());
            }
        }
        None
    }

    pub(crate) fn set_local(&mut self, name: &str, val: LogicVec) {
        if let Some(scope) = self.method_locals.last_mut() {
            scope.insert(Symbol::intern(name), val);
        }
    }

    pub(crate) fn write_ast_lvalue(&mut self, lhs: &crate::ast::Expr, val: LogicVec) -> Result<(), SimError> {
        match lhs {
            crate::ast::Expr::Ident(name) => self.write_local_or_field(name.as_str(), val),
            crate::ast::Expr::MemberAccess { obj, field } => {
                let obj_val = self.evaluate_ast_expr(obj)?;
                let obj_id = obj_val.to_u64() as ObjId;
                if let Some(obj_data) = self.state.get_object_mut(obj_id) {
                    obj_data.fields.insert(field.clone(), val);
                    Ok(())
                } else {
                    Err(self.diag_error(crate::diagnostics::DiagCode::NullHandle, format!(
                        "object {} not found for field '{}'",
                        obj_id, field
                    )))
                }
            }
            _ => Err(self.diag_error(crate::diagnostics::DiagCode::NotImplemented, format!(
                "unsupported lvalue type in task method: {:?}",
                lhs
            ))),
        }
    }

    pub(crate) fn ast_lvalue_to_ir(&self, lhs: &crate::ast::Expr) -> Option<IrLValue> {
        match lhs {
            crate::ast::Expr::Ident(name) => {
                let sig_id = self.find_signal(name.as_str())?;
                Some(IrLValue::Signal(sig_id, 0))
            }
            _ => None,
        }
    }

    pub(crate) fn find_ast_signal_id(&self, expr: &crate::ast::Expr) -> Option<SignalId> {
        match expr {
            crate::ast::Expr::Ident(name) => self.find_signal(name.as_str()),
            _ => None,
        }
    }

    pub(crate) fn write_local_or_field(&mut self, name: &str, val: LogicVec) -> Result<(), SimError> {
        if self.get_local(name).is_some() {
            self.set_local(name, val);
            return Ok(());
        }
        if let Some(obj_id) = self.current_this {
            if let Some(obj) = self.state.get_object_mut(obj_id) {
                obj.fields.insert(Symbol::intern(name), val);
                return Ok(());
            }
        }
        Err(self.diag_error(crate::diagnostics::DiagCode::NullHandle, format!(
            "cannot resolve '{}' in method context (not a local or field)",
            name
        )))
    }

}
