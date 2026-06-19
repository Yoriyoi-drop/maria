use crate::ir::*;
use crate::simulator::state::SimulationState;
use crate::simulator::value::*;
use crate::waveform::VcdWriter;
use crate::ast::*;
use std::collections::HashMap;
use std::io::Write;
use rand::Rng;

pub enum EventKind {
    EvalProcess(usize),
    ContinueBlock(Continuation),
    NbaCommit,
}

pub struct Continuation {
    pub stmts_to_exec: Vec<IrStmt>,
    pub stmts_remaining: Vec<IrStmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FlowControl {
    Break,
    Continue,
}

pub struct SimulationEngine {
    pub state: SimulationState,
    pub design: IrDesign,
    pub max_time: u64,
    pub running: bool,
    events: Vec<Vec<EventKind>>,
    nba_pending: Vec<(IrLValue, LogicVec)>,
    vcd: Option<VcdWriter>,
    current_this: Option<ObjId>,
    method_locals: Vec<HashMap<String, LogicVec>>,
    current_method: Option<String>,
    rng: rand::rngs::ThreadRng,
    file_handles: HashMap<u32, std::fs::File>,
    next_file_handle: u32,
    monitor_args: Option<Vec<IrExpr>>,
    monitor_last_values: Option<Vec<LogicVec>>,
    disable_pending: Option<String>,
    control_flow: Option<FlowControl>,
}

impl SimulationEngine {
    pub fn new(design: IrDesign, max_time: u64) -> Self {
        let state = SimulationState::new(&design);
        let max_t = max_time as usize + 1;
        SimulationEngine {
            state,
            design,
            max_time,
            running: true,
            events: (0..max_t.max(1000)).map(|_| Vec::new()).collect(),
            nba_pending: Vec::new(),
            vcd: None,
            current_this: None,
            method_locals: Vec::new(),
            current_method: None,
            rng: rand::thread_rng(),
            file_handles: HashMap::new(),
            next_file_handle: 1,
            monitor_args: None,
            monitor_last_values: None,
            disable_pending: None,
            control_flow: None,
        }
    }

    pub fn set_vcd(&mut self, vcd: VcdWriter) {
        self.vcd = Some(vcd);
    }

    pub fn run(&mut self) -> Result<(), String> {
        self.initialize_time_zero()?;
        self.execute_phases()?;

        let mut iter_count = 0u64;
        while self.running && self.state.time <= self.max_time {
            let t = self.state.time as usize;

            self.dump_vcd_time()?;

            if t < self.events.len() {
                let current_events: Vec<EventKind> = self.events[t].drain(..).collect();
                for event in current_events {
                    self.process_event(event, t)?;
                }
            }

            'delta: loop {
                self.commit_nba();
                let changed = self.state.commit_changes();
                if changed.is_empty() { break 'delta; }
                self.trigger_sensitive_processes(&changed, t)?;

                iter_count += 1;
                if iter_count > 1_000_000 {
                    return Err("simulation exceeded max iterations".to_string());
                }
            }

            self.dump_vcd_state()?;
            self.check_monitor()?;

            self.state.time += 1;
            if self.state.time > self.max_time {
                break;
            }

            if self.state.time as usize >= self.events.len() {
                self.events.push(Vec::new());
            }
        }

        Ok(())
    }

    fn dump_vcd_time(&mut self) -> Result<(), String> {
        if let Some(ref mut vcd) = self.vcd {
            vcd.write_time_header(self.state.time)?;
        }
        Ok(())
    }

    fn check_monitor(&mut self) -> Result<(), String> {
        if let Some(ref args) = self.monitor_args.clone() {
            let new_vals: Vec<LogicVec> = args.iter()
                .map(|a| self.evaluate_expr(a).unwrap_or(LogicVec::from_u64(0, 32)))
                .collect();
            let changed = match self.monitor_last_values {
                Some(ref old) => new_vals != *old,
                None => true,
            };
            if changed {
                let msg = format_display(&self.state, args);
                print!("{}", msg);
                self.monitor_last_values = Some(new_vals);
            }
        }
        Ok(())
    }

    fn dump_vcd_state(&mut self) -> Result<(), String> {
        if let Some(ref mut vcd) = self.vcd {
            vcd.dump_state(&self.design, &self.state.signals)?;
        }
        Ok(())
    }

    fn initialize_time_zero(&mut self) -> Result<(), String> {
        let t = 0usize;
        let processes = self.design.top.processes.clone();
        for (pid, process) in processes.iter().enumerate() {
            match process {
                Process::Initial { .. } => {
                    self.events[t].push(EventKind::EvalProcess(pid));
                }
                Process::AlwaysWithDelay { .. } => {
                    self.events[t].push(EventKind::EvalProcess(pid));
                }
                Process::Combinational { .. } => {
                    self.evaluate_combinational(pid, t)?;
                }
                Process::Sequential { .. } => {}
            }
        }
        Ok(())
    }

    fn process_event(&mut self, event: EventKind, t: usize) -> Result<(), String> {
        match event {
            EventKind::EvalProcess(pid) => {
                if pid >= self.design.top.processes.len() {
                    return Ok(());
                }
                let process = self.design.top.processes[pid].clone();
                match &process {
                    Process::Initial { body, .. } => {
                        if self.state.time == 0 {
                            self.disable_pending = None;
                            self.evaluate_block_with_delay(body)?;
                        }
                    }
                    Process::AlwaysWithDelay { delay, body, .. } => {
                        if t < self.events.len() {
                            self.disable_pending = None;
                            self.evaluate_block_with_delay(body)?;
                            let next_t = t + *delay as usize;
                            if next_t < self.events.len() {
                                self.events[next_t].push(EventKind::EvalProcess(pid));
                            }
                        }
                    }
                    _ => {}
                }
            }
            EventKind::ContinueBlock(cont) => {
                if t < self.events.len() {
                    self.evaluate_block_with_delay(&cont.stmts_to_exec)?;
                }
            }
            EventKind::NbaCommit => {
                self.commit_nba();
            }
        }
        Ok(())
    }

