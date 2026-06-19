use std::collections::HashMap;

use crate::ast::*;
use crate::ast::types::const_eval_simple;
use crate::ast::types::const_eval_with_params;
use crate::ir::*;

fn get_array_info(signals: &[SignalInfo], sig_id: SignalId) -> (usize, usize) {
    signals.get(sig_id).map(|s| (s.array_depth, s.elem_width)).unwrap_or((1, 0))
}

pub struct Elaborator {
    pub design: Design,
    pub modules: HashMap<String, IrModule>,
    pub param_vals: HashMap<String, i64>,
}

impl Elaborator {
    pub fn new(design: Design) -> Self {
        Elaborator {
            design,
            modules: HashMap::new(),
            param_vals: HashMap::new(),
        }
    }

    pub fn elaborate(&mut self, top_module: Option<&str>) -> Result<IrDesign, String> {
        // Inline function calls in all modules
        for module in &mut self.design.modules {
            let temps = crate::ast::inline::inline_func_calls_in_module(module)?;
            for (name, width) in temps {
                module.decls.push(Decl {
                    dtype: DataType::Logic,
                    kind: DeclKind::Reg,
                    names: vec![DeclVar {
                        name,
                        range: None,
                        expr_range: if width > 1 {
                            Some(ExprRange {
                                msb: Expr::Value(crate::ast::expr::Value::Decimal((width - 1) as i64)),
                                lsb: Expr::Value(crate::ast::expr::Value::Decimal(0)),
                            })
                        } else { None },
                        array_range: None,
                    }],
                });
            }
        }

        // Expand generates in all modules (with resolved params)
        // Use index-based iteration to avoid borrow conflicts
        for i in 0..self.design.modules.len() {
            let param_vals = resolve_param_values_fn(&self.design.modules[i], &HashMap::new())?;
            if let Some(module) = self.design.modules.get_mut(i) {
                expand_all_generates(module, &param_vals)?;
            }
        }

        // First pass: elaborate all modules
        let module_names: Vec<String> = self.design.modules.iter().map(|m| m.name.clone()).collect();

        let modules_snapshot: Vec<Module> = self.design.modules.clone();
        for module in &modules_snapshot {
            let ir = self.elaborate_module(module, &module_names)?;
            self.modules.insert(module.name.clone(), ir);
        }

        // Find top module
        let top_name = match top_module {
            Some(name) => name.to_string(),
            None => {
                self.design.modules.first()
                    .map(|m| m.name.clone())
                    .ok_or_else(|| "no modules in design".to_string())?
            }
        };

        let mut top = self.modules.remove(&top_name)
            .ok_or_else(|| format!("top module '{}' not found", top_name))?;

        // Flatten instances: merge child module processes into the top module
        self.flatten_instances(&mut top)?;

        let classes = self.elaborate_classes()?;

        Ok(IrDesign {
            top,
            modules: self.modules.clone(),
            classes,
        })
    }

    fn resolve_param_values(&self, module: &Module, instance_overrides: &HashMap<String, i64>) -> Result<HashMap<String, i64>, String> {
        resolve_param_values_fn(module, instance_overrides)
    }
}

fn resolve_param_values_fn(module: &Module, instance_overrides: &HashMap<String, i64>) -> Result<HashMap<String, i64>, String> {
    let mut vals = HashMap::new();
    // First, collect positional overrides indexed by position in module's param list
    let mut positional_overrides: Vec<i64> = Vec::new();
    for (name, val) in instance_overrides {
        if name.starts_with("__param") {
            let idx: usize = name.trim_start_matches("__param").parse().unwrap_or(0);
            if idx >= positional_overrides.len() {
                positional_overrides.resize(idx + 1, 0);
            }
            positional_overrides[idx] = *val;
        }
    }

    for (i, param) in module.params.iter().enumerate() {
        let default_val = match &param.default {
            Some(e) => const_eval_simple(e).unwrap_or(0),
            None => 0,
        };
        let val = if i < positional_overrides.len() {
            positional_overrides[i]
        } else if let Some(override_val) = instance_overrides.get(&param.name) {
            *override_val
        } else {
            default_val
        };
        vals.insert(param.name.clone(), val);
    }
    Ok(vals)
}

impl Elaborator {
    fn elaborate_module(&mut self, module: &Module, known_modules: &[String]) -> Result<IrModule, String> {
        let param_vals = self.resolve_param_values(module, &HashMap::new())?;
        self.elaborate_module_with_params(module, known_modules, &param_vals)
    }

    fn elaborate_module_with_params(&mut self, module: &Module, known_modules: &[String],
                                    param_vals: &HashMap<String, i64>) -> Result<IrModule, String> {
        self.param_vals = param_vals.clone();
        let mut signals = Vec::new();
        let mut signal_map: HashMap<String, SignalId> = HashMap::new();
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        let mut inouts = Vec::new();
        let mut processes = Vec::new();
        let mut sub_instances = Vec::new();
        let mut next_id = 0usize;

        // Helper to get or create signal
        let get_or_create_signal = |name: &str,
                                         width: usize,
                                         kind: SignalKind,
                                         signals: &mut Vec<SignalInfo>,
                                         signal_map: &mut HashMap<String, SignalId>,
                                         id: &mut SignalId,
                                         array_depth: usize,
                                         elem_width: usize|
         -> SignalId {
            if let Some(&sid) = signal_map.get(name) {
                sid
            } else {
                let sid = *id;
                *id += 1;
                signal_map.insert(name.to_string(), sid);
                signals.push(SignalInfo {
                    name: name.to_string(),
                    width,
                    kind,
                    init_val: LogicVec::new(width),
                    array_depth,
                    elem_width,
                    class_name: None,
                });
                sid
            }
        };

        // Process ports with parameter-aware width resolution
        for port in &module.ports {
            let width = port.resolved_width(param_vals)?;
            let kind = match port.direction {
                PortDirection::Input => SignalKind::Input,
                PortDirection::Output => SignalKind::Output,
                PortDirection::Inout => SignalKind::Inout,
            };
            let sid = get_or_create_signal(&port.name, width, kind.clone(), &mut signals, &mut signal_map, &mut next_id, 1, width);
            match port.direction {
                PortDirection::Input => inputs.push(sid),
                PortDirection::Output => outputs.push(sid),
                PortDirection::Inout => inouts.push(sid),
            }
        }

        // Process declarations with parameter-aware width resolution
        for decl in &module.decls {
            let class_name = match &decl.dtype {
                DataType::UserDefined(cn) => Some(cn.clone()),
                _ => None,
            };
            for var in &decl.names {
                let elem_width = decl.dtype.width().max(
                    var.resolved_width(param_vals)?
                ).max(
                    decl.kind.default_width()
                );
                let kind = match decl.kind {
                    DeclKind::Wire => SignalKind::Wire,
                    DeclKind::Reg | DeclKind::Logic | DeclKind::Int | DeclKind::Integer => SignalKind::Reg,
                };
                if let Some(ar) = &var.array_range {
                    let depth = if ar.msb >= ar.lsb { ar.msb - ar.lsb + 1 } else { ar.lsb - ar.msb + 1 };
                    let total_width = elem_width * depth;
                    let _sid = get_or_create_signal(&var.name, total_width, kind, &mut signals, &mut signal_map, &mut next_id, depth, elem_width);
                    if let Some(sig) = signals.iter_mut().find(|s| s.name == var.name) {
                        let elem_init = LogicVec::new(elem_width);
                        let mut full_init = LogicVec::new(total_width);
                        for i in 0..depth {
                            for j in 0..elem_width {
                                full_init.bits[i * elem_width + j] = elem_init.bits[j].clone();
                            }
                        }
                        sig.init_val = full_init;
                        if class_name.is_some() {
                            sig.class_name = class_name.clone();
                        }
                    }
                } else {
                    let _sid = get_or_create_signal(&var.name, elem_width, kind, &mut signals, &mut signal_map, &mut next_id, 1, elem_width);
                    if let Some(class) = &class_name {
                        if let Some(sig) = signals.iter_mut().find(|s| s.name == var.name) {
                            sig.class_name = Some(class.clone());
                        }
                    }
                }
            }
        }

        // Expand generate blocks in module items
        let expanded_items: Vec<ModuleItem> = {
            let mut items = Vec::new();
            for item in &module.items {
                match item {
                    ModuleItem::Generate(gen) => {
                        let expanded = expand_generate_block(gen, param_vals)?;
                        items.extend(expanded);
                    }
                    other => items.push(other.clone()),
                }
            }
            items
        };

        // Process module items
        for item in &expanded_items {
            match item {
                ModuleItem::Always(always) => {
                    let process = self.elaborate_always(&always, &signal_map, &signals)?;
                    processes.push(process);
                }
                ModuleItem::Initial(initial) => {
                    let body = self.elaborate_stmt_block(&initial.stmts, &signal_map, &known_modules, &signals)?;
                    processes.push(Process::Initial {
                        name: format!("initial_{}", processes.len()),
                        body,
                    });
                }
                ModuleItem::Assign(assign) => {
                    // Convert to a combinational process
                    let lhs = self.elaborate_lvalue(&assign.lhs, &signal_map, &signals)?;
                    let rhs = self.elaborate_expr(&assign.rhs, &signal_map, &signals)?;
                    let stmts = vec![IrStmt::BlockingAssign { lhs, rhs, delay: None }];
                    let sensitivity = collect_sensitivity(&assign.rhs, &signal_map);
                    processes.push(Process::Combinational {
                        name: format!("assign_{}", processes.len()),
                        sensitivity,
                        body: stmts,
                    });
                }
                ModuleItem::Instance(inst) => {
                    let mut port_map = HashMap::new();
                    for conn in &inst.port_conns {
                        match conn {
                            PortConnection::Positional(_) => {
                                // Positional requires knowing module port order
                            }
                            PortConnection::Named { port, expr } => {
                                let sig_id = self.elaborate_expr_to_signal(expr, &signal_map)?;
                                port_map.insert(port.clone(), sig_id);
                            }
                        }
                    }
                    // Resolve parameter overrides to integer values
                    let mut param_map = HashMap::new();
                    for (pname, pexpr) in &inst.param_assigns {
                        let val = const_eval_with_params(pexpr, param_vals).unwrap_or(0);
                        param_map.insert(pname.clone(), val);
                    }
                    sub_instances.push(IrInstance {
                        module_name: inst.module_name.clone(),
                        instance_name: inst.instance_name.clone(),
                        port_map,
                        param_map,
                    });
                }
                ModuleItem::Gate(gate) => {
                    let mut sig_ids = Vec::new();
                    for port in &gate.ports {
                        let sid = match port {
                            Expr::Ident(name) => signal_map.get(name).copied()
                                .ok_or_else(|| format!("signal '{}' not found for gate", name))?,
                            _ => return Err(format!("gate port must be a simple signal")),
                        };
                        sig_ids.push(sid);
                    }
                    if sig_ids.len() < 2 {
                        return Err(format!("gate requires at least 2 ports"));
                    }
                    let (out_ids, in_ids) = match gate.gate_type {
                        GateType::And | GateType::Or | GateType::Nand | GateType::Nor | GateType::Xor | GateType::Xnor => {
                            (vec![sig_ids[0]], sig_ids[1..].to_vec())
                        }
                        GateType::Buf | GateType::Not => {
                            let in_id = sig_ids[sig_ids.len() - 1];
                            let outs = sig_ids[..sig_ids.len() - 1].to_vec();
                            (outs, vec![in_id])
                        }
                    };
                    let in_exprs: Vec<IrExpr> = in_ids.iter().map(|id| IrExpr::Signal(*id, 0)).collect();
                    let gate_expr = build_gate_expr(&gate.gate_type, &in_exprs);
                    for &out_id in &out_ids {
                        let process = Process::Combinational {
                            name: format!("gate_{}", out_id),
                            sensitivity: in_ids.clone(),
                            body: vec![IrStmt::BlockingAssign {
                                lhs: IrLValue::Signal(out_id, 0),
                                rhs: gate_expr.clone(),
                                delay: None,
                            }],
                        };
                        processes.push(process);
                    }
                }
                _ => {}
            }
        }

        Ok(IrModule {
            name: module.name.clone(),
            signals,
            inputs,
            outputs,
            inouts,
            processes,
            sub_instances,
        })
    }

