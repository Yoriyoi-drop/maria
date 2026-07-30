use std::collections::HashMap;
use super::Elaborator;
use crate::ast::*;
use crate::diagnostics::diagnostic::DiagCode;
use crate::error::SimError;
use crate::intern::Symbol;
use crate::ir::*;

impl Elaborator {
    pub(crate) fn flatten_instances(
        &mut self,
        top: &mut IrModule,
    ) -> Result<HashMap<Symbol, SignalId>, SimError> {
        let mut hier_signal_map: HashMap<Symbol, SignalId> = HashMap::new();
        let instances = std::mem::take(&mut top.sub_instances);
        for inst in &instances {
            let ast_module_clone: Module = if let Some(m) = self
                .design
                .modules
                .iter()
                .find(|m| m.name == inst.module_name)
            {
                m.clone()
            } else if let Some(iface) = self
                .design
                .interfaces
                .iter()
                .find(|i| i.name == inst.module_name)
            {
                Module {
                    name: iface.name.clone(),
                    ports: vec![],
                    params: vec![],
                    decls: iface.decls.clone(),
                    items: vec![],
                }
            } else {
                return Err(self.elab_diag(DiagCode::ModuleNotFound, format!(
                    "module or interface '{}' not found for instance '{}'",
                    inst.module_name, inst.instance_name
                )));
            };

            let needs_custom_params =
                !ast_module_clone.params.is_empty() && !inst.param_map.is_empty();
            let needs_type_params = !inst.type_param_map.is_empty();
            let mut child = if needs_custom_params || needs_type_params {
                let known_mods: Vec<Symbol> =
                    self.design.modules.iter().map(|m| m.name).collect();
                let param_vals = self.resolve_param_values(&ast_module_clone, &inst.param_map)?;
                self.elaborate_module_with_params_and_type(
                    &ast_module_clone,
                    &known_mods,
                    &param_vals,
                    &inst.type_param_map,
                )?
            } else {
                // Use pre-elaborated module (default params)
                self.modules
                    .get(&inst.module_name)
                    .ok_or_else(|| {
                        self.elab_diag(DiagCode::ModuleNotFound, format!("module '{}' not found", inst.module_name))
                    })?
                    .clone()
            };

            // Recursively flatten child's own instances
            let child_hier_map = self.flatten_instances(&mut child)?;
            hier_signal_map.extend(child_hier_map);

            // Build signal remapping: child_signal_id -> parent_signal_id
            let mut sig_remap: Vec<Option<SignalId>> = vec![None; child.signals.len()];
            let mut next_parent_id = top.signals.len();

            // Map port connections
            for (port_name, &parent_sig) in inst.port_map.iter() {
                if let Some(child_sig) = child.signals.iter().position(|s| s.name == *port_name) {
                    let child_width = child.signals[child_sig].width;
                    let parent_width = top.signals[parent_sig].elem_width;
                    if child_width != parent_width {
                        return Err(self.elab_diag(DiagCode::ParamMismatch, format!(
                            "port width mismatch on instance '{}': port '{}' expects width {}, connected signal '{}' has width {}",
                            inst.instance_name, port_name, child_width,
                            top.signals[parent_sig].name, parent_width
                        )));
                    }
                    // Port type checking: inout must connect to tri
                    if child.signals[child_sig].kind == SignalKind::Inout
                        && top.signals[parent_sig].net_type != NetType::Tri
                    {
                        return Err(self.elab_diag(DiagCode::ParamMismatch, format!(
                            "port type mismatch on instance '{}': inout port '{}' must connect to a tri signal, but '{}' has net type {:?}",
                            inst.instance_name, port_name,
                            top.signals[parent_sig].name,
                            top.signals[parent_sig].net_type
                        )));
                    }
                    sig_remap[child_sig] = Some(parent_sig);
                    // Add hierarchical alias: inst_name.port_name -> parent signal ID
                    hier_signal_map
                            .insert(Symbol::intern(&format!("{}.{}", inst.instance_name, port_name)), parent_sig);
                }
            }

            // Allocate parent signal IDs for unmapped child signals (internal signals)
            for (i, sig) in child.signals.iter().enumerate() {
                if sig_remap[i].is_none() {
                    let new_id = next_parent_id;
                    next_parent_id += 1;
                    sig_remap[i] = Some(new_id);
                    top.signals.push(SignalInfo {
                        name: Symbol::intern(&format!("{}.{}", inst.instance_name, sig.name)),
                        width: sig.width,
                        kind: sig.kind.clone(),
                        net_type: sig.net_type,
                        multi_driver: sig.multi_driver,
                        init_val: sig.init_val.clone(),
                        array_depth: sig.array_depth,
                        elem_width: sig.elem_width,
                        array_dims: sig.array_dims.clone(),
                        class_name: sig.class_name.clone(),
                        is_string: sig.is_string,
                        is_mailbox: sig.is_mailbox,
                        is_semaphore: sig.is_semaphore,
                        is_real: sig.is_real,
                        is_2state: sig.is_2state,
                        is_dynamic: sig.is_dynamic,
                        is_queue: sig.is_queue,
                        is_associative: sig.is_associative,
                        is_signed: sig.is_signed,
                        is_const: sig.is_const,
                        msb: sig.msb,
                        lsb: sig.lsb,
                        struct_fields: sig.struct_fields.clone(),
                        packed_dims: sig.packed_dims.clone(),
                        delay_rise: sig.delay_rise,
                        delay_fall: sig.delay_fall,
                        iface_type: None,
                        iface_modport: None,
                    });
                    // Also add to hier_signal_map: internal signals already have the right name in flat list
                    hier_signal_map.insert(Symbol::intern(&format!("{}.{}", inst.instance_name, sig.name)), new_id);
                }
            }

            let map_sig = |child_id: SignalId| -> SignalId {
                sig_remap.get(child_id).and_then(|&v| v).unwrap_or(child_id)
            };

            for process in &child.processes {
                let translated = self.translate_process(process, &map_sig)?;
                top.processes.push(translated);
            }
        }
        Ok(hier_signal_map)
    }