    fn evaluate_block_with_delay(
        &mut self, stmts: &[IrStmt]
    ) -> Result<(), String> {
        for (i, stmt) in stmts.iter().enumerate() {
            if self.disable_pending.is_some() { return Ok(()); }
            if self.control_flow.is_some() { return Ok(()); }
            match stmt {
                IrStmt::Block { stmts: inner } => {
                    self.evaluate_block_with_delay(inner)?;
                }
                IrStmt::NamedBlock { name, stmts: inner } => {
                    if self.disable_pending.as_deref() == Some(name) {
                        self.disable_pending = None;
                        return Ok(());
                    }
                    let old = self.disable_pending.take();
                    self.evaluate_block_with_delay(inner)?;
                    if let Some(ref n) = self.disable_pending {
                        if n == name {
                            self.disable_pending = None;
                        }
                    }
                    self.disable_pending = self.disable_pending.take().or(old);
                }
                IrStmt::If { cond, true_branch: then_stmts, false_branch: else_stmts } => {
                    let cond_val = self.evaluate_expr(cond)?;
                    if cond_val.to_bool().unwrap_or(false) {
                        self.evaluate_block_with_delay(then_stmts)?;
                    } else if !else_stmts.is_empty() {
                        self.evaluate_block_with_delay(else_stmts)?;
                    }
                }
                IrStmt::Case { case_type, expr: case_expr, items, default } => {
                    let case_val = self.evaluate_expr(case_expr)?;
                    let mut matched = false;
                    for case_item in items {
                        let mut item_matched = false;
                        for pat in &case_item.labels {
                            let pat_val = self.evaluate_expr(pat)?;
                            let eq = match case_type {
                                CaseType::CaseX => case_val.casex_eq(&pat_val),
                                CaseType::CaseZ => case_val.casez_eq(&pat_val),
                                CaseType::Normal => case_val.eq(&pat_val),
                            };
                            if eq {
                                self.evaluate_block_with_delay(&case_item.body)?;
                                if self.disable_pending.is_some() { return Ok(()); }
                                item_matched = true;
                                matched = true;
                                break;
                            }
                        }
                        if item_matched { break; }
                    }
                    if !matched && !default.is_empty() {
                        self.evaluate_block_with_delay(default)?;
                    }
                }
                IrStmt::BlockingAssign { lhs, rhs, delay: _ } => {
                    let val = self.eval_assign_rhs(rhs, lhs)?;
                    self.write_lvalue(lhs, val)?;
                }
                IrStmt::NonBlockingAssign { lhs, rhs, delay: _ } => {
                    let val = self.eval_assign_rhs(rhs, lhs)?;
                    self.nba_pending.push((lhs.clone(), val));
                }
                IrStmt::Delay { delay, body } => {
                    let delay_t = self.state.time as usize + *delay as usize;
                    if delay_t < self.events.len() {
                        // Schedule the delay body to execute after the delay
                        let mut later: Vec<IrStmt> = body.clone();
                        // Also schedule remaining statements after this delay
                        let remaining: Vec<IrStmt> = stmts[i + 1..].to_vec();
                        later.extend(remaining);
                        if !later.is_empty() {
                            self.events[delay_t].push(
                                EventKind::ContinueBlock(Continuation {
                                    stmts_to_exec: later,
                                    stmts_remaining: vec![],
                                })
                            );
                        }
                    }
                    return Ok(());
                }
                IrStmt::EventControl { sig_id, edge, body } => {
                    let sig_val = self.state.read_signal(*sig_id).clone();
                    let triggered = match edge {
                        Some(ClockEdge::PosEdge(_)) => sig_val.to_bool() == Some(true),
                        Some(ClockEdge::NegEdge(_)) => sig_val.to_bool() == Some(false),
                        None => true,
                    };
                    if triggered {
                        self.evaluate_block_with_delay(body)?;
                        if i + 1 < stmts.len() {
                            self.evaluate_block_with_delay(&stmts[i + 1..])?;
                        }
                    } else {
                        let next_t = self.state.time as usize + 1;
                        if next_t < self.events.len() {
                            let later: Vec<IrStmt> = stmts[i..].to_vec();
                            if !later.is_empty() {
                                self.events[next_t].push(
                                    EventKind::ContinueBlock(Continuation {
                                        stmts_to_exec: later,
                                        stmts_remaining: vec![],
                                    })
                                );
                            }
                        }
                    }
                    return Ok(());
                }
                IrStmt::EventTrigger { sig_id } => {
                    let val = self.state.read_signal(*sig_id);
                    let toggled = if val.to_bool().unwrap_or(false) {
                        LogicVec::from_u64(0, val.width.max(1))
                    } else {
                        LogicVec::from_u64(1, val.width.max(1))
                    };
                    self.state.write_signal(*sig_id, toggled);
                }
                IrStmt::Disable { name } => {
                    self.disable_pending = Some(name.clone());
                    return Ok(());
                }
                IrStmt::Release { lvalue } => {
                    let width = self.get_lvalue_width(lvalue);
                    let x_val = LogicVec { bits: vec![LogicVal::X; width], width };
                    self.write_lvalue(lvalue, x_val)?;
                }
                IrStmt::Deassign { lvalue } => {
                    let width = self.get_lvalue_width(lvalue);
                    let x_val = LogicVec { bits: vec![LogicVal::X; width], width };
                    self.write_lvalue(lvalue, x_val)?;
                }
                IrStmt::Wait { cond, body } => {
                    let cond_val = self.evaluate_expr(cond)?;
                    if cond_val.to_bool().unwrap_or(false) {
                        self.evaluate_block_with_delay(body)?;
                        // Execute remaining statements after wait
                        if i + 1 < stmts.len() {
                            self.evaluate_block_with_delay(&stmts[i + 1..])?;
                        }
                    } else {
                        let next_t = self.state.time as usize + 1;
                        if next_t < self.events.len() {
                            // Schedule everything (Wait + remaining) to be re-checked
                            let later: Vec<IrStmt> = stmts[i..].to_vec();
                            if !later.is_empty() {
                                self.events[next_t].push(
                                    EventKind::ContinueBlock(Continuation {
                                        stmts_to_exec: later,
                                        stmts_remaining: vec![],
                                    })
                                );
                            }
                        }
                    }
                    return Ok(());
                }
                IrStmt::SysCall { name, args: ir_args } => {
                    if name == "display" || name == "write" {
                        let msg = format_display(&self.state, ir_args);
                        print!("{}", msg);
                    } else if name == "monitor" {
                        let vals: Vec<LogicVec> = ir_args.iter()
                            .map(|a| self.evaluate_expr(a).unwrap_or(LogicVec::from_u64(0, 32)))
                            .collect();
                        self.monitor_args = Some(ir_args.clone());
                        self.monitor_last_values = Some(vals);
                    } else if name == "readmemh" {
                        let file = ir_args.first().and_then(|a| if let IrExpr::String(s) = a { Some(s.clone()) } else { None });
                        let sig_id = ir_args.get(1).and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                        if let (Some(file), Some(sig_id)) = (file, sig_id) {
                            let data = read_hex_file(&file, 8, 4096, None, None)?;
                            let elem_width = data.first().map(|d| d.width).unwrap_or(8);
                            let mut all_bits = Vec::new();
                            for d in &data {
                                all_bits.extend(d.bits.iter().cloned());
                            }
                            let packed = LogicVec { bits: all_bits, width: data.len() * elem_width };
                            self.state.write_signal(sig_id, packed);
                        }
                    } else if name == "readmemb" {
                        let file = ir_args.first().and_then(|a| if let IrExpr::String(s) = a { Some(s.clone()) } else { None });
                        let sig_id = ir_args.get(1).and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                        if let (Some(file), Some(sig_id)) = (file, sig_id) {
                            let data = read_bin_file(&file, 8, 4096, None, None)?;
                            let elem_width = data.first().map(|d| d.width).unwrap_or(8);
                            let mut all_bits = Vec::new();
                            for d in &data {
                                all_bits.extend(d.bits.iter().cloned());
                            }
                            let packed = LogicVec { bits: all_bits, width: data.len() * elem_width };
                            self.state.write_signal(sig_id, packed);
                        }
                    } else if name == "random" {
                        let val: i32 = self.rng.gen_range(-(2i32.pow(31))..2i32.pow(31));
                        let sig_id = ir_args.first().and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                        if let Some(sid) = sig_id {
                            self.state.write_signal(sid, LogicVec::from_u64(val as u64, 32));
                        }
                    } else if name == "urandom" {
                        let val: u32 = self.rng.gen();
                        let sig_id = ir_args.first().and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                        if let Some(sid) = sig_id {
                            self.state.write_signal(sid, LogicVec::from_u64(val as u64, 32));
                        }
                    } else if name == "dumpfile" {
                        // $dumpfile sets the VCD file name (handled at elaboration)
                    } else if name == "dumpvars" {
                        // Enable VCD dumping
                        if let Some(ref mut vcd) = self.vcd {
                            vcd.enabled = true;
                        }
                    } else if name == "dumpon" {
                        if let Some(ref mut vcd) = self.vcd {
                            vcd.enabled = true;
                        }
                    } else if name == "dumpoff" {
                        if let Some(ref mut vcd) = self.vcd {
                            vcd.enabled = false;
                        }
                    } else if name == "fopen" {
                        let fname = ir_args.first().and_then(|a| if let IrExpr::String(s) = a { Some(s.clone()) } else { None });
                        if let Some(fname) = fname {
                            match std::fs::File::create(&fname) {
                                Ok(f) => {
                                    let handle = self.next_file_handle;
                                    self.next_file_handle += 1;
                                    self.file_handles.insert(handle, f);
                                    let sig_id = ir_args.get(1).and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                                    if let Some(sid) = sig_id {
                                        self.state.write_signal(sid, LogicVec::from_u64(handle as u64, 32));
                                    }
                                }
                                Err(_) => {
                                    let sig_id = ir_args.get(1).and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                                    if let Some(sid) = sig_id {
                                        self.state.write_signal(sid, LogicVec::from_u64(0, 32));
                                    }
                                }
                            }
                        }
                    } else if name == "fdisplay" {
                        let handle = ir_args.first().and_then(|a| if let IrExpr::Const(c) = a { Some(c.to_u64() as u32) } else { None });
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let msg = format_display(&self.state, &ir_args[1..]);
                                let _ = write!(f, "{}", msg);
                            }
                        }
                    } else if name == "fclose" {
                        let handle = ir_args.first().and_then(|a| if let IrExpr::Const(c) = a { Some(c.to_u64() as u32) } else { None });
                        if let Some(h) = handle {
                            self.file_handles.remove(&h);
                        }
                    } else {
                        eprintln!("warning: unknown system call '{}' ignored", name);
                    }
                }
                IrStmt::SysFinish => {
                    self.running = false;
                    return Ok(());
                }
                IrStmt::Null => {}
                IrStmt::Break => {
                    self.control_flow = Some(FlowControl::Break);
                    return Ok(());
                }
                IrStmt::Continue => {
                    self.control_flow = Some(FlowControl::Continue);
                    return Ok(());
                }
                IrStmt::LoopFor { init, cond, step, body } => {
                    if let Some(init_stmt) = init {
                        self.evaluate_block_with_delay(&[*init_stmt.clone()])?;
                    }
                    loop {
                        if self.disable_pending.is_some() { break; }
                        if self.control_flow.is_some() { self.control_flow = None; break; }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) { break; }
                        self.evaluate_block_with_delay(body)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            if let Some(step_stmt) = step {
                                self.evaluate_block_with_delay(&[*step_stmt.clone()])?;
                            }
                            continue;
                        }
                        if cf == Some(FlowControl::Break) { break; }
                        if self.disable_pending.is_some() { break; }
                        if let Some(step_stmt) = step {
                            self.evaluate_block_with_delay(&[*step_stmt.clone()])?;
                        }
                    }
                }
                IrStmt::LoopWhile { cond, body } => {
                    loop {
                        if self.disable_pending.is_some() { break; }
                        if self.control_flow.is_some() { self.control_flow = None; break; }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) { break; }
                        self.evaluate_block_with_delay(body)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) { continue; }
                        if cf == Some(FlowControl::Break) { break; }
                    }
                }
                IrStmt::LoopDoWhile { cond, body } => {
                    loop {
                        if self.disable_pending.is_some() { break; }
                        if self.control_flow.is_some() { self.control_flow = None; break; }
                        self.evaluate_block_with_delay(body)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) { continue; }
                        if cf == Some(FlowControl::Break) { break; }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) { break; }
                    }
                }
                IrStmt::MethodCallStmt { obj, method, args } => {
                    let obj_val = self.evaluate_expr(obj)?;
                    let obj_id = obj_val.to_u64() as ObjId;
                    let arg_vals: Vec<LogicVec> = args.iter()
                        .map(|a| self.evaluate_expr(a))
                        .collect::<Result<_, _>>()?;
                    self.execute_method(obj_id, method, &arg_vals)?;
                }
            }
        }
        Ok(())
    }

    fn evaluate_stmt_block(&mut self, stmts: &[IrStmt]) -> Result<(), String> {
        for stmt in stmts {
            if self.disable_pending.is_some() { return Ok(()); }
            if self.control_flow.is_some() { return Ok(()); }
            match stmt {
                IrStmt::BlockingAssign { lhs, rhs, delay: _ } => {
                    let val = self.eval_assign_rhs(rhs, lhs)?;
                    self.write_lvalue(lhs, val)?;
                }
                IrStmt::NonBlockingAssign { lhs, rhs, delay: _ } => {
                    let val = self.eval_assign_rhs(rhs, lhs)?;
                    self.nba_pending.push((lhs.clone(), val));
                }
                IrStmt::If { cond, true_branch: then_stmts, false_branch: else_stmts } => {
                    let cond_val = self.evaluate_expr(cond)?;
                    if cond_val.to_bool().unwrap_or(false) {
                        self.evaluate_stmt_block(then_stmts)?;
                    } else if !else_stmts.is_empty() {
                        self.evaluate_stmt_block(else_stmts)?;
                    }
                }
                IrStmt::Block { stmts: inner } => {
                    self.evaluate_stmt_block(inner)?;
                }
                IrStmt::NamedBlock { name, stmts: inner } => {
                    if self.disable_pending.as_deref() == Some(name) {
                        self.disable_pending = None;
                        return Ok(());
                    }
                    let old = self.disable_pending.take();
                    self.evaluate_stmt_block(inner)?;
                    if let Some(ref n) = self.disable_pending {
                        if n == name {
                            self.disable_pending = None;
                        }
                    }
                    self.disable_pending = self.disable_pending.take().or(old);
                }
                IrStmt::SysCall { name, args: ir_args } => {
                    if name == "display" || name == "write" {
                        let msg = format_display(&self.state, ir_args);
                        print!("{}", msg);
                    } else if name == "monitor" {
                        let vals: Vec<LogicVec> = ir_args.iter()
                            .map(|a| self.evaluate_expr(a).unwrap_or(LogicVec::from_u64(0, 32)))
                            .collect();
                        self.monitor_args = Some(ir_args.clone());
                        self.monitor_last_values = Some(vals);
                    } else if name == "urandom" {
                        let val: u32 = self.rng.gen();
                        let sig_id = ir_args.first().and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                        if let Some(sid) = sig_id {
                            self.state.write_signal(sid, LogicVec::from_u64(val as u64, 32));
                        }
                    } else if name == "random" {
                        let val: i32 = self.rng.gen_range(-(2i32.pow(31))..2i32.pow(31));
                        let sig_id = ir_args.first().and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                        if let Some(sid) = sig_id {
                            self.state.write_signal(sid, LogicVec::from_u64(val as u64, 32));
                        }
                    } else if name == "dumpvars" || name == "dumpon" {
                        if let Some(ref mut vcd) = self.vcd {
                            vcd.enabled = true;
                        }
                    } else if name == "dumpoff" {
                        if let Some(ref mut vcd) = self.vcd {
                            vcd.enabled = false;
                        }
                    } else if name == "fopen" {
                        let fname = ir_args.first().and_then(|a| if let IrExpr::String(s) = a { Some(s.clone()) } else { None });
                        if let Some(fname) = fname {
                            match std::fs::File::create(&fname) {
                                Ok(f) => {
                                    let handle = self.next_file_handle;
                                    self.next_file_handle += 1;
                                    self.file_handles.insert(handle, f);
                                    let sig_id = ir_args.get(1).and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                                    if let Some(sid) = sig_id {
                                        self.state.write_signal(sid, LogicVec::from_u64(handle as u64, 32));
                                    }
                                }
                                Err(_) => {
                                    let sig_id = ir_args.get(1).and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                                    if let Some(sid) = sig_id {
                                        self.state.write_signal(sid, LogicVec::from_u64(0, 32));
                                    }
                                }
                            }
                        }
                    } else if name == "fdisplay" {
                        let handle = ir_args.first().and_then(|a| if let IrExpr::Const(c) = a { Some(c.to_u64() as u32) } else { None });
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let msg = format_display(&self.state, &ir_args[1..]);
                                let _ = write!(f, "{}", msg);
                            }
                        }
                    } else if name == "fclose" {
                        let handle = ir_args.first().and_then(|a| if let IrExpr::Const(c) = a { Some(c.to_u64() as u32) } else { None });
                        if let Some(h) = handle {
                            self.file_handles.remove(&h);
                        }
                    } else {
                        eprintln!("warning: unknown system call '{}' ignored", name);
                    }
                }
                IrStmt::SysFinish => {
                    self.running = false;
                    return Ok(());
                }
                IrStmt::Case { case_type, expr: case_expr, items, default } => {
                    let case_val = self.evaluate_expr(case_expr)?;
                    let mut matched = false;
                    for case_item in items {
                        let mut item_matched = false;
                        for pat in &case_item.labels {
                            let pat_val = self.evaluate_expr(pat)?;
                            let eq = match case_type {
                                CaseType::CaseX => case_val.casex_eq(&pat_val),
                                CaseType::CaseZ => case_val.casez_eq(&pat_val),
                                CaseType::Normal => case_val.eq(&pat_val),
                            };
                            if eq {
                                self.evaluate_stmt_block(&case_item.body)?;
                                if self.disable_pending.is_some() { return Ok(()); }
                                item_matched = true;
                                matched = true;
                                break;
                            }
                        }
                        if item_matched { break; }
                    }
                    if !matched && !default.is_empty() {
                        self.evaluate_stmt_block(default)?;
                    }
                }
                IrStmt::Null => {}
                IrStmt::Break => {
                    self.control_flow = Some(FlowControl::Break);
                    return Ok(());
                }
                IrStmt::Continue => {
                    self.control_flow = Some(FlowControl::Continue);
                    return Ok(());
                }
                IrStmt::LoopFor { init, cond, step, body } => {
                    if let Some(init_stmt) = init {
                        self.evaluate_stmt_block(&[*init_stmt.clone()])?;
                    }
                    loop {
                        if self.disable_pending.is_some() { break; }
                        if self.control_flow.is_some() { self.control_flow = None; break; }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) { break; }
                        self.evaluate_stmt_block(body)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            if let Some(step_stmt) = step {
                                self.evaluate_stmt_block(&[*step_stmt.clone()])?;
                            }
                            continue;
                        }
                        if cf == Some(FlowControl::Break) { break; }
                        if self.disable_pending.is_some() { break; }
                        if let Some(step_stmt) = step {
                            self.evaluate_stmt_block(&[*step_stmt.clone()])?;
                        }
                    }
                }
                IrStmt::LoopWhile { cond, body } => {
                    loop {
                        if self.disable_pending.is_some() { break; }
                        if self.control_flow.is_some() { self.control_flow = None; break; }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) { break; }
                        self.evaluate_stmt_block(body)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) { continue; }
                        if cf == Some(FlowControl::Break) { break; }
                    }
                }
                IrStmt::LoopDoWhile { cond, body } => {
                    loop {
                        if self.disable_pending.is_some() { break; }
                        if self.control_flow.is_some() { self.control_flow = None; break; }
                        self.evaluate_stmt_block(body)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) { continue; }
                        if cf == Some(FlowControl::Break) { break; }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) { break; }
                    }
                }
                IrStmt::MethodCallStmt { obj, method, args } => {
                    let obj_val = self.evaluate_expr(obj)?;
                    let obj_id = obj_val.to_u64() as ObjId;
                    let arg_vals: Vec<LogicVec> = args.iter()
                        .map(|a| self.evaluate_expr(a))
                        .collect::<Result<_, _>>()?;
                    self.execute_method(obj_id, method, &arg_vals)?;
                }
                IrStmt::Delay { delay, body } => {
                    let t = self.state.time as usize + *delay as usize;
                    if t < self.events.len() {
                        self.events[t].push(EventKind::ContinueBlock(Continuation {
                            stmts_to_exec: body.clone(),
                            stmts_remaining: vec![],
                        }));
                    }
                }
                IrStmt::EventControl { sig_id, edge, body } => {
                    let sig_val = self.state.read_signal(*sig_id).clone();
                    let triggered = match edge {
                        Some(ClockEdge::PosEdge(_)) => sig_val.to_bool() == Some(true),
                        Some(ClockEdge::NegEdge(_)) => sig_val.to_bool() == Some(false),
                        None => true,
                    };
                    if triggered {
                        self.evaluate_stmt_block(body)?;
                    }
                }
                IrStmt::EventTrigger { sig_id } => {
                    let val = self.state.read_signal(*sig_id);
                    let toggled = if val.to_bool().unwrap_or(false) {
                        LogicVec::from_u64(0, val.width.max(1))
                    } else {
                        LogicVec::from_u64(1, val.width.max(1))
                    };
                    self.state.write_signal(*sig_id, toggled);
                }
                IrStmt::Wait { cond, body } => {
                    let cond_val = self.evaluate_expr(cond)?;
                    if cond_val.to_bool().unwrap_or(false) {
                        self.evaluate_stmt_block(body)?;
                    }
                }
                IrStmt::Disable { name } => {
                    self.disable_pending = Some(name.clone());
                    return Ok(());
                }
                IrStmt::Release { lvalue } => {
                    let width = self.get_lvalue_width(lvalue);
                    let x_val = LogicVec { bits: vec![LogicVal::X; width], width };
                    self.write_lvalue(lvalue, x_val)?;
                }
                IrStmt::Deassign { lvalue } => {
                    let width = self.get_lvalue_width(lvalue);
                    let x_val = LogicVec { bits: vec![LogicVal::X; width], width };
                    self.write_lvalue(lvalue, x_val)?;
                }
            }
        }
        Ok(())
    }

    fn trigger_sensitive_processes(&mut self, changed: &[(usize, LogicVec, LogicVec)], _t: usize) -> Result<(), String> {
        let processes = self.design.top.processes.clone();
        for (_pid, process) in processes.iter().enumerate() {
            match process {
                Process::Combinational { sensitivity, body, .. } => {
                    let should_trigger = sensitivity.is_empty()
                        || changed.iter().any(|(id, _, _)| sensitivity.contains(id));
                    if should_trigger {
                        self.evaluate_stmt_block(body)?;
                    }
                }
                Process::Sequential { clock, reset: _reset, body, .. } => {
                    let trigger = match clock {
                        ClockEdge::PosEdge(sig_id) => {
                            changed.iter().any(|(id, old, new)| {
                                id == sig_id
                                    && old.to_bool() != Some(true)
                                    && new.to_bool() == Some(true)
                            })
                        }
                        ClockEdge::NegEdge(sig_id) => {
                            changed.iter().any(|(id, old, new)| {
                                id == sig_id
                                    && old.to_bool() != Some(false)
                                    && new.to_bool() == Some(false)
                            })
                        }
                    };
                    if trigger {
                        self.evaluate_stmt_block(body)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn commit_nba(&mut self) {
        let pending = std::mem::take(&mut self.nba_pending);
        for (lvalue, val) in pending {
            let _ = self.write_lvalue(&lvalue, val);
        }
    }

    fn eval_assign_rhs(&mut self, expr: &IrExpr, lhs: &IrLValue) -> Result<LogicVec, String> {
        if let IrExpr::FillLit(v) = expr {
            let w = self.get_lvalue_width(lhs);
            Ok(LogicVec::fill(*v, w))
        } else {
            self.evaluate_expr(expr)
        }
    }

    fn evaluate_expr(&mut self, expr: &IrExpr) -> Result<LogicVec, String> {
        match expr {
            IrExpr::Const(val) => Ok(val.clone()),
            IrExpr::FillLit(val) => Ok(LogicVec::fill(*val, 1)),
            IrExpr::Signal(id, _) => {
                Ok(self.state.read_signal(*id).clone())
            }
            IrExpr::RangeSelect(sig_id, msb, lsb) => {
                let val = self.state.read_signal(*sig_id);
                let (start, end) = if *msb > *lsb { (*lsb, *msb) } else { (*msb, *lsb) };
                let mut bits = val.bits[start..=end].to_vec();
                if *msb > *lsb { bits.reverse(); }
                Ok(LogicVec { width: bits.len(), bits })
            }
            IrExpr::BitSelect(sig_id, idx) => {
                let val = self.state.read_signal(*sig_id);
                let bit = val.bits.get(*idx).copied().unwrap_or(LogicVal::X);
                Ok(LogicVec { bits: vec![bit], width: 1 })
            }
            IrExpr::ExprRangeSelect(inner, msb, lsb) => {
                let val = self.evaluate_expr(inner)?;
                let (start, end) = if *msb > *lsb { (*lsb, *msb) } else { (*msb, *lsb) };
                if end >= val.width {
                    return Err(format!("range select out of bounds: {}:{} on width {}", msb, lsb, val.width));
                }
                let mut bits = val.bits[start..=end].to_vec();
                if *msb > *lsb { bits.reverse(); }
                Ok(LogicVec { width: bits.len(), bits })
            }
            IrExpr::ExprBitSelect(inner, idx) => {
                let val = self.evaluate_expr(inner)?;
                let bit = val.bits.get(*idx).copied().unwrap_or(LogicVal::X);
                Ok(LogicVec { bits: vec![bit], width: 1 })
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
                Ok(LogicVec { width: bits.len(), bits })
            }
            IrExpr::ArrayIndex { sig_id, index, elem_width } => {
                let array_val = self.state.read_signal(*sig_id).clone();
                let idx_val = self.evaluate_expr(index)?;
                let idx = idx_val.to_u64() as usize;
                let start = idx * elem_width;
                let end = start + elem_width - 1;
                let mut bits = Vec::with_capacity(*elem_width);
                for i in start..=end {
                    bits.push(array_val.bits.get(i).copied().unwrap_or(LogicVal::X));
                }
                Ok(LogicVec { width: *elem_width, bits })
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
                Ok(eval_unary(op.clone(), &val))
            }
            IrExpr::BinaryOp(op, lhs, rhs) => {
                let lval = self.evaluate_expr(lhs)?;
                let rval = self.evaluate_expr(rhs)?;
                Ok(eval_binary(op.clone(), &lval, &rval))
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
                        bits.push(if (byte >> i) & 1 == 1 { LogicVal::One } else { LogicVal::Zero });
                    }
                }
                Ok(LogicVec { width: bits.len(), bits })
            }
            IrExpr::SysFunc { name, args } => {
                match name.as_str() {
                    "$random" => {
                        let val: i32 = self.rng.gen_range(-(2i32.pow(31))..2i32.pow(31));
                        Ok(LogicVec::from_u64(val as u64, 32))
                    }
                    "$urandom" => {
                        let val: u32 = self.rng.gen();
                        Ok(LogicVec::from_u64(val as u64, 32))
                    }
                    "$fopen" => {
                        let fname = args.first().and_then(|a| {
                            if let IrExpr::String(s) = a { Some(s.clone()) }
                            else { None }
                        });
                        if let Some(fname) = fname {
                            match std::fs::File::create(&fname) {
                                Ok(f) => {
                                    let handle = self.next_file_handle;
                                    self.next_file_handle += 1;
                                    self.file_handles.insert(handle, f);
                                    Ok(LogicVec::from_u64(handle as u64, 32))
                                }
                                Err(_) => Ok(LogicVec::from_u64(0, 32))
                            }
                        } else {
                            Ok(LogicVec::from_u64(0, 32))
                        }
                    }
                    "$fdisplay" => {
                        let handle = args.first().and_then(|a| {
                            if let IrExpr::Const(c) = a { Some(c.to_u64() as u32) }
                            else { None }
                        });
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let msg = format_display(&self.state, &args[1..]);
                                let _ = write!(f, "{}", msg);
                            }
                        }
                        Ok(LogicVec::from_u64(0, 1))
                    }
                    "$fclose" => {
                        let handle = args.first().and_then(|a| {
                            if let IrExpr::Const(c) = a { Some(c.to_u64() as u32) }
                            else { None }
                        });
                        if let Some(h) = handle {
                            self.file_handles.remove(&h);
                        }
                        Ok(LogicVec::from_u64(0, 1))
                    }
                    "$clog2" => {
                        if let Some(arg) = args.first() {
                            let val = self.evaluate_expr(arg)?;
                            let n = val.to_u64();
                            if n <= 1 { Ok(LogicVec::from_u64(0, 32)) }
                            else {
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
                    "$time" => {
                        Ok(LogicVec::from_u64(self.state.time as u64, 64))
                    }
                    _ => {
                        eprintln!("warning: unsupported system function '{}'", name);
                        Ok(LogicVec::from_u64(0, 32))
                    }
                }
            }
            IrExpr::NewCall { class_name, args } => {
                let arg_vals: Vec<LogicVec> = args.iter()
                    .map(|a| self.evaluate_expr(a))
                    .collect::<Result<_, _>>()?;
                let obj_id = self.state.alloc_object(&class_name);
                if !class_name.is_empty() {
                    if let Some(cls) = self.design.classes.get(class_name.as_str()) {
                        if let Some(obj) = self.state.get_object_mut(obj_id) {
                            for field in &cls.fields {
                                obj.fields.entry(field.name.clone()).or_insert_with(|| LogicVec::from_u64(0, field.width));
                            }
                        }
                    }
                    if self.find_method_in_hierarchy(&class_name, "new").is_ok() {
                        self.execute_method(obj_id, "new", &arg_vals)?;
                    }
                }
                Ok(LogicVec::from_u64(obj_id as u64, 64))
            }
            IrExpr::This => {
                if let Some(obj_id) = self.current_this {
                    Ok(LogicVec::from_u64(obj_id as u64, 64))
                } else {
                    Err("'this' used outside of class method".to_string())
                }
            }
            IrExpr::MethodCall { obj, method, args } => {
                if let IrExpr::String(s) = obj.as_ref() {
                    let arg_vals: Vec<LogicVec> = args.iter()
                        .map(|a| self.evaluate_expr(a))
                        .collect::<Result<_, _>>()?;
                    let result = evaluate_string_method(s, method, &arg_vals)?;
                    return Ok(result);
                }
                let obj_val = self.evaluate_expr(obj)?;
                let obj_id = obj_val.to_u64() as ObjId;
                let arg_vals: Vec<LogicVec> = args.iter()
                    .map(|a| self.evaluate_expr(a))
                    .collect::<Result<_, _>>()?;
                let result = self.execute_method(obj_id, method, &arg_vals)?;
                Ok(result)
            }
            IrExpr::MemberAccess { obj, field } => {
                let obj_val = self.evaluate_expr(obj)?;
                let obj_id = obj_val.to_u64() as ObjId;
                let obj_data = self.state.get_object(obj_id)
                    .ok_or_else(|| format!("object {} not found", obj_id))?;
                let val = obj_data.fields.get(field)
                    .cloned()
                    .unwrap_or_else(|| LogicVec::new(1));
                Ok(val)
            }
        }
    }

    fn write_lvalue(&mut self, lvalue: &IrLValue, val: LogicVec) -> Result<(), String> {
        match lvalue {
            IrLValue::Signal(id, _) => {
                let target_width = self.state.read_signal(*id).width;
                let resized = if val.width != target_width {
                    val.resize(target_width)
                } else {
                    val
                };
                self.state.write_signal(*id, resized);
            }
            IrLValue::RangeSelect(sig_id, msb, lsb) => {
                let mut existing = self.state.read_signal(*sig_id).clone();
                let (start, end) = if *msb > *lsb { (*lsb, *msb) } else { (*msb, *lsb) };
                for (i, b) in val.bits.iter().enumerate() {
                    if start + i <= end {
                        existing.bits[start + i] = *b;
                    }
                }
                self.state.write_signal(*sig_id, existing);
            }
            IrLValue::BitSelect(sig_id, idx) => {
                let mut existing = self.state.read_signal(*sig_id).clone();
                if let Some(b) = val.bits.first() {
                    if *idx < existing.bits.len() {
                        existing.bits[*idx] = *b;
                    }
                }
                self.state.write_signal(*sig_id, existing);
            }
            IrLValue::ArrayIndex { sig_id, index, elem_width } => {
                let mut existing = self.state.read_signal(*sig_id).clone();
                let idx_val = self.evaluate_expr(index)?;
                let idx = idx_val.to_u64() as usize;
                let start = idx * elem_width;
                for (i, b) in val.bits.iter().enumerate() {
                    if start + i < existing.bits.len() {
                        existing.bits[start + i] = *b;
                    }
                }
                self.state.write_signal(*sig_id, existing);
            }
            IrLValue::ArrayRangeSelect { sig_id, index, elem_width, msb, lsb } => {
                let mut existing = self.state.read_signal(*sig_id).clone();
                let idx_val = self.evaluate_expr(index)?;
                let idx = idx_val.to_u64() as usize;
                let base = idx * elem_width;
                let (start, end) = if *msb > *lsb { (*lsb, *msb) } else { (*msb, *lsb) };
                let abs_start = base + start;
                for (i, b) in val.bits.iter().enumerate() {
                    if abs_start + i <= base + end {
                        existing.bits[abs_start + i] = *b;
                    }
                }
                self.state.write_signal(*sig_id, existing);
            }
            IrLValue::ArrayBitSelect { sig_id, index, elem_width, bit } => {
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

    fn evaluate_combinational(&mut self, pid: usize, _t: usize) -> Result<(), String> {
        let body = self.design.top.processes.get(pid).map(|p| match p {
            Process::Combinational { body, .. } => body.clone(),
            _ => vec![],
        }).unwrap_or_default();
        if !body.is_empty() {
            self.evaluate_stmt_block(&body)?;
        }
        Ok(())
    }

    fn get_lvalue_width(&self, lvalue: &IrLValue) -> usize {
        match lvalue {
            IrLValue::Signal(id, _) => self.state.read_signal(*id).width,
            IrLValue::RangeSelect(_, msb, lsb) => {
                if *msb > *lsb { msb - lsb + 1 } else { lsb - msb + 1 }
            }
            IrLValue::BitSelect(_, _) => 1,
            IrLValue::ArrayIndex { elem_width, .. } => *elem_width,
            IrLValue::ArrayRangeSelect { msb, lsb, .. } => {
                if *msb > *lsb { msb - lsb + 1 } else { lsb - msb + 1 }
            }
            IrLValue::ArrayBitSelect { .. } => 1,
            IrLValue::Concat(parts) => parts.iter().map(|p| self.get_lvalue_width(p)).sum(),
        }
    }

    fn get_local(&self, name: &str) -> Option<LogicVec> {
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

    fn write_local_or_field(&mut self, name: &str, val: LogicVec) -> Result<(), String> {
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
        Err(format!("cannot resolve '{}' in method context (not a local or field)", name))
    }

    fn evaluate_ast_expr(&mut self, expr: &Expr) -> Result<LogicVec, String> {
        match expr {
            Expr::Value(v) => {
                match v {
                    Value::Decimal(i) => Ok(LogicVec::from_u64(*i as u64, 32)),
                    Value::Binary { bits, width: _ } => LogicVec::from_bin(bits),
                    Value::Hex { bits, width: _ } => LogicVec::from_hex(bits),
                    Value::Octal { bits, width: _ } => LogicVec::from_hex(bits),
                    Value::Real(r) => Ok(LogicVec::from_u64(*r as u64, 64)),
                }
            }
            Expr::Ident(name) => {
                if name == "this" {
                    if let Some(obj_id) = self.current_this {
                        return Ok(LogicVec::from_u64(obj_id as u64, 64));
                    } else {
                        return Err("'this' used outside of class method".to_string());
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
                let ctx = self.current_this.map(|id| format!("obj_id={}", id)).unwrap_or_else(|| "no current_this".to_string());
                Err(format!("cannot resolve identifier '{}' in method context ({})", name, ctx))
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
            Expr::TernaryOp { cond, true_expr, false_expr } => {
                let cval = self.evaluate_ast_expr(cond)?;
                if cval.to_bool().unwrap_or(false) {
                    self.evaluate_ast_expr(true_expr)
                } else {
                    self.evaluate_ast_expr(false_expr)
                }
            }
            Expr::FuncCall { name, args } if name == "new" => {
                let _arg_vals: Vec<LogicVec> = args.iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                let obj_id = self.state.alloc_object("");
                Ok(LogicVec::from_u64(obj_id as u64, 64))
            }
            Expr::FuncCall { name, args } => {
                let _arg_vals: Vec<LogicVec> = args.iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                if name == "$clog2" {
                    if let Some(arg) = args.first() {
                        let val = self.evaluate_ast_expr(arg)?;
                        let n = val.to_u64();
                        if n <= 1 { return Ok(LogicVec::from_u64(0, 32)); }
                        let msb = (64 - n.leading_zeros()) as u64;
                        let result = if n.is_power_of_two() { msb - 1 } else { msb };
                        return Ok(LogicVec::from_u64(result, 32));
                    }
                }
                Err(format!("unknown function '{}' in method context", name))
            }
            Expr::MethodCall { obj, method, args } => {
                if let Expr::Ident(s) = obj.as_ref() {
                    if s == "super" {
                        let arg_vals: Vec<LogicVec> = args.iter()
                            .map(|a| self.evaluate_ast_expr(a))
                            .collect::<Result<_, _>>()?;
                        return self.execute_super_method(method, &arg_vals);
                    }
                }
                let obj_val = self.evaluate_ast_expr(obj)?;
                let obj_id = obj_val.to_u64() as ObjId;
                let arg_vals: Vec<LogicVec> = args.iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                self.execute_method(obj_id, method, &arg_vals)
            }
            Expr::MemberAccess { obj, field } => {
                let obj_val = self.evaluate_ast_expr(obj)?;
                let obj_id = obj_val.to_u64() as ObjId;
                let obj_data = self.state.get_object(obj_id)
                    .ok_or_else(|| format!("object {} not found", obj_id))?;
                Ok(obj_data.fields.get(field)
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
                    Ok(LogicVec { width: bits.len(), bits })
                } else {
                    let bit = val.bits.get(i).copied().unwrap_or(LogicVal::X);
                    Ok(LogicVec { bits: vec![bit], width: 1 })
                }
            }
            Expr::RangeSelect { expr: inner, msb, lsb } => {
                let val = self.evaluate_ast_expr(inner)?;
                let msb_val = self.evaluate_ast_expr(msb)?;
                let lsb_val = self.evaluate_ast_expr(lsb)?;
                let m = msb_val.to_u64() as usize;
                let l = lsb_val.to_u64() as usize;
                let (start, end) = if m > l { (l, m) } else { (m, l) };
                let mut bits = val.bits[start..=end].to_vec();
                if m > l { bits.reverse(); }
                Ok(LogicVec { width: bits.len(), bits })
            }
            Expr::PartSelect { expr: inner, base, width } => {
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
                    Err(format!("part-select out of range"))
                }
            }
            Expr::Paren(inner) => self.evaluate_ast_expr(inner),
            Expr::String(s) => {
                let mut bits = Vec::with_capacity(s.len() * 8);
                for c in s.chars() {
                    let byte = c as u8;
                    for i in 0..8 {
                        bits.push(if (byte >> i) & 1 == 1 { LogicVal::One } else { LogicVal::Zero });
                    }
                }
                Ok(LogicVec { width: bits.len(), bits })
            }
            Expr::Null => Ok(LogicVec::from_u64(0, 64)),
            Expr::FillLit(v) => Ok(LogicVec::fill(*v, 1)),
        }
    }

    fn find_signal(&self, name: &str) -> Option<usize> {
        self.design.top.signals.iter().position(|s| s.name == name)
    }

    fn evaluate_ast_stmt(&mut self, stmt: &Stmt) -> Result<(), String> {
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
                    Expr::Ident(name) => {
                        self.write_local_or_field(name, val)
                    }
                    Expr::MemberAccess { obj, field } => {
                        let obj_val = self.evaluate_ast_expr(obj)?;
                        let obj_id = obj_val.to_u64() as ObjId;
                        if let Some(obj) = self.state.get_object_mut(obj_id) {
                            obj.fields.insert(field.clone(), val);
                            Ok(())
                        } else {
                            Err(format!("object {} not found for field write", obj_id))
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
                            let new_val = LogicVec { width: bits.len(), bits };
                            match inner.as_ref() {
                                Expr::Ident(name) => { self.write_local_or_field(name, new_val)?; }
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
                                Expr::Ident(name) => { self.write_local_or_field(name, new_val)?; }
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
                    Expr::RangeSelect { expr: inner, msb, lsb } => {
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
                        let new_val = LogicVec { width: bits.len(), bits };
                        match inner.as_ref() {
                            Expr::Ident(name) => { self.write_local_or_field(name, new_val)?; }
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
                    _ => Err(format!("unsupported LHS in method: {:?}", lhs)),
                }
            }
            Stmt::NonBlockingAssign { lhs, rhs, delay: _ } => {
                let val = self.evaluate_ast_expr(rhs)?;
                match lhs {
                    Expr::Ident(name) => {
                        self.write_local_or_field(name, val)
                    }
                    Expr::MemberAccess { obj, field } => {
                        let obj_val = self.evaluate_ast_expr(obj)?;
                        let obj_id = obj_val.to_u64() as ObjId;
                        if let Some(obj) = self.state.get_object_mut(obj_id) {
                            obj.fields.insert(field.clone(), val);
                            Ok(())
                        } else {
                            Err(format!("object {} not found for field write", obj_id))
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
                            let new_val = LogicVec { width: bits.len(), bits: bits.clone() };
                            match inner.as_ref() {
                                Expr::Ident(name) => { self.write_local_or_field(name, new_val)?; }
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
                                Expr::Ident(name) => { self.write_local_or_field(name, new_val)?; }
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
                    Expr::RangeSelect { expr: inner, msb, lsb } => {
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
                        let new_val = LogicVec { width: bits.len(), bits };
                        match inner.as_ref() {
                            Expr::Ident(name) => { self.write_local_or_field(name, new_val)?; }
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
                    _ => Err(format!("unsupported LHS in method: {:?}", lhs)),
                }
            }
            Stmt::IfElse { cond, true_branch, false_branch } => {
                let cval = self.evaluate_ast_expr(cond)?;
                if cval.to_bool().unwrap_or(false) {
                    self.evaluate_ast_stmt(true_branch)
                } else if let Some(f) = false_branch {
                    self.evaluate_ast_stmt(f)
                } else {
                    Ok(())
                }
            }
            Stmt::Case { expr, items, default } => {
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
                    if matched { break; }
                }
                if !matched {
                    if let Some(default_body) = default {
                        self.evaluate_ast_stmt(default_body)?;
                    }
                }
                Ok(())
            }
            Stmt::CaseX { expr, items, default } => {
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
                    if matched { break; }
                }
                if !matched {
                    if let Some(default_body) = default {
                        self.evaluate_ast_stmt(default_body)?;
                    }
                }
                Ok(())
            }
            Stmt::CaseZ { expr, items, default } => {
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
                    if matched { break; }
                }
                if !matched {
                    if let Some(default_body) = default {
                        self.evaluate_ast_stmt(default_body)?;
                    }
                }
                Ok(())
            }
            Stmt::StmtCase { expr, items, default } => {
                self.evaluate_ast_stmt(&Stmt::Case { expr: expr.clone(), items: items.clone(), default: default.clone() })
            }
            Stmt::LoopFor { init, cond, step, stmts } => {
                if let Some(init_stmt) = init {
                    self.evaluate_ast_stmt(init_stmt)?;
                }
                while self.disable_pending.is_none() && cond.as_ref().map_or(true, |c| self.evaluate_ast_expr(c).ok()
                    .map(|v| v.to_bool().unwrap_or(false))
                    .unwrap_or(false))
                {
                    for s in stmts {
                        self.evaluate_ast_stmt(s)?;
                        if self.disable_pending.is_some() { break; }
                    }
                    if self.disable_pending.is_some() { break; }
                    if let Some(step_stmt) = step {
                        self.evaluate_ast_stmt(step_stmt)?;
                    }
                }
                Ok(())
            }
            Stmt::LoopWhile { cond, stmts } => {
                while self.disable_pending.is_none() && self.evaluate_ast_expr(cond).ok()
                    .map(|v| v.to_bool().unwrap_or(false))
                    .unwrap_or(false)
                {
                    for s in stmts {
                        self.evaluate_ast_stmt(s)?;
                        if self.disable_pending.is_some() { break; }
                    }
                }
                Ok(())
            }
            Stmt::LoopForever { stmts } => {
                for _ in 0..1_000_000 {
                    if self.disable_pending.is_some() { break; }
                    for s in stmts {
                        self.evaluate_ast_stmt(s)?;
                        if self.disable_pending.is_some() { break; }
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
            Stmt::ForeachLoop { array_var, index_var, stmts } => {
                let count = self.get_foreach_count(array_var);
                for i in 0..count {
                    let idx_val = LogicVec::from_u64(i as u64, 32);
                    // Push scope with index variable
                    let mut scope = HashMap::new();
                    scope.insert(index_var.clone(), idx_val);
                    let old_locals = self.method_locals.clone();
                    self.method_locals.push(scope);
                    for s in stmts {
                        self.evaluate_ast_stmt(s)?;
                    }
                    self.method_locals = old_locals;
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
                    Expr::Ident(name) => {
                        self.write_local_or_field(name, val)
                    }
                    Expr::MemberAccess { obj, field } => {
                        let obj_val = self.evaluate_ast_expr(obj)?;
                        let obj_id = obj_val.to_u64() as ObjId;
                        if let Some(obj) = self.state.get_object_mut(obj_id) {
                            obj.fields.insert(field.clone(), val);
                            Ok(())
                        } else {
                            Err(format!("object {} not found for field write", obj_id))
                        }
                    }
                    _ => Err(format!("unsupported LHS in StmtAssign: {:?}", lhs)),
                }
            }
            _ => Err(format!("unsupported statement in method context: {:?}", stmt)),
        }
    }

    fn find_phase_class_name(&self) -> Option<String> {
        let phase_methods = ["build_phase", "connect_phase", "run_phase"];
        let mut best: Option<(String, usize)> = None;
        for (name, cls) in &self.design.classes {
            let count = phase_methods.iter()
                .filter(|pm| cls.methods.iter().any(|m| &m.name == *pm))
                .count();
            if count > 0 && best.as_ref().map_or(true, |b| count > b.1) {
                best = Some((name.clone(), count));
            }
        }
        best.map(|(name, _)| name)
    }

    fn execute_phases(&mut self) -> Result<(), String> {
        let class_name = match self.find_phase_class_name() {
            Some(c) => c,
            None => return Ok(()),
        };
        for phase in &["build_phase", "connect_phase", "run_phase"] {
            self.run_phase_method(&class_name, phase)?;
        }
        Ok(())
    }

    fn run_phase_method(&mut self, class_name: &str, phase: &str) -> Result<(), String> {
        if !self.design.classes.contains_key(class_name) {
            return Ok(());
        }
        if self.find_method_in_hierarchy(class_name, phase).is_err() {
            return Ok(());
        }
        let obj_id = self.state.alloc_object(class_name);
        self.current_this = Some(obj_id);
        let arg_vals = vec![];
        self.execute_method(obj_id, phase, &arg_vals)?;
        self.current_this = None;
        Ok(())
    }

    fn execute_method(&mut self, obj_id: ObjId, method: &str,
                      args: &[LogicVec]) -> Result<LogicVec, String>
    {
        let class_name = self.state.get_object(obj_id)
            .map(|o| o.class_name.clone())
            .unwrap_or_default();
        if class_name.is_empty() {
            return Err(format!("cannot call method '{}' on object with unknown class", method));
        }
        // Normal dispatch: find method in the full class hierarchy (virtual dispatch)
        let method_def = self.find_method_in_hierarchy(&class_name, method)?.clone();
        self.execute_method_body(Some(obj_id), &method_def, args, method)
    }

    fn execute_super_method(&mut self, method: &str, args: &[LogicVec]) -> Result<LogicVec, String> {
        let obj_id = self.current_this
            .ok_or_else(|| "'super' used outside class method".to_string())?;
        let class_name = self.state.get_object(obj_id)
            .map(|o| o.class_name.clone())
            .unwrap_or_default();
        let parent = self.design.classes.get(&class_name)
            .and_then(|c| c.extends.clone())
            .ok_or_else(|| format!("class '{}' has no parent for super call", class_name))?;
        // Super dispatch: start search from parent class, skipping current class override
        let method_def = self.find_method_in_hierarchy(&parent, method)?.clone();
        self.execute_method_body(Some(obj_id), &method_def, args, method)
    }

    fn execute_method_body(&mut self, obj_id: Option<ObjId>, method_def: &IrClassMethod,
                           args: &[LogicVec], method: &str) -> Result<LogicVec, String>
    {
        let old_this = self.current_this;
        if let Some(oid) = obj_id {
            self.current_this = Some(oid);
        }

        let mut local_signals: HashMap<String, LogicVec> = HashMap::new();
        for (i, port) in method_def.ports.iter().enumerate() {
            let port_width = port.resolved_width(&HashMap::new()).unwrap_or(1);
            let val = if i < args.len() {
                args[i].resize(port_width)
            } else {
                LogicVec::new(port_width)
            };
            local_signals.insert(port.name.clone(), val);
        }

        for decl in &method_def.decls {
            for dv in &decl.names {
                let w = dv.resolved_width(&HashMap::new()).unwrap_or(1);
                local_signals.insert(dv.name.clone(), LogicVec::new(w));
            }
        }

        let old_locals = self.method_locals.clone();
        self.method_locals.push(local_signals);

        let old_method = self.current_method.clone();
        self.current_method = Some(method.to_string());

        if !method_def.stmts.is_empty() {
            let body = Stmt::Block { stmts: method_def.stmts.clone() };
            self.evaluate_ast_stmt(&body)?;
        }

        let return_val = self.get_local(method)
            .unwrap_or_else(|| LogicVec::new(1));

        self.current_method = old_method;
        self.method_locals = old_locals;
        self.current_this = old_this;
        Ok(return_val)
    }

fn get_foreach_count(&self, array_var: &str) -> usize {
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

fn get_field_elem_width(&self, expr: &Expr) -> Option<usize> {
    match expr {
        Expr::Ident(name) => {
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
            if let Expr::Ident(s) = obj.as_ref() {
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

    fn find_method_in_hierarchy(&self, class_name: &str, method: &str)
        -> Result<IrClassMethod, String>
    {
        let mut current = class_name;
        loop {
            if let Some(cls) = self.design.classes.get(current) {
                if let Some(m) = cls.methods.iter().find(|m| m.name == method) {
                    return Ok(m.clone());
                }
                if let Some(parent) = &cls.extends {
                    current = parent;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        Err(format!("method '{}' not found in class '{}' or its parents", method, class_name))
    }
}

fn format_display(state: &SimulationState, ir_args: &[IrExpr]) -> String {
    let (fmt_str, start_idx) = if let Some(IrExpr::String(s)) = ir_args.first() {
        (s.clone(), 1)
    } else {
        let mut parts = Vec::new();
        for arg in ir_args {
            if let Ok(val) = eval_display_arg(state, arg) {
                parts.push(format!("{}", val));
            }
        }
        return parts.join(" ");
    };

    let value_args: Vec<LogicVec> = ir_args[start_idx..].iter()
        .filter_map(|a| eval_display_arg(state, a).ok())
        .collect();

    let mut value_idx = 0usize;
    let mut result = String::new();
    let mut chars = fmt_str.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('d') => {
                    if let Some(val) = value_args.get(value_idx) {
                        result.push_str(&format!("{}", val.to_u64()));
                    }
                    value_idx += 1;
                }
                Some('b') => {
                    if let Some(val) = value_args.get(value_idx) {
                        let s = format!("{}", val);
                        let trimmed = s.trim_start_matches('0');
                        result.push_str(if trimmed.is_empty() { "0" } else { trimmed });
                    }
                    value_idx += 1;
                }
                Some('h') => {
                    if let Some(val) = value_args.get(value_idx) {
                        result.push_str(&format!("{:x}", val.to_u64()));
                    }
                    value_idx += 1;
                }
                Some('s') => {
                    value_idx += 1;
                }
                Some(c2) => {
                    result.push('%');
                    result.push(c2);
                }
                None => {
                    result.push('%');
                }
            }
        } else if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some(c2) => { result.push('\\'); result.push(c2); }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn eval_display_arg(state: &SimulationState, arg: &IrExpr) -> Result<LogicVec, String> {
    match arg {
        IrExpr::String(_) => Ok(LogicVec::new(0)),
        other => {
            match other {
                IrExpr::Const(v) => Ok(v.clone()),
                IrExpr::Signal(id, _) => Ok(state.read_signal(*id).clone()),
                IrExpr::RangeSelect(sig_id, msb, lsb) => {
                    let val = state.read_signal(*sig_id);
                    let (start, end) = if *msb > *lsb { (*lsb, *msb) } else { (*msb, *lsb) };
                    let mut bits = val.bits[start..=end].to_vec();
                    if *msb > *lsb { bits.reverse(); }
                    Ok(LogicVec { width: bits.len(), bits })
                }
                IrExpr::BitSelect(sig_id, idx) => {
                    let val = state.read_signal(*sig_id);
                    let bit = val.bits.get(*idx).copied().unwrap_or(LogicVal::X);
                    Ok(LogicVec { bits: vec![bit], width: 1 })
                }
                IrExpr::ExprRangeSelect(inner, msb, lsb) => {
                    let val = eval_display_arg(state, inner)?;
                    let (start, end) = if *msb > *lsb { (*lsb, *msb) } else { (*msb, *lsb) };
                    if end < val.width {
                        let mut bits = val.bits[start..=end].to_vec();
                        if *msb > *lsb { bits.reverse(); }
                        Ok(LogicVec { width: bits.len(), bits })
                    } else {
                        Ok(LogicVec::new(1))
                    }
                }
                IrExpr::ExprBitSelect(inner, idx) => {
                    let val = eval_display_arg(state, inner)?;
                    let bit = val.bits.get(*idx).copied().unwrap_or(LogicVal::X);
                    Ok(LogicVec { bits: vec![bit], width: 1 })
                }
                IrExpr::ExprPartSelect(inner, base_expr, width_expr) => {
                    let val = eval_display_arg(state, inner)?;
                    let base = eval_display_arg(state, base_expr).ok().map(|v| v.to_u64() as usize).unwrap_or(0);
                    let width = eval_display_arg(state, width_expr).ok().map(|v| v.to_u64() as usize).unwrap_or(0);
                    if width == 0 || base >= val.width {
                        Ok(LogicVec::new(1))
                    } else {
                        let end = (base + width - 1).min(val.width - 1);
                        let mut bits = val.bits[base..=end].to_vec();
                        bits.reverse();
                        Ok(LogicVec { width: bits.len(), bits })
                    }
                }
                IrExpr::Signed(inner) => eval_display_arg(state, inner),
                IrExpr::ArrayIndex { sig_id, index, elem_width } => {
                    let array_val = state.read_signal(*sig_id);
                    let idx = eval_display_arg(state, index).ok().map(|v| v.to_u64() as usize).unwrap_or(0);
                    let start = idx * elem_width;
                    let mut bits = Vec::with_capacity(*elem_width);
                    for i in start..start + elem_width {
                        bits.push(array_val.bits.get(i).copied().unwrap_or(LogicVal::X));
                    }
                    Ok(LogicVec { width: *elem_width, bits })
                }
                _ => {
                    Ok(LogicVec::new(1))
                }
            }
        }
    }
}

fn read_hex_file(filename: &str, elem_width: usize, array_depth: usize, start: Option<usize>, end: Option<usize>) -> Result<Vec<LogicVec>, String> {
    let content = std::fs::read_to_string(filename).map_err(|e| format!("cannot read {}: {}", filename, e))?;
    let start_addr = start.unwrap_or(0);
    let end_addr = end.unwrap_or(array_depth - 1);
    let len = end_addr - start_addr + 1;
    let mut data = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') { continue; }
        let val = i64::from_str_radix(line, 16).map_err(|e| format!("bad hex value '{}': {}", line, e))?;
        data.push(LogicVec::from_u64(val as u64, elem_width));
        if data.len() >= len { break; }
    }
    Ok(data)
}

fn read_bin_file(filename: &str, elem_width: usize, array_depth: usize, start: Option<usize>, end: Option<usize>) -> Result<Vec<LogicVec>, String> {
    let content = std::fs::read_to_string(filename).map_err(|e| format!("cannot read {}: {}", filename, e))?;
    let start_addr = start.unwrap_or(0);
    let end_addr = end.unwrap_or(array_depth - 1);
    let len = end_addr - start_addr + 1;
    let mut data = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') { continue; }
        let val = i64::from_str_radix(line, 2).map_err(|e| format!("bad binary value '{}': {}", line, e))?;
        data.push(LogicVec::from_u64(val as u64, elem_width));
        if data.len() >= len { break; }
    }
    Ok(data)
}

fn logicvec_to_string(lv: &LogicVec) -> String {
    let mut s = String::new();
    let mut byte = 0u8;
    for (i, bit) in lv.bits.iter().enumerate() {
        if *bit == LogicVal::One {
            byte |= 1 << (i % 8);
        } else if *bit == LogicVal::Zero {
            // bit is 0, no change needed
        } else {
            // X or Z — treat as 0
        }
        if i % 8 == 7 {
            s.push(byte as char);
            byte = 0;
        }
    }
    if lv.bits.len() % 8 != 0 {
        s.push(byte as char);
    }
    s
}

fn evaluate_string_method(s: &str, method: &str, args: &[LogicVec]) -> Result<LogicVec, String> {
    match method {
        "len" => {
            Ok(LogicVec::from_u64(s.len() as u64, 32))
        }
        "substr" => {
            if args.len() != 2 {
                return Err(format!("substr expects 2 arguments, got {}", args.len()));
            }
            let i = args[0].to_u64() as usize;
            let j = args[1].to_u64() as usize;
            if i > j || j >= s.len() {
                return Err(format!("substr({}, {}) out of range for string of len {}", i, j, s.len()));
            }
            let sub = &s[i..=j];
            let mut bits = Vec::with_capacity(sub.len() * 8);
            for c in sub.chars() {
                let byte = c as u8;
                for b in 0..8 {
                    bits.push(if (byte >> b) & 1 == 1 { LogicVal::One } else { LogicVal::Zero });
                }
            }
            Ok(LogicVec { width: bits.len(), bits })
        }
        "atoi" => {
            let val: i64 = s.trim().parse().unwrap_or(0);
            Ok(LogicVec::from_u64(val as u64, 32))
        }
        "hextoi" => {
            let trimmed = s.trim().trim_start_matches("0x").trim_start_matches("0X");
            let val = i64::from_str_radix(trimmed, 16).unwrap_or(0);
            Ok(LogicVec::from_u64(val as u64, 32))
        }
        "bintoi" => {
            let trimmed = s.trim();
            let val = i64::from_str_radix(trimmed, 2).unwrap_or(0);
            Ok(LogicVec::from_u64(val as u64, 32))
        }
        "octtoi" => {
            let trimmed = s.trim();
            let val = i64::from_str_radix(trimmed, 8).unwrap_or(0);
            Ok(LogicVec::from_u64(val as u64, 32))
        }
        "tolower" => {
            let lower = s.to_lowercase();
            let mut bits = Vec::with_capacity(lower.len() * 8);
            for c in lower.chars() {
                let byte = c as u8;
                for b in 0..8 {
                    bits.push(if (byte >> b) & 1 == 1 { LogicVal::One } else { LogicVal::Zero });
                }
            }
            Ok(LogicVec { width: bits.len(), bits })
        }
        "toupper" => {
            let upper = s.to_uppercase();
            let mut bits = Vec::with_capacity(upper.len() * 8);
            for c in upper.chars() {
                let byte = c as u8;
                for b in 0..8 {
                    bits.push(if (byte >> b) & 1 == 1 { LogicVal::One } else { LogicVal::Zero });
                }
            }
            Ok(LogicVec { width: bits.len(), bits })
        }
        "compare" | "icompare" => {
            if args.len() < 1 {
                return Err(format!("{} expects 1 argument", method));
            }
            let other_val = &args[0];
            let other = logicvec_to_string(other_val);
            let ordering = if method == "icompare" {
                s.to_lowercase().cmp(&other.to_lowercase())
            } else {
                s.cmp(&other)
            };
            let result = match ordering {
                std::cmp::Ordering::Less => -1i64,
                std::cmp::Ordering::Equal => 0i64,
                std::cmp::Ordering::Greater => 1i64,
            };
            Ok(LogicVec::from_u64(result as u64, 32))
        }
        _ => Err(format!("unknown string method: {}", method)),
    }
}

fn map_ast_binary_op(op: &BinaryOp) -> Result<BinaryIrOp, String> {
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
        BinaryOp::EqWild => Ok(BinaryIrOp::Eq),
        BinaryOp::NeqWild => Ok(BinaryIrOp::Neq),
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

fn map_ast_unary_op(op: &UnaryOp) -> Result<UnaryIrOp, String> {
    match op {
        UnaryOp::Plus => Ok(UnaryIrOp::Plus),
        UnaryOp::Minus => Ok(UnaryIrOp::Minus),
        UnaryOp::BitNot => Ok(UnaryIrOp::BitNot),
        UnaryOp::Not => Ok(UnaryIrOp::Not),
        UnaryOp::ReductionAnd => Ok(UnaryIrOp::RedAnd),
        UnaryOp::ReductionNand => Ok(UnaryIrOp::RedNand),
        UnaryOp::ReductionOr => Ok(UnaryIrOp::RedOr),
        UnaryOp::ReductionNor => Ok(UnaryIrOp::RedNor),
        UnaryOp::ReductionXor => Ok(UnaryIrOp::RedXor),
        UnaryOp::ReductionXnor => Ok(UnaryIrOp::RedXnor),
    }
}
