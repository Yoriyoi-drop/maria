use super::super::SimulationEngine;
use crate::error::SimError;
use crate::diagnostics::DiagCode;
use crate::ir::*;
use crate::ast::*;
use crate::Symbol;
use std::collections::HashMap;

impl SimulationEngine {
    pub(crate) fn execute_method_body(
        &mut self,
        obj_id: Option<ObjId>,
        method_def: &IrClassMethod,
        args: &[LogicVec],
        method: &str,
    ) -> Result<LogicVec, SimError> {
        let old_this = self.current_this;
        if let Some(oid) = obj_id {
            self.current_this = Some(oid);
        }

        let mut local_signals: HashMap<Symbol, LogicVec> = HashMap::new();
        for (i, port) in method_def.ports.iter().enumerate() {
            let port_width = port.resolved_width(&HashMap::new()).unwrap_or(1);
            let val = if i < args.len() {
                // F18: jangan resize port 1-bit — `string name` / handle class
                // (`uvm_component parent`) di-resolve width 1, resize(1)
                // menghancurkan string & obj id (super.new(name, parent)
                // kehilangan argumen). Vector eksplisit ([7:0]) tetap resize.
                if port_width > 1 {
                    args[i].resize(port_width)
                } else {
                    args[i].clone()
                }
            } else {
                LogicVec::new(port_width)
            };
            local_signals.insert(port.name, val);
        }

        for decl in &method_def.decls {
            for dv in &decl.names {
                let w = dv.resolved_width(&HashMap::new()).unwrap_or(1);
                local_signals.insert(dv.name, LogicVec::new(w));
            }
        }

        let depth = self.method_locals.len();
        self.method_locals.push(local_signals);

        self.ast_loop_iters = 0;

        let old_method = self.current_method;
        self.current_method = Some(Symbol::intern(method));

        if !method_def.stmts.is_empty() {
            if method_def.is_task {
                // F26: fork id aktif (dari branch fork IR/AST) diteruskan ke
                // evaluasi task body — continuation resume (ContinueAstBlock)
                // membawa fork_id ini → event.rs fork_decrement saat selesai.
                // Sebelumnya selalu None → task di dalam `fork ... join`
                // (module initial) suspend dgn fork_id None → resume tak
                // pernah decrement → join selesai premature. `task_suspended`
                // memberitahu fork arm utk TIDAK decrement di titik ini
                // (resume yang decrement — cegah double-decrement).
                let completed = self
                    .evaluate_ast_block_with_delay_fork(&method_def.stmts, self.active_fork_id)?;
                if std::env::var("DBG_UVM").is_ok() {
                    eprintln!("[DBG-F26] task '{}' fid={:?} completed={} nstmts={}", method, self.active_fork_id, completed, method_def.stmts.len());
                }
                if !completed {
                    // Task suspended by delay — keep scope & context alive for
                    // continuation. F26 fix review: flag `task_suspended` HANYA
                    // di-set saat task berjalan di dalam branch fork
                    // (active_fork_id terisi) — di luar fork flag tidak relevan
                    // dan tidak boleh mengotori fork_branch_end berikutnya.
                    if self.active_fork_id.is_some() {
                        self.task_suspended = true;
                    }
                    self.current_method = old_method;
                    return Ok(LogicVec::new(0));
                }
            } else {
                let body = Stmt::Block {
                    stmts: method_def.stmts.clone(),
                };
                self.evaluate_ast_stmt(&body)?;
            }
        }

        let return_val = if method_def.is_task {
            LogicVec::new(0) // tasks return void
        } else {
            self.get_local(method).unwrap_or_else(|| LogicVec::new(1))
        };

        self.current_method = old_method;
        self.method_locals.truncate(depth);
        self.current_this = old_this;
        Ok(return_val)
    }

    pub(crate) fn get_foreach_count(&self, array_var: &str) -> usize {
        if let Some(obj_id) = self.current_this {
            if let Some(obj) = self.state.get_object(obj_id) {
                if let Some(cls) = self.design.classes.get(&obj.class_name) {
                    for field in &cls.fields {
                        if field.name == array_var {
                            return field.array_depth;
                        }
                    }
                }
            }
        }
        1
    }