    fn translate_process(
        &self,
        process: &Process,
        map_sig: &dyn Fn(SignalId) -> SignalId,
    ) -> Result<Process, SimError> {
        match process {
            Process::Combinational {
                name,
                sensitivity,
                body,
            } => {
                let new_sens = sensitivity.iter().map(|s| map_sig(*s)).collect();
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::Combinational {
                    name: name.clone(),
                    sensitivity: new_sens,
                    body: new_body,
                })
            }
            Process::CombReactive {
                name,
                sensitivity,
                body,
            } => {
                let new_sens = sensitivity.iter().map(|s| map_sig(*s)).collect();
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::CombReactive {
                    name: name.clone(),
                    sensitivity: new_sens,
                    body: new_body,
                })
            }
            Process::Sequential {
                name,
                clock,
                reset,
                body,
            } => {
                let new_clock = match clock {
                    ClockEdge::PosEdge(id) => ClockEdge::PosEdge(map_sig(*id)),
                    ClockEdge::NegEdge(id) => ClockEdge::NegEdge(map_sig(*id)),
                };
                let new_reset = reset.as_ref().map(|r| ResetInfo {
                    signal: map_sig(r.signal),
                    polarity: r.polarity,
                    r#async: r.r#async,
                    value: r.value.clone(),
                });
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::Sequential {
                    name: name.clone(),
                    clock: new_clock,
                    reset: new_reset,
                    body: new_body,
                })
            }
            Process::Initial { name, body } => {
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::Initial {
                    name: name.clone(),
                    body: new_body,
                })
            }
            Process::AlwaysWithDelay { name, delay, body } => {
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::AlwaysWithDelay {
                    name: name.clone(),
                    delay: *delay,
                    body: new_body,
                })
            }
            Process::Final { name, body } => {
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::Final {
                    name: name.clone(),
                    body: new_body,
                })
            }
        }
    }

    fn translate_stmts(
        &self,
        stmts: &[IrStmt],
        map_sig: &dyn Fn(SignalId) -> SignalId,
    ) -> Result<Vec<IrStmt>, SimError> {
        stmts
            .iter()
            .map(|s| self.translate_stmt(s, map_sig))
            .collect()
    }

    fn translate_stmt(
        &self,
        stmt: &IrStmt,
        map_sig: &dyn Fn(SignalId) -> SignalId,
    ) -> Result<IrStmt, SimError> {
        match stmt {
            IrStmt::Block { stmts } => {
                let new = self.translate_stmts(stmts, map_sig)?;
                Ok(IrStmt::Block { stmts: new })
            }
            IrStmt::NamedBlock { name, stmts, decls } => {
                let new = self.translate_stmts(stmts, map_sig)?;
                Ok(IrStmt::NamedBlock {
                    name: name.clone(),
                    stmts: new,
                    decls: decls.clone(),
                })
            }
            IrStmt::BlockingAssign { lhs, rhs, delay } => {
                let new_lhs = self.translate_lvalue(lhs, map_sig);
                let new_rhs = self.translate_expr(rhs, map_sig);
                Ok(IrStmt::BlockingAssign {
                    lhs: new_lhs,
                    rhs: new_rhs,
                    delay: *delay,
                })
            }
            IrStmt::NonBlockingAssign { lhs, rhs, delay } => {
                let new_lhs = self.translate_lvalue(lhs, map_sig);
                let new_rhs = self.translate_expr(rhs, map_sig);
                Ok(IrStmt::NonBlockingAssign {
                    lhs: new_lhs,
                    rhs: new_rhs,
                    delay: *delay,
                })
            }
            IrStmt::If {
                cond,
                true_branch,
                false_branch,
            } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_true = self.translate_stmts(true_branch, map_sig)?;
                let new_false = self.translate_stmts(false_branch, map_sig)?;
                Ok(IrStmt::If {
                    cond: new_cond,
                    true_branch: new_true,
                    false_branch: new_false,
                })
            }
            IrStmt::Case {
                case_type,
                expr,
                items,
                default,
            } => {
                let new_expr = self.translate_expr(expr, map_sig);
                let new_items = items
                    .iter()
                    .map(|item| {
                        let labels = item
                            .labels
                            .iter()
                            .map(|l| self.translate_expr(l, map_sig))
                            .collect();
                        let body = self.translate_stmts(&item.body, map_sig)?;
                        Ok(IrCaseItem { labels, body })
                    })
                    .collect::<Result<Vec<_>, SimError>>()?;
                let new_default = self.translate_stmts(default, map_sig)?;
                Ok(IrStmt::Case {
                    case_type: case_type.clone(),
                    expr: new_expr,
                    items: new_items,
                    default: new_default,
                })
            }
            IrStmt::LoopFor {
                init,
                cond,
                step,
                body,
            } => {
                let new_init = init
                    .as_ref()
                    .map(|i| Box::new(self.translate_stmt(i, map_sig).unwrap_or(IrStmt::Null)));
                let new_cond = self.translate_expr(cond, map_sig);
                let new_step = step
                    .as_ref()
                    .map(|s| Box::new(self.translate_stmt(s, map_sig).unwrap_or(IrStmt::Null)));
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::LoopFor {
                    init: new_init,
                    cond: new_cond,
                    step: new_step,
                    body: new_body,
                })
            }
            IrStmt::LoopWhile { cond, body } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::LoopWhile {
                    cond: new_cond,
                    body: new_body,
                })
            }
            IrStmt::LoopDoWhile { cond, body } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::LoopDoWhile {
                    cond: new_cond,
                    body: new_body,
                })
            }
            IrStmt::Repeat { count, body } => {
                let new_count = self.translate_expr(count, map_sig);
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::Repeat {
                    count: new_count,
                    body: new_body,
                })
            }
            IrStmt::Foreach {
                array_var,
                index_var,
                body,
            } => {
                let new_var = self.translate_expr(array_var, map_sig);
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::Foreach {
                    array_var: new_var,
                    index_var: index_var.clone(),
                    body: new_body,
                })
            }
            IrStmt::Delay { delay, body } => {
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::Delay {
                    delay: *delay,
                    body: new_body,
                })
            }
            IrStmt::Wait { cond, body } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::Wait {
                    cond: new_cond,
                    body: new_body,
                })
            }
            IrStmt::SysCall { name, args } => {
                let new_args = args
                    .iter()
                    .map(|a| self.translate_expr(a, map_sig))
                    .collect();
                Ok(IrStmt::SysCall {
                    name: name.clone(),
                    args: new_args,
                })
            }
            IrStmt::EventControl { sig_id, edge, body } => {
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::EventControl {
                    sig_id: map_sig(*sig_id),
                    edge: edge.clone(),
                    body: new_body,
                })
            }
            IrStmt::EventTrigger { sig_id } => Ok(IrStmt::EventTrigger {
                sig_id: map_sig(*sig_id),
            }),
            IrStmt::SysFinish => Ok(IrStmt::SysFinish),
            IrStmt::Null => Ok(IrStmt::Null),
            IrStmt::MethodCallStmt {
                obj,
                method,
                args,
                with_clause,
            } => Ok(IrStmt::MethodCallStmt {
                obj: self.translate_expr(obj, map_sig),
                method: method.clone(),
                args: args
                    .iter()
                    .map(|a| self.translate_expr(a, map_sig))
                    .collect(),
                with_clause: with_clause
                    .as_ref()
                    .map(|wc| Box::new(self.translate_expr(wc, map_sig))),
            }),
            IrStmt::Break => Ok(IrStmt::Break),
            IrStmt::Continue => Ok(IrStmt::Continue),
            IrStmt::Disable { name } => Ok(IrStmt::Disable { name: name.clone() }),
            IrStmt::Force { lvalue, rhs } => Ok(IrStmt::Force {
                lvalue: self.translate_lvalue(lvalue, map_sig),
                rhs: self.translate_expr(rhs, map_sig),
            }),
            IrStmt::Release { lvalue } => Ok(IrStmt::Release {
                lvalue: self.translate_lvalue(lvalue, map_sig),
            }),
            IrStmt::Deassign { lvalue } => Ok(IrStmt::Deassign {
                lvalue: self.translate_lvalue(lvalue, map_sig),
            }),
            IrStmt::Fork {
                processes,
                join_type,
            } => {
                let new_proc = processes
                    .iter()
                    .map(|p| self.translate_stmts(p, map_sig))
                    .collect::<Result<Vec<_>, SimError>>()?;
                Ok(IrStmt::Fork {
                    processes: new_proc,
                    join_type: join_type.clone(),
                })
            }
            IrStmt::Assert {
                cond,
                pass_stmt,
                fail_stmt,
                clock_event,
                disable_iff,
                sequence: _,
            } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_pass = self.translate_stmts(pass_stmt, map_sig)?;
                let new_fail = self.translate_stmts(fail_stmt, map_sig)?;
                let new_disable = disable_iff
                    .as_ref()
                    .map(|e| Box::new(self.translate_expr(e, map_sig)));
                Ok(IrStmt::Assert {
                    cond: new_cond,
                    pass_stmt: new_pass,
                    fail_stmt: new_fail,
                    clock_event: clock_event.clone(),
                    disable_iff: new_disable,
                    sequence: None,
                })
            }
            IrStmt::Assume {
                cond,
                pass_stmt,
                fail_stmt,
                clock_event,
                disable_iff,
                sequence: _,
            } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_pass = self.translate_stmts(pass_stmt, map_sig)?;
                let new_fail = self.translate_stmts(fail_stmt, map_sig)?;
                let new_disable = disable_iff
                    .as_ref()
                    .map(|e| Box::new(self.translate_expr(e, map_sig)));
                Ok(IrStmt::Assume {
                    cond: new_cond,
                    pass_stmt: new_pass,
                    fail_stmt: new_fail,
                    clock_event: clock_event.clone(),
                    disable_iff: new_disable,
                    sequence: None,
                })
            }
            IrStmt::Cover {
                cond,
                pass_stmt,
                clock_event,
                disable_iff,
                sequence: _,
            } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_pass = self.translate_stmts(pass_stmt, map_sig)?;
                let new_disable = disable_iff
                    .as_ref()
                    .map(|e| Box::new(self.translate_expr(e, map_sig)));
                Ok(IrStmt::Cover {
                    cond: new_cond,
                    pass_stmt: new_pass,
                    clock_event: clock_event.clone(),
                    disable_iff: new_disable,
                    sequence: None,
                })
            }
            IrStmt::WaitOrder {
                events,
                failure_stmts,
            } => {
                let new_events = events.iter().map(|id| map_sig(*id)).collect();
                let new_failure = self.translate_stmts(failure_stmts, map_sig)?;
                Ok(IrStmt::WaitOrder {
                    events: new_events,
                    failure_stmts: new_failure,
                })
            }
            IrStmt::RandCase { items } => {
                let new_items: Result<Vec<(IrExpr, Vec<IrStmt>)>, SimError> = items
                    .iter()
                    .map(|(weight_expr, body)| {
                        let new_weight = self.translate_expr(weight_expr, map_sig);
                        let new_body = self.translate_stmts(body, map_sig)?;
                        Ok((new_weight, new_body))
                    })
                    .collect();
                Ok(IrStmt::RandCase { items: new_items? })
            }
            IrStmt::RandSequence { productions } => {
                let new_prods: Result<Vec<(Symbol, Vec<(IrExpr, Vec<IrStmt>)>)>, SimError> =
                    productions
                        .iter()
                        .map(|(name, items)| {
                            let new_items: Vec<(IrExpr, Vec<IrStmt>)> = items
                                .iter()
                                .map(|(weight_expr, body)| {
                                    let new_weight = self.translate_expr(weight_expr, map_sig);
                                    let new_body =
                                        self.translate_stmts(body, map_sig).unwrap_or_default();
                                    (new_weight, new_body)
                                })
                                .collect();
                            Ok((*name, new_items))
                        })
                        .collect();
                Ok(IrStmt::RandSequence {
                    productions: new_prods?,
                })
            }
        }
    }

    fn translate_lvalue(&self, lv: &IrLValue, map_sig: &dyn Fn(SignalId) -> SignalId) -> IrLValue {
        match lv {
            IrLValue::Signal(id, w) => IrLValue::Signal(map_sig(*id), *w),
            IrLValue::RangeSelect(id, msb, lsb) => IrLValue::RangeSelect(map_sig(*id), *msb, *lsb),
            IrLValue::BitSelect(id, idx) => IrLValue::BitSelect(map_sig(*id), *idx),
            IrLValue::ArrayIndex {
                sig_id,
                index,
                elem_width,
            } => IrLValue::ArrayIndex {
                sig_id: map_sig(*sig_id),
                index: Box::new(self.translate_expr(index, map_sig)),
                elem_width: *elem_width,
            },
            IrLValue::ArrayRangeSelect {
                sig_id,
                index,
                elem_width,
                msb,
                lsb,
            } => IrLValue::ArrayRangeSelect {
                sig_id: map_sig(*sig_id),
                index: Box::new(self.translate_expr(index, map_sig)),
                elem_width: *elem_width,
                msb: *msb,
                lsb: *lsb,
            },
            IrLValue::ArrayBitSelect {
                sig_id,
                index,
                elem_width,
                bit,
            } => IrLValue::ArrayBitSelect {
                sig_id: map_sig(*sig_id),
                index: Box::new(self.translate_expr(index, map_sig)),
                elem_width: *elem_width,
                bit: *bit,
            },
            IrLValue::Concat(parts) => IrLValue::Concat(
                parts
                    .iter()
                    .map(|p| self.translate_lvalue(p, map_sig))
                    .collect(),
            ),
        }
    }

    fn translate_expr(&self, expr: &IrExpr, map_sig: &dyn Fn(SignalId) -> SignalId) -> IrExpr {
        match expr {
            IrExpr::Const(v) => IrExpr::Const(v.clone()),
            IrExpr::FillLit(val) => IrExpr::FillLit(*val),
            IrExpr::Signal(id, w) => IrExpr::Signal(map_sig(*id), *w),
            IrExpr::RangeSelect(id, msb, lsb) => IrExpr::RangeSelect(map_sig(*id), *msb, *lsb),
            IrExpr::BitSelect(id, idx) => IrExpr::BitSelect(map_sig(*id), *idx),
            IrExpr::ArrayIndex {
                sig_id,
                index,
                elem_width,
            } => IrExpr::ArrayIndex {
                sig_id: map_sig(*sig_id),
                index: Box::new(self.translate_expr(index, map_sig)),
                elem_width: *elem_width,
            },
            IrExpr::Concat(exprs) => IrExpr::Concat(
                exprs
                    .iter()
                    .map(|e| self.translate_expr(e, map_sig))
                    .collect(),
            ),
            IrExpr::Replicate(n, inner) => {
                IrExpr::Replicate(*n, Box::new(self.translate_expr(inner, map_sig)))
            }
            IrExpr::UnaryOp(op, inner) => {
                IrExpr::UnaryOp(op.clone(), Box::new(self.translate_expr(inner, map_sig)))
            }
            IrExpr::BinaryOp(op, l, r) => IrExpr::BinaryOp(
                op.clone(),
                Box::new(self.translate_expr(l, map_sig)),
                Box::new(self.translate_expr(r, map_sig)),
            ),
            IrExpr::Cond(c, t, f) => IrExpr::Cond(
                Box::new(self.translate_expr(c, map_sig)),
                Box::new(self.translate_expr(t, map_sig)),
                Box::new(self.translate_expr(f, map_sig)),
            ),
            IrExpr::Signed(inner) => IrExpr::Signed(Box::new(self.translate_expr(inner, map_sig))),
            IrExpr::String(s) => IrExpr::String(s.clone()),
            IrExpr::SysFunc { name, args } => IrExpr::SysFunc {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| self.translate_expr(a, map_sig))
                    .collect(),
            },
            IrExpr::NewCall { class_name, args } => IrExpr::NewCall {
                class_name: class_name.clone(),
                args: args
                    .iter()
                    .map(|a| self.translate_expr(a, map_sig))
                    .collect(),
            },
            IrExpr::This => IrExpr::This,
            IrExpr::MethodCall {
                obj,
                method,
                args,
                with_clause,
            } => IrExpr::MethodCall {
                obj: Box::new(self.translate_expr(obj, map_sig)),
                method: method.clone(),
                args: args
                    .iter()
                    .map(|a| self.translate_expr(a, map_sig))
                    .collect(),
                with_clause: with_clause
                    .as_ref()
                    .map(|wc| Box::new(self.translate_expr(wc, map_sig))),
            },
            IrExpr::MemberAccess { obj, field } => IrExpr::MemberAccess {
                obj: Box::new(self.translate_expr(obj, map_sig)),
                field: field.clone(),
            },
            IrExpr::ExprRangeSelect(inner, msb, lsb) => {
                IrExpr::ExprRangeSelect(Box::new(self.translate_expr(inner, map_sig)), *msb, *lsb)
            }
            IrExpr::ExprBitSelect(inner, idx) => {
                IrExpr::ExprBitSelect(Box::new(self.translate_expr(inner, map_sig)), *idx)
            }
            IrExpr::ExprPartSelect(inner, base_expr, width_expr) => IrExpr::ExprPartSelect(
                Box::new(self.translate_expr(inner, map_sig)),
                Box::new(self.translate_expr(base_expr, map_sig)),
                Box::new(self.translate_expr(width_expr, map_sig)),
            ),
            IrExpr::DpiCall {
                name,
                args,
                return_width,
            } => IrExpr::DpiCall {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| self.translate_expr(a, map_sig))
                    .collect(),
                return_width: *return_width,
            },
            IrExpr::HierRef(name) => IrExpr::HierRef(name.clone()),
            IrExpr::Inside { expr, list } => IrExpr::Inside {
                expr: Box::new(self.translate_expr(expr, map_sig)),
                list: list
                    .iter()
                    .map(|e| self.translate_expr(e, map_sig))
                    .collect(),
            },
            IrExpr::Cast { width, expr } => IrExpr::Cast {
                width: *width,
                expr: Box::new(self.translate_expr(expr, map_sig)),
            },
            IrExpr::StreamingConcat {
                op,
                slice_size,
                slices,
            } => IrExpr::StreamingConcat {
                op: op.clone(),
                slice_size: *slice_size,
                slices: slices
                    .iter()
                    .map(|e| self.translate_expr(e, map_sig))
                    .collect(),
            },
            IrExpr::Dist { expr, items } => IrExpr::Dist {
                expr: Box::new(self.translate_expr(expr, map_sig)),
                items: items.clone(),
            },
            IrExpr::UdpLookup { udp_name, args } => IrExpr::UdpLookup {
                udp_name: udp_name.clone(),
                args: args
                    .iter()
                    .map(|a| self.translate_expr(a, map_sig))
                    .collect(),
            },
            IrExpr::VifBinding { instance_name } => IrExpr::VifBinding {
                instance_name: instance_name.clone(),
            },
            IrExpr::VirtualIfaceAccess {
                vif_name,
                field,
                field_width,
            } => IrExpr::VirtualIfaceAccess {
                vif_name: vif_name.clone(),
                field: field.clone(),
                field_width: *field_width,
            },
            IrExpr::FuncCall { func_name, args } => IrExpr::FuncCall {
                func_name: func_name.clone(),
                args: args
                    .iter()
                    .map(|a| self.translate_expr(a, map_sig))
                    .collect(),
            },
        }
    }
}