    fn elaborate_classes(&self) -> Result<HashMap<String, IrClassDef>, String> {
        let mut classes = HashMap::new();
        for cd in &self.design.classes {
            let mut fields = Vec::new();
            for member in &cd.members {
                if let ClassMember::Decl(decl) = member {
                    for dv in &decl.names {
                        let decl_width = decl.dtype.width();
                        let var_width = dv.resolved_width(&HashMap::new()).unwrap_or(1);
                        let elem_width = decl_width.max(var_width).max(1);
                        let (array_depth, actual_elem_width) = if let Some(ar) = &dv.array_range {
                            let depth = if ar.msb >= ar.lsb { ar.msb - ar.lsb + 1 } else { ar.lsb - ar.msb + 1 };
                            (depth, elem_width)
                        } else {
                            (1, elem_width)
                        };
                        let total_width = array_depth * actual_elem_width;
                        fields.push(IrClassField {
                            name: dv.name.clone(),
                            width: total_width,
                            array_depth,
                            elem_width: actual_elem_width,
                        });
                    }
                }
            }
            let methods = cd.members.iter().filter_map(|m| {
                match m {
                    ClassMember::Function(fd) => Some(IrClassMethod {
                        name: fd.name.clone(),
                        virtual_flag: fd.virtual_flag,
                        ports: fd.ports.clone(),
                        decls: fd.decls.clone(),
                        stmts: fd.stmts.clone(),
                    }),
                    ClassMember::Task(td) => Some(IrClassMethod {
                        name: td.name.clone(),
                        virtual_flag: td.virtual_flag,
                        ports: td.ports.clone(),
                        decls: td.decls.clone(),
                        stmts: td.stmts.clone(),
                    }),
                    _ => None,
                }
            }).collect();
            classes.insert(cd.name.clone(), IrClassDef {
                name: cd.name.clone(),
                extends: cd.extends.clone(),
                fields,
                methods,
            });
        }
        Ok(classes)
    }

    fn flatten_instances(&mut self, top: &mut IrModule) -> Result<(), String> {
        let instances = std::mem::take(&mut top.sub_instances);
        for inst in &instances {
            let ast_module_clone = self.design.modules.iter()
                .find(|m| m.name == inst.module_name)
                .ok_or_else(|| format!("module '{}' not found for instance '{}'",
                    inst.module_name, inst.instance_name))?
                .clone();

            let needs_custom_params = !ast_module_clone.params.is_empty() && !inst.param_map.is_empty();
            let mut child = if needs_custom_params {
                let known_mods: Vec<String> = self.design.modules.iter().map(|m| m.name.clone()).collect();
                let param_vals = self.resolve_param_values(&ast_module_clone, &inst.param_map)?;
                self.elaborate_module_with_params(&ast_module_clone, &known_mods, &param_vals)?
            } else {
                // Use pre-elaborated module (default params)
                self.modules.get(&inst.module_name)
                    .ok_or_else(|| format!("module '{}' not found", inst.module_name))?
                    .clone()
            };

            // Recursively flatten child's own instances
            self.flatten_instances(&mut child)?;

            // Build signal remapping: child_signal_id -> parent_signal_id
            let mut sig_remap: Vec<Option<SignalId>> = vec![None; child.signals.len()];
            let mut next_parent_id = top.signals.len();

            // Map port connections
            for (port_name, &parent_sig) in &inst.port_map {
                if let Some(child_sig) = child.signals.iter().position(|s| s.name == *port_name) {
                    sig_remap[child_sig] = Some(parent_sig);
                }
            }

            // Allocate parent signal IDs for unmapped child signals (internal signals)
            for (i, sig) in child.signals.iter().enumerate() {
                if sig_remap[i].is_none() {
                    let new_id = next_parent_id;
                    next_parent_id += 1;
                    sig_remap[i] = Some(new_id);
                    top.signals.push(SignalInfo {
                        name: format!("{}.{}", inst.instance_name, sig.name),
                        width: sig.width,
                        kind: sig.kind.clone(),
                        init_val: sig.init_val.clone(),
                        array_depth: sig.array_depth,
                        elem_width: sig.elem_width,
                        class_name: sig.class_name.clone(),
                    });
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
        Ok(())
    }

    fn translate_process(&self, process: &Process, map_sig: &dyn Fn(SignalId) -> SignalId) -> Result<Process, String> {
        match process {
            Process::Combinational { name, sensitivity, body } => {
                let new_sens = sensitivity.iter().map(|s| map_sig(*s)).collect();
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::Combinational { name: name.clone(), sensitivity: new_sens, body: new_body })
            }
            Process::Sequential { name, clock, reset, body } => {
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
                Ok(Process::Sequential { name: name.clone(), clock: new_clock, reset: new_reset, body: new_body })
            }
            Process::Initial { name, body } => {
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::Initial { name: name.clone(), body: new_body })
            }
            Process::AlwaysWithDelay { name, delay, body } => {
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::AlwaysWithDelay { name: name.clone(), delay: *delay, body: new_body })
            }
        }
    }

    fn translate_stmts(&self, stmts: &[IrStmt], map_sig: &dyn Fn(SignalId) -> SignalId) -> Result<Vec<IrStmt>, String> {
        stmts.iter().map(|s| self.translate_stmt(s, map_sig)).collect()
    }