    pub(crate) fn get_field_elem_width(&self, expr: &Expr) -> Option<usize> {
        match expr {
            Expr::Ident { name, .. } => {
                if let Some(obj_id) = self.current_this {
                    if let Some(obj) = self.state.get_object(obj_id) {
                        if let Some(cls) = self.design.classes.get(&obj.class_name) {
                            for field in &cls.fields {
                                if field.name == *name && field.array_depth > 1 {
                                    return Some(field.elem_width);
                                }
                            }
                        }
                    }
                }
                None
            }
            Expr::MemberAccess { obj, field } => {
                if let Expr::Ident { name: s, .. } = obj.as_ref() {
                    if s == "this" {
                        if let Some(obj_id) = self.current_this {
                            if let Some(obj) = self.state.get_object(obj_id) {
                                if let Some(cls) = self.design.classes.get(&obj.class_name) {
                                    for f in &cls.fields {
                                        if f.name == *field && f.array_depth > 1 {
                                            return Some(f.elem_width);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub(crate) fn find_method_in_hierarchy(
        &self,
        class_name: &str,
        method: &str,
    ) -> Result<IrClassMethod, SimError> {
        let mut current = class_name;
        while let Some(cls) = self.design.classes.get::<str>(current) {
            if let Some(m) = cls.methods.iter().find(|m| m.name == method) {
                return Ok(m.clone());
            }
            if let Some(parent) = &cls.extends {
                current = parent.as_str();
            } else {
                break;
            }
        }
        Err(SimError::with_diag(
            DiagCode::DpiError,
            format!("method '{}' not found in class '{}' or its parents", method, class_name),
        ))
    }
}
impl SimulationEngine {
    pub(crate) fn check_with_clause(
        &mut self,
        with_clause: Option<&IrExpr>,
        elem: &LogicVec,
    ) -> Result<bool, SimError> {
        if let Some(wc) = with_clause {
            let depth = self.method_locals.len();
            let mut scope = std::collections::HashMap::new();
            scope.insert(Symbol::intern("item"), elem.clone());
            self.method_locals.push(scope);
            let result = self.evaluate_expr(wc)?.to_bool().unwrap_or(false);
            self.method_locals.truncate(depth);
            Ok(result)
        } else {
            Ok(true)
        }
    }

    pub(crate) fn evaluate_array_method(
        &mut self,
        sig_id: SignalId,
        sig: &SignalInfo,
        method: &str,
        args: &[IrExpr],
        with_clause: Option<&IrExpr>,
    ) -> Result<LogicVec, SimError> {
        // Check if this is an associative array method
        if sig.is_associative {
            // Evaluate args first to avoid borrow conflicts with assoc_data access
            let args_eval: Vec<LogicVec> = args
                .iter()
                .map(|a| self.evaluate_expr(a))
                .collect::<Result<Vec<_>, SimError>>()?;
            let assoc_map = self.assoc_data.entry(sig_id).or_default();
            match method {
                "num" => {
                    let n = assoc_map.len();
                    return Ok(LogicVec::from_u64(n as u64, 32));
                }
                "delete" => {
                    if args_eval.is_empty() {
                        assoc_map.clear();
                    } else {
                        assoc_map.remove(&args_eval[0]);
                    }
                    return Ok(LogicVec::new(0));
                }
                "exists" => {
                    let found = assoc_map.contains_key(&args_eval[0]);
                    return Ok(LogicVec::from_u64(if found { 1 } else { 0 }, 1));
                }
                "first" => {
                    if let Some(key) = assoc_map.keys().next() {
                        return Ok(key.clone());
                    }
                    return Ok(LogicVec::new(0));
                }
                "last" => {
                    if let Some(key) = assoc_map.keys().last() {
                        return Ok(key.clone());
                    }
                    return Ok(LogicVec::new(0));
                }
                "next" => {
                    if let Some(key) = args_eval.first() {
                        let mut found = false;
                        let mut next_val = LogicVec::new(0);
                        for k in assoc_map.keys() {
                            if found {
                                next_val = k.clone();
                                break;
                            }
                            if *k == *key {
                                found = true;
                            }
                        }
                        return Ok(next_val);
                    }
                    return Ok(LogicVec::new(0));
                }
                "prev" => {
                    if let Some(key) = args_eval.first() {
                        let mut prev_val = LogicVec::new(0);
                        for k in assoc_map.keys() {
                            if *k == *key {
                                return Ok(prev_val);
                            }
                            prev_val = k.clone();
                        }
                        return Ok(LogicVec::new(0));
                    }
                    return Ok(LogicVec::new(0));
                }
                _ => {
                    // Fall through to default array methods (like push_back, etc.)
                }
            }
        }
        match method {
            "size" => {
                let lv = self.state.read_signal(sig_id);
                let count = lv.width.checked_div(sig.elem_width).unwrap_or(0);
                Ok(LogicVec::from_u64(count as u64, 32))
            }
            "delete" => {
                if let Some(index_expr) = args.first() {
                    let idx_val = self.evaluate_expr(index_expr)?;
                    let idx = idx_val.to_u64() as usize;
                    let lv = self.state.read_signal(sig_id);
                    let elem_width = sig.elem_width;
                    let count = lv.width.checked_div(elem_width).unwrap_or(0);
                    if idx >= count {
                        return Err(SimError::with_diag(
                            DiagCode::MemoryOutOfBounds,
                            format!("delete index {} out of range (size {})", idx, count),
                        ));
                    }
                    let before = lv.bits[..idx * elem_width].to_vec();
                    let after = lv.bits[(idx + 1) * elem_width..].to_vec();
                    let mut remaining = Vec::with_capacity(before.len() + after.len());
                    remaining.extend(before);
                    remaining.extend(after);
                    let new_lv = LogicVec {
                        width: remaining.len(),
                        bits: remaining,
                    };
                    self.state.write_signal(sig_id, new_lv);
                    Ok(LogicVec::new(0))
                } else {
                    self.state.write_signal(sig_id, LogicVec::new(0));
                    Ok(LogicVec::new(0))
                }
            }
            "pop_front" => {
                let lv = self.state.read_signal(sig_id);
                let elem_width = sig.elem_width;
                if lv.width < elem_width {
                    return Err(SimError::with_diag(DiagCode::MemoryOutOfBounds, "pop_front on empty queue"));
                }
                let mut bits = Vec::with_capacity(elem_width);
                for i in 0..elem_width {
                    bits.push(lv.bits.get(i).copied().unwrap_or(LogicVal::X));
                }
                let result = LogicVec {
                    width: elem_width,
                    bits,
                };
                let remaining = LogicVec {
                    width: lv.width - elem_width,
                    bits: lv.bits[elem_width..].to_vec(),
                };
                self.state.write_signal(sig_id, remaining);
                Ok(result)
            }
            "pop_back" => {
                let lv = self.state.read_signal(sig_id);
                let elem_width = sig.elem_width;
                if lv.width < elem_width {
                    return Err(SimError::with_diag(DiagCode::MemoryOutOfBounds, "pop_back on empty queue"));
                }
                let start = lv.width - elem_width;
                let mut bits = Vec::with_capacity(elem_width);
                for i in start..lv.width {
                    bits.push(lv.bits.get(i).copied().unwrap_or(LogicVal::X));
                }
                let result = LogicVec {
                    width: elem_width,
                    bits,
                };
                let remaining = LogicVec {
                    width: lv.width - elem_width,
                    bits: lv.bits[..start].to_vec(),
                };
                self.state.write_signal(sig_id, remaining);
                Ok(result)
            }
            "push_front" => {
                let arg_val = if let Some(a) = args.first() {
                    self.evaluate_expr(a)?
                } else {
                    return Err(SimError::with_diag(DiagCode::DpiError, "push_front expects 1 argument"));
                };
                let elem_width = sig.elem_width;
                let padded = if arg_val.width >= elem_width {
                    let bits = arg_val.bits[..elem_width].to_vec();
                    LogicVec {
                        width: elem_width,
                        bits,
                    }
                } else {
                    let mut bits = arg_val.bits.clone();
                    bits.resize(elem_width, LogicVal::X);
                    LogicVec {
                        width: elem_width,
                        bits,
                    }
                };
                let mut existing = self.state.read_signal(sig_id).clone();
                let mut new_bits = Vec::with_capacity(existing.width + elem_width);
                new_bits.extend(padded.bits.iter().copied());
                new_bits.extend(existing.bits.iter().copied());
                existing.bits = new_bits;
                existing.width += elem_width;
                self.state.write_signal(sig_id, existing);
                Ok(LogicVec::new(0))
            }
            "exists" => {
                let index_expr = args
                    .first()
                    .ok_or_else(|| SimError::with_diag(DiagCode::DpiError, "exists expects 1 argument"))?;
                let idx_val = self.evaluate_expr(index_expr)?;
                let idx = idx_val.to_u64() as usize;
                let lv = self.state.read_signal(sig_id);
                let elem_width = sig.elem_width;
                let count = lv.width.checked_div(elem_width).unwrap_or(0);
                Ok(LogicVec::from_u64(if idx < count { 1 } else { 0 }, 1))
            }
            "push_back" => {
                let arg_val = if let Some(a) = args.first() {
                    self.evaluate_expr(a)?
                } else {
                    return Err(SimError::with_diag(DiagCode::DpiError, "push_back expects 1 argument"));
                };
                let elem_width = sig.elem_width;
                let padded = if arg_val.width >= elem_width {
                    let bits = arg_val.bits[..elem_width].to_vec();
                    LogicVec {
                        width: elem_width,
                        bits,
                    }
                } else {
                    let mut bits = arg_val.bits.clone();
                    bits.resize(elem_width, LogicVal::X);
                    LogicVec {
                        width: elem_width,
                        bits,
                    }
                };
                let mut existing = self.state.read_signal(sig_id).clone();
                existing.bits.extend(padded.bits.iter().copied());
                existing.width += elem_width;
                self.state.write_signal(sig_id, existing);
                Ok(LogicVec::new(0))
            }
            "insert" => {
                if args.len() < 2 {
                    return Err(SimError::with_diag(DiagCode::DpiError, "insert expects 2 arguments (index, value)"));
                }
                let idx_val = self.evaluate_expr(&args[0])?;
                let idx = idx_val.to_u64() as usize;
                let arg_val = self.evaluate_expr(&args[1])?;
                let elem_width = sig.elem_width;
                let padded = if arg_val.width >= elem_width {
                    let bits = arg_val.bits[..elem_width].to_vec();
                    LogicVec {
                        width: elem_width,
                        bits,
                    }
                } else {
                    let mut bits = arg_val.bits.clone();
                    bits.resize(elem_width, LogicVal::X);
                    LogicVec {
                        width: elem_width,
                        bits,
                    }
                };
                let mut existing = self.state.read_signal(sig_id).clone();
                let count = existing.width.checked_div(elem_width).unwrap_or(0);
                let pos = idx.min(count);
                let mut new_bits = Vec::with_capacity(existing.width + elem_width);
                new_bits.extend(existing.bits[..pos * elem_width].iter().copied());
                new_bits.extend(padded.bits.iter().copied());
                new_bits.extend(existing.bits[pos * elem_width..].iter().copied());
                existing.bits = new_bits;
                existing.width += elem_width;
                self.state.write_signal(sig_id, existing);
                Ok(LogicVec::new(0))
            }
            "reverse" => {
                let mut lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width.checked_div(elem_width).unwrap_or(0);
                    let mut new_bits = Vec::with_capacity(lv.width);
                    for i in (0..count).rev() {
                        for j in 0..elem_width {
                            new_bits.push(lv.bits[i * elem_width + j]);
                        }
                    }
                    lv.bits = new_bits;
                }
                self.state.write_signal(sig_id, lv);
                Ok(LogicVec::new(0))
            }
            "sort" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width.checked_div(elem_width).unwrap_or(0);
                    let mut elems: Vec<LogicVec> = (0..count)
                        .map(|i| {
                            let mut bits = Vec::with_capacity(elem_width);
                            for j in 0..elem_width {
                                bits.push(lv.bits[i * elem_width + j]);
                            }
                            LogicVec {
                                width: elem_width,
                                bits,
                            }
                        })
                        .collect();
                    elems.sort_by_key(|a| a.to_u64());
                    let mut new_bits = Vec::with_capacity(lv.width);
                    for e in &elems {
                        new_bits.extend(e.bits.iter().copied());
                    }
                    let sorted = LogicVec {
                        width: lv.width,
                        bits: new_bits,
                    };
                    self.state.write_signal(sig_id, sorted);
                }
                Ok(LogicVec::new(0))
            }
            "rsort" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width.checked_div(elem_width).unwrap_or(0);
                    let mut elems: Vec<LogicVec> = (0..count)
                        .map(|i| {
                            let mut bits = Vec::with_capacity(elem_width);
                            for j in 0..elem_width {
                                bits.push(lv.bits[i * elem_width + j]);
                            }
                            LogicVec {
                                width: elem_width,
                                bits,
                            }
                        })
                        .collect();
                    elems.sort_by_key(|a| std::cmp::Reverse(a.to_u64()));
                    let mut new_bits = Vec::with_capacity(lv.width);
                    for e in &elems {
                        new_bits.extend(e.bits.iter().copied());
                    }
                    let sorted = LogicVec {
                        width: lv.width,
                        bits: new_bits,
                    };
                    self.state.write_signal(sig_id, sorted);
                }
                Ok(LogicVec::new(0))
            }
            "shuffle" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width.checked_div(elem_width).unwrap_or(0);
                    let mut elems: Vec<LogicVec> = (0..count)
                        .map(|i| {
                            let mut bits = Vec::with_capacity(elem_width);
                            for j in 0..elem_width {
                                bits.push(lv.bits[i * elem_width + j]);
                            }
                            LogicVec {
                                width: elem_width,
                                bits,
                            }
                        })
                        .collect();
                    use rand::seq::SliceRandom;
                    elems.shuffle(&mut rand::thread_rng());
                    let mut new_bits = Vec::with_capacity(lv.width);
                    for e in &elems {
                        new_bits.extend(e.bits.iter().copied());
                    }
                    let shuffled = LogicVec {
                        width: lv.width,
                        bits: new_bits,
                    };
                    self.state.write_signal(sig_id, shuffled);
                }
                Ok(LogicVec::new(0))
            }
            // --- Reduction methods ---
            "sum" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width.checked_div(elem_width).unwrap_or(0);
                    let mut result: u64 = 0;
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits,
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        result = result.wrapping_add(elem.to_u64());
                    }
                    Ok(LogicVec::from_u64(result, elem_width.max(32)))
                } else {
                    Ok(LogicVec::new(0))
                }
            }
            "product" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width.checked_div(elem_width).unwrap_or(0);
                    let mut result: u64 = 1;
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits,
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        result = result.wrapping_mul(elem.to_u64());
                    }
                    Ok(LogicVec::from_u64(result, elem_width.max(32)))
                } else {
                    Ok(LogicVec::new(0))
                }
            }
            "and" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    let mut result = LogicVec::fill(LogicVal::One, elem_width);
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            let idx = i * elem_width + j;
                            bits.push(lv.bits.get(idx).copied().unwrap_or(LogicVal::X));
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits: bits.clone(),
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        for j in 0..elem_width {
                            if bits.get(j) == Some(&LogicVal::Zero) {
                                result.bits[j] = LogicVal::Zero;
                            }
                        }
                    }
                    Ok(result)
                } else {
                    Ok(LogicVec::fill(LogicVal::One, elem_width.max(1)))
                }
            }
            "or" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    let mut result = LogicVec::fill(LogicVal::Zero, elem_width);
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            let idx = i * elem_width + j;
                            bits.push(lv.bits.get(idx).copied().unwrap_or(LogicVal::X));
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits: bits.clone(),
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        for j in 0..elem_width {
                            if bits.get(j) == Some(&LogicVal::One) {
                                result.bits[j] = LogicVal::One;
                            }
                        }
                    }
                    Ok(result)
                } else {
                    Ok(LogicVec::fill(LogicVal::Zero, elem_width.max(1)))
                }
            }
            "xor" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    let mut result = LogicVec::fill(LogicVal::Zero, elem_width);
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            let idx = i * elem_width + j;
                            bits.push(lv.bits.get(idx).copied().unwrap_or(LogicVal::X));
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits: bits.clone(),
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        for j in 0..elem_width {
                            if bits.get(j) == Some(&LogicVal::One) {
                                result.bits[j] = match result.bits[j] {
                                    LogicVal::Zero => LogicVal::One,
                                    LogicVal::One => LogicVal::Zero,
                                    other => other,
                                };
                            }
                        }
                    }
                    Ok(result)
                } else {
                    Ok(LogicVec::fill(LogicVal::Zero, elem_width.max(1)))
                }
            }
            // --- Ordering methods ---
            "min" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    let mut min_val = u64::MAX;
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits,
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        let v = elem.to_u64();
                        if v < min_val {
                            min_val = v;
                        }
                    }
                    Ok(LogicVec::from_u64(min_val, elem_width))
                } else {
                    Ok(LogicVec::new(1))
                }
            }
            "max" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    let mut max_val: u64 = 0;
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits,
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        let v = elem.to_u64();
                        if v > max_val {
                            max_val = v;
                        }
                    }
                    Ok(LogicVec::from_u64(max_val, elem_width))
                } else {
                    Ok(LogicVec::new(1))
                }
            }
            "unique" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    let mut seen = std::collections::HashSet::new();
                    let mut new_bits = Vec::new();
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        let elem = LogicVec {
                            width: elem_width,
                            bits,
                        };
                        if !self.check_with_clause(with_clause, &elem)? {
                            continue;
                        }
                        if seen.insert(elem.to_u64()) {
                            for j in 0..elem_width {
                                let idx = i * elem_width + j;
                                new_bits.push(lv.bits.get(idx).copied().unwrap_or(LogicVal::X));
                            }
                        }
                    }
                    let result = LogicVec {
                        width: new_bits.len(),
                        bits: new_bits,
                    };
                    self.state.write_signal(sig_id, result);
                }
                Ok(LogicVec::new(0))
            }
            // --- Locator methods ---
            "find" | "find_first" | "find_last" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    if with_clause.is_some() {
                        // If with_clause is provided, iterate and find matching elements
                        let mut result_elems: Vec<LogicVec> = Vec::new();
                        if method == "find_last" {
                            for i in (0..count).rev() {
                                let mut bits = Vec::with_capacity(elem_width);
                                for j in 0..elem_width {
                                    bits.push(lv.bits[i * elem_width + j]);
                                }
                                let elem = LogicVec {
                                    width: elem_width,
                                    bits,
                                };
                                if self.check_with_clause(with_clause, &elem)? {
                                    result_elems.push(elem);
                                }
                            }
                        } else {
                            for i in 0..count {
                                let mut bits = Vec::with_capacity(elem_width);
                                for j in 0..elem_width {
                                    bits.push(lv.bits[i * elem_width + j]);
                                }
                                let elem = LogicVec {
                                    width: elem_width,
                                    bits,
                                };
                                if self.check_with_clause(with_clause, &elem)? {
                                    result_elems.push(elem);
                                    if method == "find_first" {
                                        break;
                                    }
                                }
                            }
                        }
                        let total_width = result_elems.len() * elem_width;
                        let mut all_bits = Vec::with_capacity(total_width);
                        for e in &result_elems {
                            all_bits.extend(e.bits.iter());
                        }
                        return Ok(LogicVec {
                            width: total_width,
                            bits: all_bits,
                        });
                    }
                    if method == "find_first" && count > 0 {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[j]);
                        }
                        return Ok(LogicVec {
                            width: elem_width,
                            bits,
                        });
                    }
                    if method == "find_last" && count > 0 {
                        let start = (count - 1) * elem_width;
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[start + j]);
                        }
                        return Ok(LogicVec {
                            width: elem_width,
                            bits,
                        });
                    }
                    // "find" returns all elements (same as array)
                    return Ok(lv);
                }
                Ok(LogicVec::new(0))
            }
            "find_index" | "find_first_index" | "find_last_index" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 && lv.width >= elem_width {
                    let count = lv.width / elem_width;
                    if with_clause.is_some() {
                        let mut indices: Vec<u64> = Vec::new();
                        if method == "find_last_index" {
                            for i in (0..count).rev() {
                                let mut bits = Vec::with_capacity(elem_width);
                                for j in 0..elem_width {
                                    bits.push(lv.bits[i * elem_width + j]);
                                }
                                let elem = LogicVec {
                                    width: elem_width,
                                    bits,
                                };
                                if self.check_with_clause(with_clause, &elem)? {
                                    indices.push(i as u64);
                                }
                            }
                        } else {
                            for i in 0..count {
                                let mut bits = Vec::with_capacity(elem_width);
                                for j in 0..elem_width {
                                    bits.push(lv.bits[i * elem_width + j]);
                                }
                                let elem = LogicVec {
                                    width: elem_width,
                                    bits,
                                };
                                if self.check_with_clause(with_clause, &elem)? {
                                    indices.push(i as u64);
                                    if method == "find_first_index" {
                                        break;
                                    }
                                }
                            }
                        }
                        let mut bits = Vec::new();
                        for idx in &indices {
                            let idx_vec = LogicVec::from_u64(*idx, 32);
                            bits.extend(idx_vec.bits.iter());
                        }
                        return Ok(LogicVec {
                            width: bits.len(),
                            bits,
                        });
                    }
                    // Return indices as 32-bit values packed into result
                    if method == "find_first_index" && count > 0 {
                        return Ok(LogicVec::from_u64(0, 32));
                    }
                    if method == "find_last_index" && count > 0 {
                        return Ok(LogicVec::from_u64((count - 1) as u64, 32));
                    }
                    // "find_index" returns all indices (0..count) as a packed queue
                    let mut bits = Vec::new();
                    for i in 0..count {
                        let idx_vec = LogicVec::from_u64(i as u64, 32);
                        bits.extend(idx_vec.bits.iter());
                    }
                    return Ok(LogicVec {
                        width: bits.len(),
                        bits,
                    });
                }
                Ok(LogicVec::new(0))
            }
            _ => Err(SimError::with_diag(
                DiagCode::NotImplemented,
                format!("unknown array/queue method: {}", method),
            )),
        }
    }

}