    fn translate_stmt(&self, stmt: &IrStmt, map_sig: &dyn Fn(SignalId) -> SignalId) -> Result<IrStmt, String> {
        match stmt {
            IrStmt::Block { stmts } => {
                let new = self.translate_stmts(stmts, map_sig)?;
                Ok(IrStmt::Block { stmts: new })
            }
            IrStmt::NamedBlock { name, stmts } => {
                let new = self.translate_stmts(stmts, map_sig)?;
                Ok(IrStmt::NamedBlock { name: name.clone(), stmts: new })
            }
            IrStmt::BlockingAssign { lhs, rhs, delay } => {
                let new_lhs = self.translate_lvalue(lhs, map_sig);
                let new_rhs = self.translate_expr(rhs, map_sig);
                Ok(IrStmt::BlockingAssign { lhs: new_lhs, rhs: new_rhs, delay: *delay })
            }
            IrStmt::NonBlockingAssign { lhs, rhs, delay } => {
                let new_lhs = self.translate_lvalue(lhs, map_sig);
                let new_rhs = self.translate_expr(rhs, map_sig);
                Ok(IrStmt::NonBlockingAssign { lhs: new_lhs, rhs: new_rhs, delay: *delay })
            }
            IrStmt::If { cond, true_branch, false_branch } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_true = self.translate_stmts(true_branch, map_sig)?;
                let new_false = self.translate_stmts(false_branch, map_sig)?;
                Ok(IrStmt::If { cond: new_cond, true_branch: new_true, false_branch: new_false })
            }
            IrStmt::Case { case_type, expr, items, default } => {
                let new_expr = self.translate_expr(expr, map_sig);
                let new_items = items.iter().map(|item| {
                    let labels = item.labels.iter().map(|l| self.translate_expr(l, map_sig)).collect();
                    let body = self.translate_stmts(&item.body, map_sig)?;
                    Ok(IrCaseItem { labels, body })
                }).collect::<Result<Vec<_>, String>>()?;
                let new_default = self.translate_stmts(default, map_sig)?;
                Ok(IrStmt::Case { case_type: case_type.clone(), expr: new_expr, items: new_items, default: new_default })
            }
            IrStmt::LoopFor { init, cond, step, body } => {
                let new_init = init.as_ref().map(|i| Box::new(self.translate_stmt(i, map_sig).unwrap_or(IrStmt::Null)));
                let new_cond = self.translate_expr(cond, map_sig);
                let new_step = step.as_ref().map(|s| Box::new(self.translate_stmt(s, map_sig).unwrap_or(IrStmt::Null)));
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::LoopFor { init: new_init, cond: new_cond, step: new_step, body: new_body })
            }
            IrStmt::LoopWhile { cond, body } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::LoopWhile { cond: new_cond, body: new_body })
            }
            IrStmt::LoopDoWhile { cond, body } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::LoopDoWhile { cond: new_cond, body: new_body })
            }
            IrStmt::Delay { delay, body } => {
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::Delay { delay: *delay, body: new_body })
            }
            IrStmt::Wait { cond, body } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::Wait { cond: new_cond, body: new_body })
            }
            IrStmt::SysCall { name, args } => {
                let new_args = args.iter().map(|a| self.translate_expr(a, map_sig)).collect();
                Ok(IrStmt::SysCall { name: name.clone(), args: new_args })
            }
            IrStmt::EventControl { sig_id, edge, body } => {
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::EventControl { sig_id: map_sig(*sig_id), edge: edge.clone(), body: new_body })
            }
            IrStmt::EventTrigger { sig_id } => {
                Ok(IrStmt::EventTrigger { sig_id: map_sig(*sig_id) })
            }
            IrStmt::SysFinish => Ok(IrStmt::SysFinish),
            IrStmt::Null => Ok(IrStmt::Null),
            IrStmt::MethodCallStmt { obj, method, args } => {
                Ok(IrStmt::MethodCallStmt {
                    obj: self.translate_expr(obj, map_sig),
                    method: method.clone(),
                    args: args.iter().map(|a| self.translate_expr(a, map_sig)).collect(),
                })
            }
            IrStmt::Break => Ok(IrStmt::Break),
            IrStmt::Continue => Ok(IrStmt::Continue),
            IrStmt::Disable { name } => {
                Ok(IrStmt::Disable { name: name.clone() })
            }
            IrStmt::Release { lvalue } => {
                Ok(IrStmt::Release { lvalue: self.translate_lvalue(lvalue, map_sig) })
            }
            IrStmt::Deassign { lvalue } => {
                Ok(IrStmt::Deassign { lvalue: self.translate_lvalue(lvalue, map_sig) })
            }
        }
    }

    fn translate_lvalue(&self, lv: &IrLValue, map_sig: &dyn Fn(SignalId) -> SignalId) -> IrLValue {
        match lv {
            IrLValue::Signal(id, w) => IrLValue::Signal(map_sig(*id), *w),
            IrLValue::RangeSelect(id, msb, lsb) => IrLValue::RangeSelect(map_sig(*id), *msb, *lsb),
            IrLValue::BitSelect(id, idx) => IrLValue::BitSelect(map_sig(*id), *idx),
            IrLValue::ArrayIndex { sig_id, index, elem_width } => IrLValue::ArrayIndex {
                sig_id: map_sig(*sig_id),
                index: Box::new(self.translate_expr(index, map_sig)),
                elem_width: *elem_width,
            },
            IrLValue::ArrayRangeSelect { sig_id, index, elem_width, msb, lsb } => IrLValue::ArrayRangeSelect {
                sig_id: map_sig(*sig_id),
                index: Box::new(self.translate_expr(index, map_sig)),
                elem_width: *elem_width,
                msb: *msb,
                lsb: *lsb,
            },
            IrLValue::ArrayBitSelect { sig_id, index, elem_width, bit } => IrLValue::ArrayBitSelect {
                sig_id: map_sig(*sig_id),
                index: Box::new(self.translate_expr(index, map_sig)),
                elem_width: *elem_width,
                bit: *bit,
            },
            IrLValue::Concat(parts) => IrLValue::Concat(parts.iter().map(|p| self.translate_lvalue(p, map_sig)).collect()),
        }
    }

    fn translate_expr(&self, expr: &IrExpr, map_sig: &dyn Fn(SignalId) -> SignalId) -> IrExpr {
        match expr {
            IrExpr::Const(v) => IrExpr::Const(v.clone()),
            IrExpr::FillLit(val) => IrExpr::FillLit(*val),
            IrExpr::Signal(id, w) => IrExpr::Signal(map_sig(*id), *w),
            IrExpr::RangeSelect(id, msb, lsb) => IrExpr::RangeSelect(map_sig(*id), *msb, *lsb),
            IrExpr::BitSelect(id, idx) => IrExpr::BitSelect(map_sig(*id), *idx),
            IrExpr::ArrayIndex { sig_id, index, elem_width } => IrExpr::ArrayIndex {
                sig_id: map_sig(*sig_id),
                index: Box::new(self.translate_expr(index, map_sig)),
                elem_width: *elem_width,
            },
            IrExpr::Concat(exprs) => IrExpr::Concat(exprs.iter().map(|e| self.translate_expr(e, map_sig)).collect()),
            IrExpr::Replicate(n, inner) => IrExpr::Replicate(*n, Box::new(self.translate_expr(inner, map_sig))),
            IrExpr::UnaryOp(op, inner) => IrExpr::UnaryOp(op.clone(), Box::new(self.translate_expr(inner, map_sig))),
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
                args: args.iter().map(|a| self.translate_expr(a, map_sig)).collect(),
            },
            IrExpr::NewCall { class_name, args } => IrExpr::NewCall {
                class_name: class_name.clone(),
                args: args.iter().map(|a| self.translate_expr(a, map_sig)).collect(),
            },
            IrExpr::This => IrExpr::This,
            IrExpr::MethodCall { obj, method, args } => IrExpr::MethodCall {
                obj: Box::new(self.translate_expr(obj, map_sig)),
                method: method.clone(),
                args: args.iter().map(|a| self.translate_expr(a, map_sig)).collect(),
            },
            IrExpr::MemberAccess { obj, field } => IrExpr::MemberAccess {
                obj: Box::new(self.translate_expr(obj, map_sig)),
                field: field.clone(),
            },
            IrExpr::ExprRangeSelect(inner, msb, lsb) => IrExpr::ExprRangeSelect(
                Box::new(self.translate_expr(inner, map_sig)),
                *msb,
                *lsb,
            ),
            IrExpr::ExprBitSelect(inner, idx) => IrExpr::ExprBitSelect(
                Box::new(self.translate_expr(inner, map_sig)),
                *idx,
            ),
            IrExpr::ExprPartSelect(inner, base_expr, width_expr) => IrExpr::ExprPartSelect(
                Box::new(self.translate_expr(inner, map_sig)),
                Box::new(self.translate_expr(base_expr, map_sig)),
                Box::new(self.translate_expr(width_expr, map_sig)),
            ),
        }
    }

    fn elaborate_always(&self, always: &AlwaysBlock, signal_map: &HashMap<String, SignalId>,
                         signals: &[SignalInfo])
        -> Result<Process, String>
    {
        let name = format!("always_{}", 0);

        match always.kind {
            AlwaysKind::AlwaysComb => {
                let body = self.elaborate_stmt_block(&always.stmts, signal_map, &[], signals)?;
                let sensitivity = match &always.sensitivity {
                    Some(sl) => {
                        sl.events.iter().filter_map(|e| match e {
                            SensitivityEvent::Level(expr) => {
                                resolve_expr_signal(expr, signal_map)
                            }
                            _ => None,
                        }).collect()
                    }
                    None => {
                        infer_comb_sensitivity(&body)
                    }
                };
                Ok(Process::Combinational { name, sensitivity, body })
            }
            AlwaysKind::AlwaysFF => {
                let (clock, reset) = self.extract_clock_reset(&always.sensitivity, signal_map)?;
                let body = self.elaborate_stmt_block(&always.stmts, signal_map, &[], signals)?;
                Ok(Process::Sequential { name, clock, reset, body })
            }
            AlwaysKind::Always | AlwaysKind::AlwaysLatch => {
                // Check if body starts with a delay (always #N pattern)
                if always.sensitivity.is_none()
                    && always.stmts.len() == 1
                {
                    if let Stmt::Delay { delay, stmt } = &always.stmts[0] {
                        if let Ok(d) = const_eval(delay) {
                            let body = self.elaborate_stmt_block(&[stmt.as_ref().clone()], signal_map, &[], signals)?;
                            return Ok(Process::AlwaysWithDelay {
                                name,
                                delay: d as u64,
                                body,
                            });
                        }
                    }
                }
                // Check if sensitivity has clock edges -> Sequential process
                if let Some(sl) = &always.sensitivity {
                    if sl.events.iter().any(|e| matches!(e, SensitivityEvent::PosEdge(_) | SensitivityEvent::NegEdge(_))) {
                        match self.extract_clock_reset(&always.sensitivity, signal_map) {
                            Ok((clock, reset)) => {
                                let body = self.elaborate_stmt_block(&always.stmts, signal_map, &[], signals)?;
                                return Ok(Process::Sequential { name, clock, reset, body });
                            }
                            Err(_) => {} // fall through to combinational
                        }
                    }
                }
                let body = self.elaborate_stmt_block(&always.stmts, signal_map, &[], signals)?;
                let sensitivity = match &always.sensitivity {
                    Some(sl) => {
                        let has_wildcard = sl.events.iter().any(|e| matches!(e, SensitivityEvent::Wildcard));
                        if has_wildcard {
                            infer_comb_sensitivity(&body)
                        } else {
                            sl.events.iter().filter_map(|e| match e {
                                SensitivityEvent::Level(expr) => resolve_expr_signal(expr, signal_map),
                                _ => None,
                            }).collect()
                        }
                    }
                    None => Vec::new(),
                };
                Ok(Process::Combinational { name, sensitivity, body })
            }
        }
    }

    fn extract_clock_reset(&self, sensitivity: &Option<SensitivityList>,
                           signal_map: &HashMap<String, SignalId>)
        -> Result<(ClockEdge, Option<ResetInfo>), String>
    {
        let events = match sensitivity {
            Some(sl) => &sl.events,
            None => return Err("always_ff requires sensitivity list".to_string()),
        };

        let mut clock_edge = None;
        let mut reset = None;

        for event in events {
            match event {
                SensitivityEvent::PosEdge(expr) | SensitivityEvent::NegEdge(expr) => {
                    let sig_id = resolve_expr_signal(expr, signal_map);
                    let is_pos = matches!(event, SensitivityEvent::PosEdge(_));
                    if let Some(sid) = sig_id {
                        if clock_edge.is_none() {
                            clock_edge = Some(if is_pos {
                                ClockEdge::PosEdge(sid)
                            } else {
                                ClockEdge::NegEdge(sid)
                            });
                        } else if reset.is_none() {
                            reset = Some(ResetInfo {
                                signal: sid,
                                polarity: is_pos,
                                r#async: true,
                                value: LogicVec::new(1),
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        clock_edge.ok_or_else(|| "always_ff must have at least one clock edge".to_string())
            .map(|ce| (ce, reset))
    }

    fn elaborate_stmt_block(&self, stmts: &[Stmt],
                            signal_map: &HashMap<String, SignalId>,
                            _known_modules: &[String],
                            signals: &[SignalInfo])
        -> Result<Vec<IrStmt>, String>
    {
        let mut ir_stmts = Vec::new();
        for stmt in stmts {
            ir_stmts.push(self.elaborate_stmt(stmt, signal_map, _known_modules, signals)?);
        }
        Ok(ir_stmts)
    }

    fn elaborate_stmt(&self, stmt: &Stmt,
                      signal_map: &HashMap<String, SignalId>,
                      known_modules: &[String],
                      signals: &[SignalInfo])
        -> Result<IrStmt, String>
    {
        match stmt {
            Stmt::Block { stmts } => {
                let body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::Block { stmts: body })
            }
            Stmt::BlockingAssign { lhs, rhs, .. } => {
                let ir_lhs = self.elaborate_lvalue(lhs, signal_map, signals)?;
                let mut ir_rhs = self.elaborate_expr(rhs, signal_map, signals)?;
                // Fill in class name for new() calls from LHS signal info
                if let IrExpr::NewCall { ref mut class_name, .. } = ir_rhs {
                    if class_name.is_empty() {
                        if let IrLValue::Signal(sid, _) = ir_lhs {
                            if let Some(sig) = signals.get(sid) {
                                if let Some(cn) = &sig.class_name {
                                    *class_name = cn.clone();
                                }
                            }
                        }
                    }
                }
                Ok(IrStmt::BlockingAssign { lhs: ir_lhs, rhs: ir_rhs, delay: None })
            }
            Stmt::NonBlockingAssign { lhs, rhs, .. } => {
                let ir_lhs = self.elaborate_lvalue(lhs, signal_map, signals)?;
                let mut ir_rhs = self.elaborate_expr(rhs, signal_map, signals)?;
                if let IrExpr::NewCall { ref mut class_name, .. } = ir_rhs {
                    if class_name.is_empty() {
                        if let IrLValue::Signal(sid, _) = ir_lhs {
                            if let Some(sig) = signals.get(sid) {
                                if let Some(cn) = &sig.class_name {
                                    *class_name = cn.clone();
                                }
                            }
                        }
                    }
                }
                Ok(IrStmt::NonBlockingAssign { lhs: ir_lhs, rhs: ir_rhs, delay: None })
            }
            Stmt::IfElse { cond, true_branch, false_branch } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let true_stmt = vec![self.elaborate_stmt(true_branch, signal_map, known_modules, signals)?];
                let false_stmt = match false_branch {
                    Some(fb) => vec![self.elaborate_stmt(fb, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::If { cond: ir_cond, true_branch: true_stmt, false_branch: false_stmt })
            }
            Stmt::Case { expr, items, default } => {
                let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                let mut ir_items = Vec::new();
                for item in items {
                    let mut labels = Vec::new();
                    for label in &item.labels {
                        labels.push(self.elaborate_expr(label, signal_map, signals)?);
                    }
                    let body = match &*item.stmt {
                        Stmt::Block { stmts } => self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?,
                        other => self.elaborate_stmt_block(&[other.clone()], signal_map, known_modules, signals)?,
                    };
                    ir_items.push(IrCaseItem { labels, body });
                }
                let ir_default = match default {
                    Some(d) => {
                        vec![self.elaborate_stmt(d, signal_map, known_modules, signals)?]
                    }
                    None => vec![],
                };
                Ok(IrStmt::Case { case_type: CaseType::Normal, expr: ir_expr, items: ir_items, default: ir_default })
            }
            Stmt::StmtAssign { lhs, rhs } => {
                let ir_lhs = self.elaborate_lvalue(lhs, signal_map, signals)?;
                let ir_rhs = self.elaborate_expr(rhs, signal_map, signals)?;
                Ok(IrStmt::BlockingAssign { lhs: ir_lhs, rhs: ir_rhs, delay: None })
            }
            Stmt::Expr { expr } => {
                match expr {
                    Expr::MethodCall { obj, method, args } => {
                        let ir_obj = self.elaborate_expr(obj, signal_map, signals)?;
                        let ir_args: Vec<IrExpr> = args.iter()
                            .map(|a| self.elaborate_expr(a, signal_map, signals))
                            .collect::<Result<_, _>>()?;
                        Ok(IrStmt::MethodCallStmt {
                            obj: ir_obj,
                            method: method.clone(),
                            args: ir_args,
                        })
                    }
                    _ => {
                        let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                        // Evaluate expr for side effects, discard result
                        Ok(IrStmt::SysCall { name: String::new(), args: vec![ir_expr] })
                    }
                }
            }
            Stmt::SysCall { name, args } => {
                let ir_args: Vec<IrExpr> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect::<Result<_, _>>()?;
                Ok(IrStmt::SysCall { name: name.clone(), args: ir_args })
            }
            Stmt::SysFinish => Ok(IrStmt::SysFinish),
            Stmt::Null => Ok(IrStmt::Null),
            Stmt::Return(_) => Ok(IrStmt::Null),
            Stmt::EventControl { events, stmt } => {
                if events.is_empty() {
                    return Ok(IrStmt::Null);
                }
                let event = &events[0];
                let body = match stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                match event {
                    SensitivityEvent::PosEdge(expr) | SensitivityEvent::NegEdge(expr) => {
                        let is_pos = matches!(event, SensitivityEvent::PosEdge(_));
                        if let Some(sig_id) = resolve_expr_signal(expr, signal_map) {
                            let edge = if is_pos {
                                ClockEdge::PosEdge(sig_id)
                            } else {
                                ClockEdge::NegEdge(sig_id)
                            };
                            Ok(IrStmt::EventControl { sig_id, edge: Some(edge), body })
                        } else {
                            Err(format!("cannot resolve signal in @(...)"))
                        }
                    }
                    SensitivityEvent::Level(expr) => {
                        if let Some(sig_id) = resolve_expr_signal(expr, signal_map) {
                            Ok(IrStmt::EventControl { sig_id, edge: None, body })
                        } else {
                            Err(format!("cannot resolve signal in @(...)"))
                        }
                    }
                    SensitivityEvent::Wildcard => {
                        // @(*) in procedural context: wait for any signal change
                        // For now, treat as immediate
                        Ok(IrStmt::Block { stmts: body })
                    }
                }
            }
            Stmt::EventTrigger { name } => {
                if let Some(sig_id) = signal_map.get(name) {
                    Ok(IrStmt::EventTrigger { sig_id: *sig_id })
                } else {
                    Ok(IrStmt::Null)
                }
            }
            Stmt::Force { lhs, rhs } => {
                let ir_lhs = self.elaborate_lvalue(lhs, signal_map, signals)?;
                let ir_rhs = self.elaborate_expr(rhs, signal_map, signals)?;
                Ok(IrStmt::BlockingAssign { lhs: ir_lhs, rhs: ir_rhs, delay: None })
            }
            Stmt::Release { expr } => {
                let ir_lhs = self.elaborate_lvalue(expr, signal_map, signals)?;
                Ok(IrStmt::Release { lvalue: ir_lhs })
            }
            Stmt::Deassign { expr } => {
                let ir_lhs = self.elaborate_lvalue(expr, signal_map, signals)?;
                Ok(IrStmt::Deassign { lvalue: ir_lhs })
            }
            Stmt::CaseX { expr, items, default } => {
                let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                let mut ir_items = Vec::new();
                for item in items {
                    let mut labels = Vec::new();
                    for label in &item.labels {
                        labels.push(self.elaborate_expr(label, signal_map, signals)?);
                    }
                    let body = match &*item.stmt {
                        Stmt::Block { stmts } => self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?,
                        other => self.elaborate_stmt_block(&[other.clone()], signal_map, known_modules, signals)?,
                    };
                    ir_items.push(IrCaseItem { labels, body });
                }
                let ir_default = match default {
                    Some(d) => vec![self.elaborate_stmt(d, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Case { case_type: CaseType::CaseX, expr: ir_expr, items: ir_items, default: ir_default })
            }
            Stmt::CaseZ { expr, items, default } => {
                let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                let mut ir_items = Vec::new();
                for item in items {
                    let mut labels = Vec::new();
                    for label in &item.labels {
                        labels.push(self.elaborate_expr(label, signal_map, signals)?);
                    }
                    let body = match &*item.stmt {
                        Stmt::Block { stmts } => self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?,
                        other => self.elaborate_stmt_block(&[other.clone()], signal_map, known_modules, signals)?,
                    };
                    ir_items.push(IrCaseItem { labels, body });
                }
                let ir_default = match default {
                    Some(d) => vec![self.elaborate_stmt(d, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Case { case_type: CaseType::CaseZ, expr: ir_expr, items: ir_items, default: ir_default })
            }
            Stmt::NamedBlock { name, stmts, decls: _ } => {
                let body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::NamedBlock { name: name.clone(), stmts: body })
            }
            Stmt::Delay { delay, stmt } => {
                let d = const_eval(delay)? as u64;
                let body = vec![self.elaborate_stmt(stmt, signal_map, known_modules, signals)?];
                Ok(IrStmt::Delay { delay: d, body })
            }
            Stmt::Wait { cond, stmt } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let body = match stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Wait { cond: ir_cond, body })
            }
            Stmt::LoopFor { init, cond, step, stmts } => {
                // Try to unroll constant-bounded for loops at elaboration time
                if let Ok(Some(unrolled)) = try_unroll_for_loop(
                    init.as_deref(), cond.as_ref(), step.as_deref(), stmts,
                    &|stmts, var_name, iter_val| {
                        let subst_stmts = substitute_loop_var_in_stmts(stmts, var_name, iter_val);
                        self.elaborate_stmt_block(&subst_stmts, signal_map, known_modules, signals)
                    }
                ) {
                    return Ok(IrStmt::Block { stmts: unrolled });
                }
                // Fallback: generate runtime LoopFor
                let ir_init = match init {
                    Some(s) => Some(Box::new(self.elaborate_stmt(s, signal_map, known_modules, signals)?)),
                    None => None,
                };
                let ir_cond = if let Some(c) = cond {
                    self.elaborate_expr(c, signal_map, signals)?
                } else {
                    IrExpr::Const(LogicVec::from_u64(1, 1))
                };
                let ir_step = match step {
                    Some(s) => Some(Box::new(self.elaborate_stmt(s, signal_map, known_modules, signals)?)),
                    None => None,
                };
                let ir_body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::LoopFor { init: ir_init, cond: ir_cond, step: ir_step, body: ir_body })
            }
            Stmt::LoopWhile { cond, stmts } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let ir_body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::LoopWhile { cond: ir_cond, body: ir_body })
            }
            Stmt::DoWhile { cond, stmts } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let ir_body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::LoopDoWhile { cond: ir_cond, body: ir_body })
            }
            Stmt::LoopForever { stmts } => {
                let ir_body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::LoopWhile {
                    cond: IrExpr::Const(LogicVec::from_u64(1, 1)),
                    body: ir_body,
                })
            }
            Stmt::ForeachLoop { array_var, index_var, stmts } => {
                // Find the array signal and unroll the loop
                let sig_id = signal_map.get(array_var)
                    .ok_or_else(|| format!("array '{}' not found for foreach", array_var))?;
                let sig_info = signals.get(*sig_id)
                    .ok_or_else(|| format!("signal info not found for '{}'", array_var))?;
                let n = sig_info.array_depth;
                if n == 0 {
                    return Err(format!("'{}' is not an array, cannot use foreach", array_var));
                }
                let mut all_stmts = Vec::new();
                for i in 0..n {
                    let subst_stmts = substitute_loop_var_in_stmts(stmts, index_var, i as i64);
                    all_stmts.extend(self.elaborate_stmt_block(&subst_stmts, signal_map, known_modules, signals)?);
                }
                Ok(IrStmt::Block { stmts: all_stmts })
            }
            Stmt::StmtCase { expr, items, default } => {
                let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                let mut ir_items = Vec::new();
                for item in items {
                    let mut labels = Vec::new();
                    for label in &item.labels {
                        labels.push(self.elaborate_expr(label, signal_map, signals)?);
                    }
                    let body = match &*item.stmt {
                        Stmt::Block { stmts } => self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?,
                        other => self.elaborate_stmt_block(&[other.clone()], signal_map, known_modules, signals)?,
                    };
                    ir_items.push(IrCaseItem { labels, body });
                }
                let ir_default = match default {
                    Some(d) => vec![self.elaborate_stmt(d, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Case { case_type: CaseType::Normal, expr: ir_expr, items: ir_items, default: ir_default })
            }
            Stmt::Break => Ok(IrStmt::Break),
            Stmt::Continue => Ok(IrStmt::Continue),
            Stmt::Disable { name } => {
                Ok(IrStmt::Disable { name: name.clone() })
            }
            Stmt::Repeat { count, stmts } => {
                let n = const_eval(count)?;
                let mut all = Vec::new();
                for _ in 0..n {
                    all.extend(self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?);
                }
                Ok(IrStmt::Block { stmts: all })
            }
        }
    }

    fn elaborate_lvalue(&self, expr: &Expr, signal_map: &HashMap<String, SignalId>, signals: &[SignalInfo]) -> Result<IrLValue, String> {
        match expr {
            Expr::Ident(name) => {
                let sig_id = signal_map.get(name)
                    .ok_or_else(|| format!("signal '{}' not found", name))?;
                Ok(IrLValue::Signal(*sig_id, 0))
            }
            Expr::RangeSelect { expr: inner, msb, lsb } => {
                let inner_lv = self.elaborate_lvalue(inner, signal_map, signals)?;
                let msb_c = const_eval(msb)? as usize;
                let lsb_c = const_eval(lsb)? as usize;
                match inner_lv {
                    IrLValue::Signal(sid, _) => {
                        Ok(IrLValue::RangeSelect(sid, msb_c, lsb_c))
                    }
                    IrLValue::RangeSelect(sid, outer_msb, outer_lsb) => {
                        let outer_start = if outer_msb > outer_lsb { outer_lsb } else { outer_msb };
                        let inner_start = outer_start + if msb_c > lsb_c { lsb_c } else { msb_c };
                        let inner_end = outer_start + if msb_c > lsb_c { msb_c } else { lsb_c };
                        Ok(IrLValue::RangeSelect(sid, inner_end, inner_start))
                    }
                    IrLValue::ArrayIndex { sig_id, index, elem_width } => {
                        Ok(IrLValue::ArrayRangeSelect { sig_id, index, elem_width, msb: msb_c, lsb: lsb_c })
                    }
                    _ => Err("nested range select not supported".to_string()),
                }
            }
            Expr::BitSelect { expr: inner, index } => {
                let inner_lv = self.elaborate_lvalue(inner, signal_map, signals)?;
                let idx = const_eval(index)? as usize;
                match inner_lv {
                    IrLValue::Signal(sid, _) => {
                        let (array_depth, elem_width) = get_array_info(signals, sid);
                        if array_depth > 1 {
                            let index_expr = self.elaborate_expr(index, signal_map, signals)?;
                            Ok(IrLValue::ArrayIndex { sig_id: sid, index: Box::new(index_expr), elem_width })
                        } else {
                            Ok(IrLValue::BitSelect(sid, idx))
                        }
                    }
                    IrLValue::RangeSelect(sid, outer_msb, outer_lsb) => {
                        let base = if outer_msb > outer_lsb { outer_lsb } else { outer_msb };
                        Ok(IrLValue::BitSelect(sid, base + idx))
                    }
                    IrLValue::ArrayIndex { sig_id, index, elem_width } => {
                        Ok(IrLValue::ArrayBitSelect { sig_id, index, elem_width, bit: idx })
                    }
                    _ => Err("nested bit select not supported".to_string()),
                }
            }
            Expr::PartSelect { expr: inner, base, width } => {
                let inner_lv = self.elaborate_lvalue(inner, signal_map, signals)?;
                let base_c = const_eval(base)? as usize;
                let width_c = const_eval(width)? as usize;
                match inner_lv {
                    IrLValue::Signal(sid, _) => {
                        if width_c > 0 {
                            Ok(IrLValue::RangeSelect(sid, base_c + width_c - 1, base_c))
                        } else {
                            Ok(IrLValue::RangeSelect(sid, base_c, base_c))
                        }
                    }
                    IrLValue::RangeSelect(sid, outer_msb, outer_lsb) => {
                        let outer_base = if outer_msb > outer_lsb { outer_lsb } else { outer_msb };
                        let new_base = outer_base + base_c;
                        if width_c > 0 {
                            Ok(IrLValue::RangeSelect(sid, new_base + width_c - 1, new_base))
                        } else {
                            Ok(IrLValue::RangeSelect(sid, new_base, new_base))
                        }
                    }
                    IrLValue::ArrayIndex { sig_id, index, elem_width } => {
                        if width_c > 0 {
                            Ok(IrLValue::ArrayRangeSelect { sig_id, index, elem_width, msb: base_c + width_c - 1, lsb: base_c })
                        } else {
                            Ok(IrLValue::ArrayRangeSelect { sig_id, index, elem_width, msb: base_c, lsb: base_c })
                        }
                    }
                    _ => Err("nested part-select in lvalue not supported".to_string()),
                }
            }
            Expr::Concat(exprs) => {
                let parts: Result<Vec<IrLValue>, String> = exprs.iter()
                    .map(|e| self.elaborate_lvalue(e, signal_map, signals))
                    .collect();
                Ok(IrLValue::Concat(parts?))
            }
            Expr::MethodCall { .. } => Err("method calls cannot be used as lvalues".to_string()),
            Expr::MemberAccess { .. } => Err("member access cannot be used as lvalues".to_string()),
            _ => Err(format!("invalid lvalue expression: {:?}", expr)),
        }
    }

    fn elaborate_expr(&self, expr: &Expr, signal_map: &HashMap<String, SignalId>, signals: &[SignalInfo]) -> Result<IrExpr, String> {
        match expr {
            Expr::Ident(name) if name == "this" => Ok(IrExpr::This),
            Expr::Value(v) => {
                let lv = value_to_logicvec(v);
                Ok(IrExpr::Const(lv))
            }
            Expr::FillLit(val) => Ok(IrExpr::FillLit(*val)),
            Expr::Ident(name) => {
                if name.starts_with("$") {
                    return Ok(IrExpr::SysFunc { name: name.clone(), args: vec![] });
                }
                let sig_id = signal_map.get(name)
                    .ok_or_else(|| format!("signal '{}' not found", name))?;
                Ok(IrExpr::Signal(*sig_id, 0))
            }
            Expr::RangeSelect { expr: inner, msb, lsb } => {
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                let msb_c = const_eval(msb)? as usize;
                let lsb_c = const_eval(lsb)? as usize;
                if let IrExpr::Signal(sid, _) = &inner_expr {
                    Ok(IrExpr::RangeSelect(*sid, msb_c, lsb_c))
                } else {
                    Ok(IrExpr::ExprRangeSelect(Box::new(inner_expr), msb_c, lsb_c))
                }
            }
            Expr::BitSelect { expr: inner, index } => {
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                if let IrExpr::Signal(sid, _) = &inner_expr {
                    let (array_depth, elem_width) = get_array_info(signals, *sid);
                    if array_depth > 1 {
                        let index_expr = self.elaborate_expr(index, signal_map, signals)?;
                        Ok(IrExpr::ArrayIndex { sig_id: *sid, index: Box::new(index_expr), elem_width })
                    } else {
                        let idx = const_eval(index)? as usize;
                        Ok(IrExpr::BitSelect(*sid, idx))
                    }
                } else {
                    let idx = const_eval(index)? as usize;
                    Ok(IrExpr::ExprBitSelect(Box::new(inner_expr), idx))
                }
            }
            Expr::Concat(exprs) => {
                let parts: Result<Vec<IrExpr>, String> = exprs.iter()
                    .map(|e| self.elaborate_expr(e, signal_map, signals))
                    .collect();
                Ok(IrExpr::Concat(parts?))
            }
            Expr::Replicate { count, expr: inner } => {
                let c = const_eval_params(count, &self.param_vals)? as usize;
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                Ok(IrExpr::Replicate(c, Box::new(inner_expr)))
            }
            Expr::UnaryOp { op, expr: inner } => {
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                let ir_op = map_unary_op(op)?;
                Ok(IrExpr::UnaryOp(ir_op, Box::new(inner_expr)))
            }
            Expr::BinaryOp { op, lhs, rhs } => {
                let lhs_expr = self.elaborate_expr(lhs, signal_map, signals)?;
                let rhs_expr = self.elaborate_expr(rhs, signal_map, signals)?;
                let ir_op = map_binary_op(op)?;
                Ok(IrExpr::BinaryOp(ir_op, Box::new(lhs_expr), Box::new(rhs_expr)))
            }
            Expr::TernaryOp { cond, true_expr, false_expr } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let ir_true = self.elaborate_expr(true_expr, signal_map, signals)?;
                let ir_false = self.elaborate_expr(false_expr, signal_map, signals)?;
                Ok(IrExpr::Cond(Box::new(ir_cond), Box::new(ir_true), Box::new(ir_false)))
            }
            Expr::PartSelect { expr: inner, base, width } => {
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                if let IrExpr::Signal(sid, _) = &inner_expr {
                    if let (Ok(base_c), Ok(width_c)) = (const_eval(base), const_eval(width)) {
                        let base = base_c as usize;
                        let width = width_c as usize;
                        if width > 0 {
                            Ok(IrExpr::RangeSelect(*sid, base + width - 1, base))
                        } else {
                            Ok(IrExpr::RangeSelect(*sid, base, base))
                        }
                    } else {
                        let base_expr = self.elaborate_expr(base, signal_map, signals)?;
                        let width_expr = self.elaborate_expr(width, signal_map, signals)?;
                        Ok(IrExpr::ExprPartSelect(Box::new(inner_expr), Box::new(base_expr), Box::new(width_expr)))
                    }
                } else if let (Ok(base_c), Ok(width_c)) = (const_eval(base), const_eval(width)) {
                    let base = base_c as usize;
                    let width = width_c as usize;
                    if width > 0 {
                        Ok(IrExpr::ExprRangeSelect(Box::new(inner_expr), base + width - 1, base))
                    } else {
                        Ok(IrExpr::ExprRangeSelect(Box::new(inner_expr), base, base))
                    }
                } else {
                    let base_expr = self.elaborate_expr(base, signal_map, signals)?;
                    let width_expr = self.elaborate_expr(width, signal_map, signals)?;
                    Ok(IrExpr::ExprPartSelect(Box::new(inner_expr), Box::new(base_expr), Box::new(width_expr)))
                }
            }
            Expr::Paren(inner) => self.elaborate_expr(inner, signal_map, signals),
            Expr::FuncCall { name, args } if name.starts_with("$") => {
                match name.as_str() {
                    "$signed" => {
                        if args.len() != 1 {
                            return Err("$signed requires exactly one argument".to_string());
                        }
                        let inner = self.elaborate_expr(&args[0], signal_map, signals)?;
                        Ok(IrExpr::Signed(Box::new(inner)))
                    }
                    "$unsigned" => {
                        if args.len() != 1 {
                            return Err("$unsigned requires exactly one argument".to_string());
                        }
                        self.elaborate_expr(&args[0], signal_map, signals)
                    }
                    "$clog2" => {
                        if let Some(arg) = args.first() {
                            let val = const_eval_params(arg, &self.param_vals)?;
                            if val <= 1 { return Ok(IrExpr::Const(LogicVec::from_u64(0, 32))); }
                            let n = val as u64;
                            let msb = (64 - n.leading_zeros()) as u64;
                            let result = if n.is_power_of_two() { msb - 1 } else { msb };
                            Ok(IrExpr::Const(LogicVec::from_u64(result, 32)))
                        } else {
                            Err("$clog2 requires one argument".to_string())
                        }
                    }
                    "$bits" => {
                        if let Some(arg) = args.first() {
                            let sig_id = resolve_expr_signal(arg, signal_map)
                                .ok_or_else(|| "$bits argument must resolve to a signal".to_string())?;
                            let info = &signals[sig_id];
                            let total = info.width * if info.array_depth > 0 { info.array_depth } else { 1 };
                            Ok(IrExpr::Const(LogicVec::from_u64(total as u64, 32)))
                        } else {
                            Err("$bits requires one argument".to_string())
                        }
                    }
                    "$high" => {
                        if let Some(arg) = args.first() {
                            let sig_id = resolve_expr_signal(arg, signal_map)
                                .ok_or_else(|| "$high argument must resolve to a signal".to_string())?;
                            let info = &signals[sig_id];
                            Ok(IrExpr::Const(LogicVec::from_u64((info.width - 1) as u64, 32)))
                        } else {
                            Err("$high requires one argument".to_string())
                        }
                    }
                    "$low" => {
                        Ok(IrExpr::Const(LogicVec::from_u64(0, 32)))
                    }
                    "$left" => {
                        if let Some(arg) = args.first() {
                            let sig_id = resolve_expr_signal(arg, signal_map)
                                .ok_or_else(|| "$left argument must resolve to a signal".to_string())?;
                            let info = &signals[sig_id];
                            Ok(IrExpr::Const(LogicVec::from_u64((info.width - 1) as u64, 32)))
                        } else {
                            Err("$left requires one argument".to_string())
                        }
                    }
                    "$right" => {
                        Ok(IrExpr::Const(LogicVec::from_u64(0, 32)))
                    }
                    "$size" => {
                        if let Some(arg) = args.first() {
                            let sig_id = resolve_expr_signal(arg, signal_map)
                                .ok_or_else(|| "$size argument must resolve to a signal".to_string())?;
                            let info = &signals[sig_id];
                            Ok(IrExpr::Const(LogicVec::from_u64(info.width as u64, 32)))
                        } else {
                            Err("$size requires one argument".to_string())
                        }
                    }
                    _ => {
                        let ir_args: Result<Vec<IrExpr>, String> = args.iter()
                            .map(|a| self.elaborate_expr(a, signal_map, signals))
                            .collect();
                        Ok(IrExpr::SysFunc { name: name.to_string(), args: ir_args? })
                    }
                }
            }
            Expr::FuncCall { name, args } if name == "new" => {
                let ir_args: Result<Vec<IrExpr>, String> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                Ok(IrExpr::NewCall { class_name: String::new(), args: ir_args? })
            }
            Expr::String(s) => Ok(IrExpr::String(s.clone())),
            Expr::MethodCall { obj, method, args } => {
                let ir_obj = self.elaborate_expr(obj, signal_map, signals)?;
                let ir_args: Result<Vec<IrExpr>, String> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                Ok(IrExpr::MethodCall {
                    obj: Box::new(ir_obj),
                    method: method.clone(),
                    args: ir_args?,
                })
            }
            Expr::MemberAccess { obj, field } => {
                let ir_obj = self.elaborate_expr(obj, signal_map, signals)?;
                Ok(IrExpr::MemberAccess {
                    obj: Box::new(ir_obj),
                    field: field.clone(),
                })
            }
            Expr::Null => Ok(IrExpr::Const(LogicVec::from_u64(0, 64))),
            _ => Err(format!("expression type not yet supported")),
        }
    }

    fn elaborate_expr_to_signal(&self, expr: &Expr, signal_map: &HashMap<String, SignalId>)
        -> Result<SignalId, String>
    {
        match expr {
            Expr::Ident(name) => {
                signal_map.get(name)
                    .ok_or_else(|| format!("signal '{}' not found", name))
                    .copied()
            }
            Expr::MethodCall { .. } => Err("method calls cannot resolve to a signal".to_string()),
            Expr::MemberAccess { .. } => Err("member access cannot resolve to a signal".to_string()),
            _ => Err("expected simple signal identifier".to_string())
        }
    }
}

fn expand_all_generates(module: &mut Module, param_vals: &HashMap<String, i64>) -> Result<(), String> {
    let mut i = 0;
    while i < module.items.len() {
        if let ModuleItem::Generate(gen) = &module.items[i] {
            let expanded = expand_generate_block(gen, param_vals)?;
            for item in &expanded {
                if let ModuleItem::Decl(d) = item {
                    module.decls.push(d.clone());
                }
            }
            module.items.splice(i..=i, expanded);
        } else {
            i += 1;
        }
    }
    Ok(())
}

fn expand_generate_block(gen: &GenerateBlock, param_vals: &HashMap<String, i64>) -> Result<Vec<ModuleItem>, String> {
    let mut result = Vec::new();
    for item in &gen.items {
        match item {
            GenerateItem::If { cond, true_items, false_items } => {
                let val = const_eval_with_params(cond, param_vals)
                    .map_err(|_| format!("non-constant condition in generate if"))?;
                let branch = if val != 0 { true_items } else { false_items };
                for item in branch {
                    result.push(item.clone());
                }
            }
            GenerateItem::For { var, init, cond, step: _, body_items } => {
                let start_val: i64 = match init {
                    Some(Stmt::BlockingAssign { rhs, .. }) => const_eval_with_params(rhs, param_vals)?,
                    _ => 0,
                };
                let limit: i64 = match cond {
                    Some(Expr::BinaryOp { op: BinaryOp::Lt, rhs, .. }) => {
                        const_eval_with_params(rhs, param_vals)?
                    }
                    Some(Expr::BinaryOp { op: BinaryOp::Le, rhs, .. }) => {
                        const_eval_with_params(rhs, param_vals)? + 1
                    }
                    Some(c) => {
                        if const_eval_with_params(c, param_vals)? != 0 { 1 } else { 0 }
                    }
                    None => 0,
                };
                for cur in start_val..limit {
                    for mut item in body_items.clone() {
                        substitute_genvar_in_module_item(&mut item, var, cur);
                        result.push(item);
                    }
                }
            }
            GenerateItem::Items(items) => {
                for item in items {
                    result.push(item.clone());
                }
            }
        }
    }
    Ok(result)
}

fn substitute_genvar_in_module_item(item: &mut ModuleItem, var_name: &str, value: i64) {
    match item {
        ModuleItem::Always(always) => {
            for stmt in &mut always.stmts {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = substitute_loop_var_in_stmt(&old, var_name, value);
            }
        }
        ModuleItem::Initial(initial) => {
            for stmt in &mut initial.stmts {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = substitute_loop_var_in_stmt(&old, var_name, value);
            }
        }
        ModuleItem::Assign(assign) => {
            let old_lhs = std::mem::replace(
                &mut assign.lhs, Expr::Value(crate::ast::expr::Value::Decimal(0))
            );
            let old_rhs = std::mem::replace(
                &mut assign.rhs, Expr::Value(crate::ast::expr::Value::Decimal(0))
            );
            assign.lhs = substitute_loop_var_in_expr(&old_lhs, var_name, value);
            assign.rhs = substitute_loop_var_in_expr(&old_rhs, var_name, value);
        }
        ModuleItem::Instance(inst) => {
            for (_, expr) in &mut inst.param_assigns {
                let old = std::mem::replace(
                    expr, Expr::Value(crate::ast::expr::Value::Decimal(0))
                );
                *expr = substitute_loop_var_in_expr(&old, var_name, value);
            }
            for conn in &mut inst.port_conns {
                match conn {
                    PortConnection::Positional(expr) => {
                        let old = std::mem::replace(
                            expr, Expr::Value(crate::ast::expr::Value::Decimal(0))
                        );
                        *expr = substitute_loop_var_in_expr(&old, var_name, value);
                    }
                    PortConnection::Named { expr, .. } => {
                        let old = std::mem::replace(
                            expr, Expr::Value(crate::ast::expr::Value::Decimal(0))
                        );
                        *expr = substitute_loop_var_in_expr(&old, var_name, value);
                    }
                }
            }
        }
        ModuleItem::Decl(decl) => {
            for var in &mut decl.names {
                if let Some(er) = &var.expr_range {
                    let old_msb = er.msb.clone();
                    let old_lsb = er.lsb.clone();
                    let new_msb = substitute_loop_var_in_expr(&old_msb, var_name, value);
                    let new_lsb = substitute_loop_var_in_expr(&old_lsb, var_name, value);
                    if let (Ok(msb), Ok(lsb)) = (const_eval_simple(&new_msb), const_eval_simple(&new_lsb)) {
                        var.expr_range = None;
                        var.range = Some(Range { msb: msb as usize, lsb: lsb as usize });
                    }
                }
            }
        }
        ModuleItem::Gate(ref mut gate) => {
            for port in &mut gate.ports {
                let old = std::mem::replace(
                    port, Expr::Value(crate::ast::expr::Value::Decimal(0))
                );
                *port = substitute_loop_var_in_expr(&old, var_name, value);
            }
        }
        ModuleItem::Func(_) | ModuleItem::Generate(_) | ModuleItem::Typedef(_) => {}
    }
}

fn try_unroll_for_loop<'a, F>(init: Option<&'a Stmt>, cond: Option<&'a Expr>, step: Option<&'a Stmt>,
                           stmts: &[Stmt], elaborate_body: &F)
    -> Result<Option<Vec<IrStmt>>, String>
    where F: Fn(&[Stmt], &str, i64) -> Result<Vec<IrStmt>, String>
{
    // Extract loop variable name and initial value from init statement
    let (var_name, init_val) = match init {
        Some(Stmt::BlockingAssign { lhs: Expr::Ident(name), rhs, .. }) => {
            (name.clone(), const_eval(rhs)?)
        }
        _ => return Ok(None),
    };

    // Extract step function from step statement
    let step_fn: Box<dyn Fn(i64) -> Result<i64, String>> = match step {
        Some(Stmt::BlockingAssign { lhs: Expr::Ident(n), rhs, .. }) if *n == var_name => {
            match rhs {
                Expr::BinaryOp { op: BinaryOp::Add, lhs, rhs } => {
                    if let Expr::Ident(n2) = lhs.as_ref() {
                        if n2 == &var_name {
                            let inc = const_eval(rhs)?;
                            Box::new(move |v| Ok(v + inc))
                        } else { return Ok(None) }
                    } else if let Expr::Ident(n2) = rhs.as_ref() {
                        if n2 == &var_name {
                            let inc = const_eval(lhs)?;
                            Box::new(move |v| Ok(v + inc))
                        } else { return Ok(None) }
                    } else { return Ok(None) }
                }
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };

    // Extract loop limit from condition: var < limit
    let limit = match cond {
        Some(Expr::BinaryOp { op: BinaryOp::Lt, lhs, rhs }) => {
            match lhs.as_ref() {
                Expr::Ident(n) if *n == var_name => const_eval(rhs)?,
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };

    // Unroll the loop
    let mut all_stmts = Vec::new();
    let mut ivar = init_val;
    while ivar < limit {
        let body = elaborate_body(stmts, &var_name, ivar)?;
        all_stmts.extend(body);
        ivar = step_fn(ivar)?;
    }

    Ok(Some(all_stmts))
}

fn substitute_loop_var_in_stmts(stmts: &[Stmt], var_name: &str, value: i64) -> Vec<Stmt> {
    stmts.iter().map(|s| substitute_loop_var_in_stmt(s, var_name, value)).collect()
}

fn substitute_loop_var_in_stmt(stmt: &Stmt, var_name: &str, value: i64) -> Stmt {
    match stmt {
        Stmt::Block { stmts } => Stmt::Block {
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::BlockingAssign { lhs, rhs, delay } => Stmt::BlockingAssign {
            lhs: substitute_loop_var_in_expr(lhs, var_name, value),
            rhs: substitute_loop_var_in_expr(rhs, var_name, value),
            delay: delay.clone(),
        },
        Stmt::NonBlockingAssign { lhs, rhs, delay } => Stmt::NonBlockingAssign {
            lhs: substitute_loop_var_in_expr(lhs, var_name, value),
            rhs: substitute_loop_var_in_expr(rhs, var_name, value),
            delay: delay.clone(),
        },
        Stmt::IfElse { cond, true_branch, false_branch } => Stmt::IfElse {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            true_branch: Box::new(substitute_loop_var_in_stmt(true_branch, var_name, value)),
            false_branch: false_branch.as_ref().map(|fb|
                Box::new(substitute_loop_var_in_stmt(fb, var_name, value))),
        },
        Stmt::Case { expr, items, default } => Stmt::Case {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items.iter().map(|item| crate::ast::stmt::CaseItem {
                labels: item.labels.iter().map(|l| substitute_loop_var_in_expr(l, var_name, value)).collect(),
                stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
            }).collect(),
            default: default.as_ref().map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::StmtAssign { lhs, rhs } => Stmt::StmtAssign {
            lhs: substitute_loop_var_in_expr(lhs, var_name, value),
            rhs: substitute_loop_var_in_expr(rhs, var_name, value),
        },
        Stmt::Delay { delay, stmt } => Stmt::Delay {
            delay: substitute_loop_var_in_expr(delay, var_name, value),
            stmt: Box::new(substitute_loop_var_in_stmt(stmt, var_name, value)),
        },
        Stmt::SysCall { name, args } => Stmt::SysCall {
            name: name.clone(),
            args: args.iter().map(|a| substitute_loop_var_in_expr(a, var_name, value)).collect(),
        },
        Stmt::Expr { expr } => Stmt::Expr {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
        },
        Stmt::CaseX { expr, items, default } => Stmt::CaseX {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items.iter().map(|item| crate::ast::stmt::CaseItem {
                labels: item.labels.iter().map(|l| substitute_loop_var_in_expr(l, var_name, value)).collect(),
                stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
            }).collect(),
            default: default.as_ref().map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::CaseZ { expr, items, default } => Stmt::CaseZ {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items.iter().map(|item| crate::ast::stmt::CaseItem {
                labels: item.labels.iter().map(|l| substitute_loop_var_in_expr(l, var_name, value)).collect(),
                stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
            }).collect(),
            default: default.as_ref().map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::StmtCase { expr, items, default } => Stmt::StmtCase {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items.iter().map(|item| crate::ast::stmt::CaseItem {
                labels: item.labels.iter().map(|l| substitute_loop_var_in_expr(l, var_name, value)).collect(),
                stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
            }).collect(),
            default: default.as_ref().map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::LoopForever { stmts } => Stmt::LoopForever {
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::LoopWhile { cond, stmts } => Stmt::LoopWhile {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::LoopFor { init, cond, step, stmts } => Stmt::LoopFor {
            init: init.as_ref().map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
            cond: cond.as_ref().map(|c| substitute_loop_var_in_expr(c, var_name, value)),
            step: step.as_ref().map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::Repeat { count, stmts } => Stmt::Repeat {
            count: substitute_loop_var_in_expr(count, var_name, value),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::Wait { cond, stmt } => Stmt::Wait {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            stmt: stmt.as_ref().map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
        },
        Stmt::Disable { name } => Stmt::Disable { name: name.clone() },
        Stmt::Force { lhs, rhs } => Stmt::Force {
            lhs: substitute_loop_var_in_expr(lhs, var_name, value),
            rhs: substitute_loop_var_in_expr(rhs, var_name, value),
        },
        Stmt::Release { expr } => Stmt::Release {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
        },
        Stmt::Deassign { expr } => Stmt::Deassign {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
        },
        Stmt::Return(expr) => Stmt::Return(
            expr.as_ref().map(|e| Box::new(substitute_loop_var_in_expr(e, var_name, value))),
        ),
        Stmt::Null => Stmt::Null,
        Stmt::SysFinish => Stmt::SysFinish,
        Stmt::EventControl { events, stmt } => Stmt::EventControl {
            events: events.iter().map(|e| substitute_sensitivity_event(e, var_name, value)).collect(),
            stmt: stmt.as_ref().map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
        },
        Stmt::EventTrigger { name } => Stmt::EventTrigger { name: name.clone() },
        Stmt::ForeachLoop { array_var, index_var, stmts } => Stmt::ForeachLoop {
            array_var: array_var.clone(),
            index_var: index_var.clone(),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::NamedBlock { name, stmts, decls } => Stmt::NamedBlock {
            name: name.clone(),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
            decls: decls.clone(),
        },
        Stmt::Break => Stmt::Break,
        Stmt::Continue => Stmt::Continue,
        Stmt::DoWhile { cond, stmts } => Stmt::DoWhile {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
    }
}

fn substitute_sensitivity_event(event: &SensitivityEvent, var_name: &str, value: i64) -> SensitivityEvent {
    match event {
        SensitivityEvent::PosEdge(e) => SensitivityEvent::PosEdge(substitute_loop_var_in_expr(e, var_name, value)),
        SensitivityEvent::NegEdge(e) => SensitivityEvent::NegEdge(substitute_loop_var_in_expr(e, var_name, value)),
        SensitivityEvent::Level(e) => SensitivityEvent::Level(substitute_loop_var_in_expr(e, var_name, value)),
        SensitivityEvent::Wildcard => SensitivityEvent::Wildcard,
    }
}

fn substitute_loop_var_in_expr(expr: &Expr, var_name: &str, value: i64) -> Expr {
    match expr {
        Expr::Ident(name) if name == var_name => Expr::Value(crate::ast::expr::Value::Decimal(value)),
        Expr::Ident(_) => expr.clone(),
        Expr::Value(_) | Expr::String(_) | Expr::Null => expr.clone(),
        Expr::RangeSelect { expr: inner, msb, lsb } => Expr::RangeSelect {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            msb: Box::new(substitute_loop_var_in_expr(msb, var_name, value)),
            lsb: Box::new(substitute_loop_var_in_expr(lsb, var_name, value)),
        },
        Expr::BitSelect { expr: inner, index } => Expr::BitSelect {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            index: Box::new(substitute_loop_var_in_expr(index, var_name, value)),
        },
        Expr::PartSelect { expr: inner, base, width } => Expr::PartSelect {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            base: Box::new(substitute_loop_var_in_expr(base, var_name, value)),
            width: Box::new(substitute_loop_var_in_expr(width, var_name, value)),
        },
        Expr::Concat(exprs) => Expr::Concat(
            exprs.iter().map(|e| substitute_loop_var_in_expr(e, var_name, value)).collect()
        ),
        Expr::FuncCall { name, args } => Expr::FuncCall {
            name: name.clone(),
            args: args.iter().map(|a| substitute_loop_var_in_expr(a, var_name, value)).collect(),
        },
        Expr::Replicate { count, expr: inner } => Expr::Replicate {
            count: Box::new(substitute_loop_var_in_expr(count, var_name, value)),
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
        },
        Expr::UnaryOp { op, expr: inner } => Expr::UnaryOp {
            op: op.clone(),
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
        },
        Expr::BinaryOp { op, lhs, rhs } => Expr::BinaryOp {
            op: op.clone(),
            lhs: Box::new(substitute_loop_var_in_expr(lhs, var_name, value)),
            rhs: Box::new(substitute_loop_var_in_expr(rhs, var_name, value)),
        },
        Expr::TernaryOp { cond, true_expr, false_expr } => Expr::TernaryOp {
            cond: Box::new(substitute_loop_var_in_expr(cond, var_name, value)),
            true_expr: Box::new(substitute_loop_var_in_expr(true_expr, var_name, value)),
            false_expr: Box::new(substitute_loop_var_in_expr(false_expr, var_name, value)),
        },
        Expr::Paren(inner) => Expr::Paren(Box::new(substitute_loop_var_in_expr(inner, var_name, value))),
        Expr::MethodCall { obj, method, args } => Expr::MethodCall {
            obj: Box::new(substitute_loop_var_in_expr(obj, var_name, value)),
            method: method.clone(),
            args: args.iter().map(|a| substitute_loop_var_in_expr(a, var_name, value)).collect(),
        },
        Expr::MemberAccess { obj, field } => Expr::MemberAccess {
            obj: Box::new(substitute_loop_var_in_expr(obj, var_name, value)),
            field: field.clone(),
        },
        Expr::FillLit(val) => Expr::FillLit(*val),
    }
}

fn infer_comb_sensitivity(body: &[IrStmt]) -> Vec<SignalId> {
    let mut sigs = Vec::new();
    collect_read_signals_stmts(body, &mut sigs);
    sigs.sort();
    sigs.dedup();
    sigs
}

fn collect_read_signals_stmts(stmts: &[IrStmt], out: &mut Vec<SignalId>) {
    for stmt in stmts {
        collect_read_signals_stmt(stmt, out);
    }
}

fn collect_read_signals_stmt(stmt: &IrStmt, out: &mut Vec<SignalId>) {
    match stmt {
        IrStmt::Block { stmts } => collect_read_signals_stmts(stmts, out),
        IrStmt::BlockingAssign { rhs, .. } | IrStmt::NonBlockingAssign { rhs, .. } => {
            collect_read_signals_expr(rhs, out);
        }
        IrStmt::If { cond, true_branch, false_branch } => {
            collect_read_signals_expr(cond, out);
            collect_read_signals_stmts(true_branch, out);
            collect_read_signals_stmts(false_branch, out);
        }
        IrStmt::Case { expr, items, default, .. } => {
            collect_read_signals_expr(expr, out);
            for item in items {
                for label in &item.labels {
                    collect_read_signals_expr(label, out);
                }
                collect_read_signals_stmts(&item.body, out);
            }
            collect_read_signals_stmts(default, out);
        }
        IrStmt::Delay { body, .. } => collect_read_signals_stmts(body, out),
        IrStmt::Wait { cond, body } => {
            collect_read_signals_expr(cond, out);
            collect_read_signals_stmts(body, out);
        }
        IrStmt::SysCall { args, .. } => {
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrStmt::LoopWhile { cond, body } => {
            collect_read_signals_expr(cond, out);
            collect_read_signals_stmts(body, out);
        }
        IrStmt::LoopDoWhile { cond, body } => {
            collect_read_signals_expr(cond, out);
            collect_read_signals_stmts(body, out);
        }
        IrStmt::LoopFor { init, cond, step, body } => {
            if let Some(s) = init {
                collect_read_signals_stmt(s, out);
            }
            collect_read_signals_expr(cond, out);
            if let Some(s) = step {
                collect_read_signals_stmt(s, out);
            }
            collect_read_signals_stmts(body, out);
        }
        IrStmt::EventControl { sig_id, body, .. } => {
            out.push(*sig_id);
            collect_read_signals_stmts(body, out);
        }
        IrStmt::EventTrigger { sig_id } => {
            out.push(*sig_id);
        }
        IrStmt::MethodCallStmt { obj, args, .. } => {
            collect_read_signals_expr(obj, out);
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrStmt::NamedBlock { stmts, .. } => {
            collect_read_signals_stmts(stmts, out);
        }
        IrStmt::Release { .. } | IrStmt::Deassign { .. } => {}
        IrStmt::Disable { .. } => {}
        _ => {}
    }
}

fn collect_read_signals_expr(expr: &IrExpr, out: &mut Vec<SignalId>) {
    match expr {
        IrExpr::Signal(id, _) | IrExpr::RangeSelect(id, ..) | IrExpr::BitSelect(id, _) | IrExpr::ArrayIndex { sig_id: id, .. } => {
            out.push(*id);
        }
        IrExpr::Const(_) | IrExpr::String(_) | IrExpr::FillLit(_) => {}
        IrExpr::Concat(exprs) => {
            for e in exprs {
                collect_read_signals_expr(e, out);
            }
        }
        IrExpr::Replicate(_, inner) => {
            collect_read_signals_expr(inner, out);
        }
        IrExpr::UnaryOp(_, inner) => collect_read_signals_expr(inner, out),
        IrExpr::BinaryOp(_, lhs, rhs) => {
            collect_read_signals_expr(lhs, out);
            collect_read_signals_expr(rhs, out);
        }
        IrExpr::Cond(c, t, f) => {
            collect_read_signals_expr(c, out);
            collect_read_signals_expr(t, out);
            collect_read_signals_expr(f, out);
        }
        IrExpr::Signed(inner) => collect_read_signals_expr(inner, out),
        IrExpr::NewCall { args, .. } => {
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrExpr::This => {}
        IrExpr::SysFunc { args, .. } => {
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrExpr::MethodCall { obj, args, .. } => {
            collect_read_signals_expr(obj, out);
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrExpr::MemberAccess { obj, .. } => {
            collect_read_signals_expr(obj, out);
        }
        IrExpr::ExprRangeSelect(inner, _, _) => {
            collect_read_signals_expr(inner, out);
        }
        IrExpr::ExprBitSelect(inner, _) => {
            collect_read_signals_expr(inner, out);
        }
        IrExpr::ExprPartSelect(inner, base_expr, width_expr) => {
            collect_read_signals_expr(inner, out);
            collect_read_signals_expr(base_expr, out);
            collect_read_signals_expr(width_expr, out);
        }
    }
}

fn resolve_expr_signal(expr: &Expr, signal_map: &HashMap<String, SignalId>) -> Option<SignalId> {
    match expr {
        Expr::Ident(name) => signal_map.get(name).copied(),
        Expr::MethodCall { .. } => None,
        Expr::MemberAccess { .. } => None,
        _ => None,
    }
}

fn collect_sensitivity(expr: &Expr, signal_map: &HashMap<String, SignalId>) -> Vec<SignalId> {
    match expr {
        Expr::Ident(name) => {
            signal_map.get(name).map(|&id| vec![id]).unwrap_or_default()
        }
        Expr::BinaryOp { lhs, rhs, .. } => {
            let mut v = collect_sensitivity(lhs, signal_map);
            v.extend(collect_sensitivity(rhs, signal_map));
            v
        }
        Expr::UnaryOp { expr: inner, .. } => collect_sensitivity(inner, signal_map),
        Expr::Concat(exprs) => {
            exprs.iter().flat_map(|e| collect_sensitivity(e, signal_map)).collect()
        }
        Expr::BitSelect { expr: inner, index } => {
            let mut v = collect_sensitivity(inner, signal_map);
            v.extend(collect_sensitivity(index, signal_map));
            v
        }
        Expr::RangeSelect { expr: inner, msb, lsb } => {
            let mut v = collect_sensitivity(inner, signal_map);
            v.extend(collect_sensitivity(msb, signal_map));
            v.extend(collect_sensitivity(lsb, signal_map));
            v
        }
        Expr::PartSelect { expr: inner, base, width } => {
            let mut v = collect_sensitivity(inner, signal_map);
            v.extend(collect_sensitivity(base, signal_map));
            v.extend(collect_sensitivity(width, signal_map));
            v
        }
        Expr::TernaryOp { cond, true_expr, false_expr } => {
            let mut v = collect_sensitivity(cond, signal_map);
            v.extend(collect_sensitivity(true_expr, signal_map));
            v.extend(collect_sensitivity(false_expr, signal_map));
            v
        }
        Expr::MethodCall { obj, .. } => collect_sensitivity(obj, signal_map),
        Expr::MemberAccess { obj, .. } => collect_sensitivity(obj, signal_map),
        _ => vec![],
    }
}

fn const_eval(expr: &Expr) -> Result<i64, String> {
    const_eval_with_params(expr, &HashMap::new())
}

fn const_eval_params(expr: &Expr, params: &HashMap<String, i64>) -> Result<i64, String> {
    const_eval_with_params(expr, params)
}

fn value_to_logicvec(val: &Value) -> LogicVec {
    match val {
        Value::Decimal(n) => {
            let abs = n.unsigned_abs();
            let width = if *n == 0 { 1 } else { 64 - (abs.leading_zeros() as usize) };
            let mut lv = LogicVec::from_u64(abs, width.max(1));
            if *n < 0 {
                // Two's complement
                for b in lv.bits.iter_mut() {
                    *b = match b {
                        LogicVal::Zero => LogicVal::One,
                        LogicVal::One => LogicVal::Zero,
                        _ => LogicVal::X,
                    };
                }
                // Add 1
                let mut carry = true;
                for b in lv.bits.iter_mut() {
                    if carry {
                        match b {
                            LogicVal::Zero => { *b = LogicVal::One; carry = false; }
                            LogicVal::One => { *b = LogicVal::Zero; }
                            _ => {}
                        }
                    }
                }
            }
            lv
        }
        Value::Binary { bits, width } => {
            let w = width.unwrap_or(bits.len());
            let mut vec = LogicVec::new(w);
            for (i, c) in bits.chars().rev().enumerate() {
                if i >= w { break; }
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
        Value::Hex { bits, width } => {
            let w = width.unwrap_or(bits.len() * 4);
            let mut vec = LogicVec::new(w);
            let digits: String = bits.chars().filter(|c| *c != '_').collect();
            for (i, c) in digits.chars().rev().enumerate() {
                let hex_val = c.to_digit(16).unwrap_or(0);
                for j in 0..4 {
                    let bit_idx = i * 4 + j;
                    if bit_idx >= w { break; }
                    vec.bits[bit_idx] = if (hex_val >> j) & 1 == 1 {
                        LogicVal::One
                    } else {
                        LogicVal::Zero
                    };
                }
            }
            vec
        }
        Value::Octal { bits, width } => {
            let w = width.unwrap_or(bits.len() * 3);
            let mut vec = LogicVec::new(w);
            let digits: String = bits.chars().filter(|c| *c != '_').collect();
            for (i, c) in digits.chars().rev().enumerate() {
                let oct_val = c.to_digit(8).unwrap_or(0);
                for j in 0..3 {
                    let bit_idx = i * 3 + j;
                    if bit_idx >= w { break; }
                    vec.bits[bit_idx] = if (oct_val >> j) & 1 == 1 {
                        LogicVal::One
                    } else {
                        LogicVal::Zero
                    };
                }
            }
            vec
        }
        Value::Real(_) => LogicVec::from_u64(0, 1),
    }
}

fn map_unary_op(op: &UnaryOp) -> Result<UnaryIrOp, String> {
    match op {
        UnaryOp::Plus => Ok(UnaryIrOp::Plus),
        UnaryOp::Minus => Ok(UnaryIrOp::Minus),
        UnaryOp::Not => Ok(UnaryIrOp::Not),
        UnaryOp::BitNot => Ok(UnaryIrOp::BitNot),
        UnaryOp::ReductionAnd => Ok(UnaryIrOp::RedAnd),
        UnaryOp::ReductionNand => Ok(UnaryIrOp::RedNand),
        UnaryOp::ReductionOr => Ok(UnaryIrOp::RedOr),
        UnaryOp::ReductionNor => Ok(UnaryIrOp::RedNor),
        UnaryOp::ReductionXor => Ok(UnaryIrOp::RedXor),
        UnaryOp::ReductionXnor => Ok(UnaryIrOp::RedXnor),
    }
}

fn map_binary_op(op: &BinaryOp) -> Result<BinaryIrOp, String> {
    match op {
        BinaryOp::Add => Ok(BinaryIrOp::Add),
        BinaryOp::Sub => Ok(BinaryIrOp::Sub),
        BinaryOp::Mul => Ok(BinaryIrOp::Mul),
        BinaryOp::Div => Ok(BinaryIrOp::Div),
        BinaryOp::Mod => Ok(BinaryIrOp::Mod),
        BinaryOp::Power => Ok(BinaryIrOp::Power),
        BinaryOp::Eq => Ok(BinaryIrOp::Eq),
        BinaryOp::Neq => Ok(BinaryIrOp::Neq),
        BinaryOp::CaseEq => Ok(BinaryIrOp::CaseEq),
        BinaryOp::CaseNeq => Ok(BinaryIrOp::CaseNeq),
        BinaryOp::EqWild => Ok(BinaryIrOp::EqWild),
        BinaryOp::NeqWild => Ok(BinaryIrOp::NeqWild),
        BinaryOp::Lt => Ok(BinaryIrOp::Lt),
        BinaryOp::Le => Ok(BinaryIrOp::Le),
        BinaryOp::Gt => Ok(BinaryIrOp::Gt),
        BinaryOp::Ge => Ok(BinaryIrOp::Ge),
        BinaryOp::BitAnd => Ok(BinaryIrOp::BitAnd),
        BinaryOp::BitOr => Ok(BinaryIrOp::BitOr),
        BinaryOp::BitXor => Ok(BinaryIrOp::BitXor),
        BinaryOp::BitXnor => Ok(BinaryIrOp::BitXnor),
        BinaryOp::Shl => Ok(BinaryIrOp::Shl),
        BinaryOp::Shr => Ok(BinaryIrOp::Shr),
        BinaryOp::Sshl => Ok(BinaryIrOp::Sshl),
        BinaryOp::Sshr => Ok(BinaryIrOp::Sshr),
        BinaryOp::LogicalAnd => Ok(BinaryIrOp::LogicalAnd),
        BinaryOp::LogicalOr => Ok(BinaryIrOp::LogicalOr),
    }
}

fn build_gate_expr(gate_type: &GateType, inputs: &[IrExpr]) -> IrExpr {
    match gate_type {
        GateType::And => fold_binary(BinaryIrOp::BitAnd, inputs),
        GateType::Or => fold_binary(BinaryIrOp::BitOr, inputs),
        GateType::Nand => IrExpr::UnaryOp(UnaryIrOp::BitNot, Box::new(fold_binary(BinaryIrOp::BitAnd, inputs))),
        GateType::Nor => IrExpr::UnaryOp(UnaryIrOp::BitNot, Box::new(fold_binary(BinaryIrOp::BitOr, inputs))),
        GateType::Xor => fold_binary(BinaryIrOp::BitXor, inputs),
        GateType::Xnor => IrExpr::UnaryOp(UnaryIrOp::BitNot, Box::new(fold_binary(BinaryIrOp::BitXor, inputs))),
        GateType::Buf => inputs[0].clone(),
        GateType::Not => IrExpr::UnaryOp(UnaryIrOp::BitNot, Box::new(inputs[0].clone())),
    }
}

fn fold_binary(op: BinaryIrOp, exprs: &[IrExpr]) -> IrExpr {
    if exprs.is_empty() {
        return IrExpr::Const(LogicVec::from_u64(0, 1));
    }
    let mut result = exprs[0].clone();
    for e in &exprs[1..] {
        result = IrExpr::BinaryOp(op.clone(), Box::new(result), Box::new(e.clone()));
    }
    result
}

impl DataType {
    fn width(&self) -> usize {
        match self {
            DataType::Bit | DataType::Logic => 1,
            DataType::Byte => 8,
            DataType::Shortint => 16,
            DataType::Int | DataType::Integer => 32,
            DataType::Longint => 64,
            DataType::Signed(inner) => inner.width(),
            DataType::UserDefined(_) => 64,
            DataType::EnumType { base: _, members: _ } => 32,
            DataType::StructType { members } => members.iter().map(|m| m.range.as_ref().map(|r| r.width()).unwrap_or(1)).sum(),
            DataType::UnionType { members } => members.iter().map(|m| m.range.as_ref().map(|r| r.width()).unwrap_or(1)).max().unwrap_or(1),
        }
    }
}

impl DeclKind {
    fn default_width(&self) -> usize {
        match self {
            DeclKind::Wire | DeclKind::Reg | DeclKind::Logic => 1,
            DeclKind::Int | DeclKind::Integer => 32,
        }
    }
}
