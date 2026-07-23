use super::{SequenceAttempt, SimulationEngine, MAX_LOOP_ITER};
use crate::waveform::VcdWriter;
use crate::simulator::parallel;
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
    pub(crate) fn process_event(&mut self, event: EventKind, t: usize) -> Result<(), SimError> {
        self.current_time = t as u64;
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
                                self.events[next_t].push(RegionEvent {
                                    region: EventRegion::Active,
                                    event: EventKind::EvalProcess(pid),
                                });
                            }
                        }
                    }
                    Process::Combinational { body, .. } => {
                        self.evaluate_stmt_block(body)?;
                    }
                    Process::CombReactive { body, .. } => {
                        self.evaluate_stmt_block(body)?;
                    }
                    _ => {}
                }
            }
            EventKind::ContinueBlock(cont) => {
                if t < self.events.len() {
                    let all_consumed =
                        self.evaluate_block_with_delay_fork(&cont.stmts_to_exec, cont.fork_id)?;
                    // Detect natural process completion: when a continuation runs to completion (all_consumed)
                    // and has a stored process_id, mark that process as Finished and trigger await continuations
                    if all_consumed {
                        if let Some(pid) = cont.process_id {
                            if let Some(pi) = self.process_map.get_mut(&pid) {
                                if pi.status == ProcessStatus::Running {
                                    pi.status = ProcessStatus::Finished;
                                    let conts = std::mem::take(&mut pi.await_continuations);
                                    for c in conts {
                                        self.evaluate_block_with_delay(&c)?;
                                    }
                                }
                            }
                        }
                    }
                    if let Some(fid) = cont.fork_id {
                        if fid < self.fork_groups.len() && all_consumed {
                            if self.fork_groups[fid].remaining > 0 {
                                self.fork_groups[fid].remaining -= 1;
                            }
                            if self.fork_groups[fid].remaining == 0 {
                                let group = self.fork_groups[fid].clone();
                                if !group.continuation.is_empty() {
                                    self.evaluate_block_with_delay_fork(&group.continuation, None)?;
                                }
                            }
                        }
                    }
                }
            }
            EventKind::ContinueAstBlock(stmts, fork_id) => {
                if t < self.events.len() {
                    let all_consumed = self.evaluate_ast_block_with_delay_fork(&stmts, fork_id)?;
                    if let Some(fid) = fork_id {
                        if fid < self.fork_groups.len() && all_consumed {
                            if self.fork_groups[fid].remaining > 0 {
                                self.fork_groups[fid].remaining -= 1;
                            }
                            if self.fork_groups[fid].remaining == 0 {
                                let group = self.fork_groups[fid].clone();
                                if !group.continuation.is_empty() {
                                    self.evaluate_block_with_delay_fork(&group.continuation, None)?;
                                }
                            }
                        }
                    }
                }
            }
            EventKind::NbaCommit => {
                self.commit_nba();
            }
        }
        Ok(())
    }

    pub(crate) fn evaluate_block_with_delay(&mut self, stmts: &[IrStmt]) -> Result<bool, SimError> {
        self.evaluate_block_with_delay_fork(stmts, None)
    }

    pub(crate) fn evaluate_block_with_delay_fork(
        &mut self,
        stmts: &[IrStmt],
        fork_id: Option<usize>,
    ) -> Result<bool, SimError> {
        for (i, stmt) in stmts.iter().enumerate() {
            if self.disable_pending.is_some() {
                return Ok(true);
            }
            if self.control_flow.is_some() {
                return Ok(true);
            }
            match stmt {
                IrStmt::Block { stmts: inner } => {
                    self.evaluate_block_with_delay_fork(inner, fork_id)?;
                }
                IrStmt::NamedBlock {
                    name, stmts: inner, ..
                } => {
                    if self.disable_pending == Some(*name) {
                        self.disable_pending = None;
                        return Ok(true);
                    }
                    let old = self.disable_pending.take();
                    self.evaluate_block_with_delay_fork(inner, fork_id)?;
                    if let Some(ref n) = self.disable_pending {
                        if *n == *name {
                            self.disable_pending = None;
                        }
                    }
                    self.disable_pending = self.disable_pending.take().or(old);
                }
                IrStmt::If {
                    cond,
                    true_branch: then_stmts,
                    false_branch: else_stmts,
                } => {
                    let cond_val = self.evaluate_expr(cond)?;
                    if cond_val.to_bool().unwrap_or(false) {
                        self.evaluate_block_with_delay_fork(then_stmts, fork_id)?;
                    } else if !else_stmts.is_empty() {
                        self.evaluate_block_with_delay_fork(else_stmts, fork_id)?;
                    }
                }
                IrStmt::Case {
                    case_type,
                    expr: case_expr,
                    items,
                    default,
                } => {
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
                                self.evaluate_block_with_delay_fork(&case_item.body, fork_id)?;
                                if self.disable_pending.is_some() {
                                    return Ok(true);
                                }
                                item_matched = true;
                                matched = true;
                                break;
                            }
                        }
                        if item_matched {
                            break;
                        }
                    }
                    if !matched && !default.is_empty() {
                        self.evaluate_block_with_delay_fork(default, fork_id)?;
                    }
                }
                IrStmt::BlockingAssign { lhs, rhs, delay: _ } => {
                    if !self.is_forced(lhs) {
                        let val = self.eval_assign_rhs(rhs, lhs)?;
                        self.write_lvalue(lhs, val)?;
                    }
                }
                IrStmt::NonBlockingAssign { lhs, rhs, delay: _ } => {
                    if !self.is_forced(lhs) {
                        let val = self.eval_assign_rhs(rhs, lhs)?;
                        self.nba_pending.push((lhs.clone(), val));
                    }
                }
                IrStmt::Force { lvalue, rhs } => {
                    let val = self.eval_assign_rhs(rhs, lvalue)?;
                    self.write_lvalue(lvalue, val)?;
                    if let Some(id) = self.signal_id_from_lvalue(lvalue) {
                        self.forced_signals.insert(id);
                    }
                }
                IrStmt::Delay { delay, body } => {
                    let delay_val = *delay as usize;
                    let delay_t = self.state.time as usize + delay_val;
                    if delay_t < self.events.len() {
                        let mut later: Vec<IrStmt> = body.clone();
                        let remaining: Vec<IrStmt> = stmts[i + 1..].to_vec();
                        later.extend(remaining);
                        if let Some(loop_cont) = &self.loop_continuation {
                            later.extend(loop_cont.clone());
                        }
                        if !later.is_empty() {
                            let region = if delay_val == 0 {
                                EventRegion::Inactive
                            } else {
                                EventRegion::Active
                            };
                            let pid = self.current_process_id;
                            self.events[delay_t].push(RegionEvent {
                                region,
                                event: EventKind::ContinueBlock(Continuation {
                                    stmts_to_exec: later,
                                    stmts_remaining: vec![],
                                    fork_id,
                                    process_id: pid,
                                }),
                            });
                        }
                    }
                    return Ok(false);
                }
                IrStmt::EventControl { sig_id, edge, body } => {
                    let sig_val = self.state.read_signal(*sig_id).clone();
                    let triggered = match edge {
                        Some(ClockEdge::PosEdge(_)) => {
                            if let Some(ref snap) = self.signal_snapshot {
                                let old_val = snap
                                    .get(*sig_id)
                                    .cloned()
                                    .unwrap_or_else(|| LogicVec::new(1));
                                old_val.to_bool() != Some(true) && sig_val.to_bool() == Some(true)
                            } else {
                                sig_val.to_bool() == Some(true)
                            }
                        }
                        Some(ClockEdge::NegEdge(_)) => {
                            if let Some(ref snap) = self.signal_snapshot {
                                let old_val = snap
                                    .get(*sig_id)
                                    .cloned()
                                    .unwrap_or_else(|| LogicVec::new(1));
                                old_val.to_bool() != Some(false) && sig_val.to_bool() == Some(false)
                            } else {
                                sig_val.to_bool() == Some(false)
                            }
                        }
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
                IrStmt::Disable { name } => {
                    self.disable_pending = Some(*name);
                    return Ok(true);
                }
                IrStmt::Release { lvalue } => {
                    if let Some(id) = self.signal_id_from_lvalue(lvalue) {
                        self.forced_signals.remove(&id);
                    }
                }
                IrStmt::Deassign { lvalue } => {
                    if let Some(id) = self.signal_id_from_lvalue(lvalue) {
                        self.forced_signals.remove(&id);
                    }
                }
                IrStmt::Wait { cond, body } => {
                    let cond_val = self.evaluate_expr(cond)?;
                    if cond_val.to_bool().unwrap_or(false) {
                        self.evaluate_block_with_delay_fork(body, fork_id)?;
                        if i + 1 < stmts.len() {
                            self.evaluate_block_with_delay_fork(&stmts[i + 1..], fork_id)?;
                        }
                    } else {
                        let deps = extract_signal_deps(cond);
                        if !deps.is_empty() {
                            let later: Vec<IrStmt> = stmts[i..].to_vec();
                            if !later.is_empty() {
                                self.pending_waits.push((deps, later));
                            }
                        }
                    }
                    return Ok(true);
                }
                IrStmt::WaitOrder {
                    events,
                    failure_stmts,
                } => {
                    let remaining = stmts[i + 1..].to_vec();
                    self.pending_wait_orders.push(WaitOrderState {
                        events: events.clone(),
                        expected_idx: 0,
                        continuation: remaining,
                        failure_stmts: failure_stmts.clone(),
                    });
                    return Ok(false);
                }
                IrStmt::RandCase { items } => {
                    let total: u64 = items
                        .iter()
                        .map(|(w_expr, _)| {
                            self.evaluate_expr(w_expr)
                                .unwrap_or(LogicVec::from_u64(1, 32))
                                .to_u64()
                        })
                        .sum();
                    if total > 0 {
                        let r = self.rng.gen::<u64>() % total;
                        let mut cumulative = 0u64;
                        for (w_expr, body) in items {
                            let weight = self
                                .evaluate_expr(w_expr)
                                .unwrap_or(LogicVec::from_u64(1, 32))
                                .to_u64();
                            cumulative += weight;
                            if r < cumulative {
                                let completed =
                                    self.evaluate_block_with_delay_fork(body, fork_id)?;
                                if !completed {
                                    return Ok(false);
                                }
                                break;
                            }
                        }
                    }
                }
                IrStmt::RandSequence { productions } => {
                    if let Some((_, items)) = productions.first() {
                        let total: u64 = items
                            .iter()
                            .map(|(w, _)| {
                                self.evaluate_expr(w)
                                    .unwrap_or(LogicVec::from_u64(1, 32))
                                    .to_u64()
                            })
                            .sum();
                        if total > 0 {
                            let r = self.rng.gen::<u64>() % total;
                            let mut acc = 0u64;
                            for (w, body) in items {
                                acc += self
                                    .evaluate_expr(w)
                                    .unwrap_or(LogicVec::from_u64(1, 32))
                                    .to_u64();
                                if r < acc {
                                    let completed =
                                        self.evaluate_block_with_delay_fork(body, fork_id)?;
                                    if !completed {
                                        return Ok(false);
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
                IrStmt::SysCall {
                    name,
                    args: ir_args,
                } => {
                    // Handle wrapped $value$plusargs / $test$plusargs from elaborator
                    if name.is_empty() {
                        if let Some(IrExpr::SysFunc {
                            name: fn_name,
                            args: fn_args,
                        }) = ir_args.first()
                        {
                            if fn_name == "value$plusargs" {
                                if let Ok(pat_val) = self.evaluate_expr(
                                    fn_args.first().unwrap_or(&IrExpr::Const(LogicVec::new(0))),
                                ) {
                                    let pattern = logicvec_to_string(&pat_val);
                                    let plusarg_name = pattern
                                        .split('%')
                                        .next()
                                        .unwrap_or(&pattern)
                                        .trim_end_matches('=');
                                    let plusargs = self.plusargs.clone();
                                    for (key, val) in &plusargs {
                                        if key == plusarg_name {
                                            if let Some(var_arg) = fn_args.get(1) {
                                                let num = if let Some(hex) = val
                                                    .strip_prefix("0x")
                                                    .or_else(|| val.strip_prefix("0X"))
                                                {
                                                    u64::from_str_radix(hex, 16).unwrap_or(0)
                                                } else {
                                                    val.parse::<u64>().unwrap_or(0)
                                                };
                                                if let IrExpr::Signal(id, _) = var_arg {
                                                    self.state.write_signal(
                                                        *id,
                                                        LogicVec::from_u64(num, 32),
                                                    );
                                                }
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                    }
                    if name == "display" || name == "write" {
                        let msg = format_display(
                            &self.state,
                            &self.design.top.signals,
                            &self.design.hier_signal_map,
                            &self.assoc_data,
                            ir_args,
                        );
                        print!("{}", msg);
                    } else if name == "strobe" {
                        self.strobe_events.push(ir_args.clone());
                    } else if name == "fstrobe" {
                        let handle = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            self.fstrobe_events.push((h, ir_args[1..].to_vec()));
                        }
                    } else if name == "fmonitor" {
                        let handle = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            let vals: Vec<LogicVec> = ir_args[1..]
                                .iter()
                                .map(|a| self.evaluate_expr(a).unwrap_or(LogicVec::from_u64(0, 32)))
                                .collect();
                            self.fmonitor_map.insert(h, (ir_args[1..].to_vec(), vals));
                        }
                    } else if name == "monitor" {
                        let vals: Vec<LogicVec> = ir_args
                            .iter()
                            .map(|a| self.evaluate_expr(a).unwrap_or(LogicVec::from_u64(0, 32)))
                            .collect();
                        self.monitor_args = Some(ir_args.clone());
                        self.monitor_last_values = Some(vals);
                    } else if name == "readmemh" {
                        let file = ir_args.first().and_then(|a| {
                            if let IrExpr::String(s) = a {
                                Some(s.clone())
                            } else {
                                None
                            }
                        });
                        let sig_id = ir_args.get(1).and_then(|a| {
                            if let IrExpr::Signal(id, _) = a {
                                Some(*id)
                            } else {
                                None
                            }
                        });
                        if let (Some(file), Some(sig_id)) = (file, sig_id) {
                            let data = read_hex_file(&file, 8, 4096, None, None)?;
                            let elem_width = data.first().map(|d| d.width).unwrap_or(8);
                            let mut all_bits = Vec::new();
                            for d in &data {
                                all_bits.extend(d.bits.iter().cloned());
                            }
                            let packed = LogicVec {
                                bits: all_bits,
                                width: data.len() * elem_width,
                            };
                            self.state.write_signal(sig_id, packed);
                        }
                    } else if name == "readmemb" {
                        let file = ir_args.first().and_then(|a| {
                            if let IrExpr::String(s) = a {
                                Some(s.clone())
                            } else {
                                None
                            }
                        });
                        let sig_id = ir_args.get(1).and_then(|a| {
                            if let IrExpr::Signal(id, _) = a {
                                Some(*id)
                            } else {
                                None
                            }
                        });
                        if let (Some(file), Some(sig_id)) = (file, sig_id) {
                            let data = read_bin_file(&file, 8, 4096, None, None)?;
                            let elem_width = data.first().map(|d| d.width).unwrap_or(8);
                            let mut all_bits = Vec::new();
                            for d in &data {
                                all_bits.extend(d.bits.iter().cloned());
                            }
                            let packed = LogicVec {
                                bits: all_bits,
                                width: data.len() * elem_width,
                            };
                            self.state.write_signal(sig_id, packed);
                        }
                    } else if name == "random" {
                        // If seed argument provided (second arg after dest signal),
                        // reseed RNG for reproducibility
                        if let Some(seed_arg) = ir_args.get(1) {
                            if let Ok(seed_val) = self.evaluate_expr(seed_arg) {
                                let seed = seed_val.to_u64();
                                self.rng = rand::rngs::StdRng::seed_from_u64(seed);
                            }
                        }
                        let val: i32 = self.rng.gen();
                        let sig_id = ir_args.first().and_then(|a| {
                            if let IrExpr::Signal(id, _) = a {
                                Some(*id)
                            } else {
                                None
                            }
                        });
                        if let Some(sid) = sig_id {
                            self.state
                                .write_signal(sid, LogicVec::from_u64(val as u64, 32));
                        }
                    } else if name == "urandom" {
                        let val: u32 = self.rng.gen();
                        let sig_id = ir_args.first().and_then(|a| {
                            if let IrExpr::Signal(id, _) = a {
                                Some(*id)
                            } else {
                                None
                            }
                        });
                        if let Some(sid) = sig_id {
                            self.state
                                .write_signal(sid, LogicVec::from_u64(val as u64, 32));
                        }
                    } else if name == "urandom_range" {
                        let args_eval: Vec<LogicVec> = ir_args
                            .iter()
                            .map(|a| self.evaluate_expr(a).unwrap_or(LogicVec::from_u64(0, 32)))
                            .collect();
                        let maxval = args_eval.first().map(|v| v.to_u64()).unwrap_or(0);
                        let minval = args_eval.get(1).map(|v| v.to_u64()).unwrap_or(0);
                        let val = if maxval <= minval {
                            minval
                        } else {
                            let range = maxval - minval + 1;
                            if range <= 1 {
                                minval
                            } else {
                                minval + (self.rng.gen::<u64>() % range)
                            }
                        };
                        let sig_id = ir_args.first().and_then(|a| {
                            if let IrExpr::Signal(id, _) = a {
                                Some(*id)
                            } else {
                                None
                            }
                        });
                        if let Some(sid) = sig_id {
                            self.state.write_signal(sid, LogicVec::from_u64(val, 32));
                        }
                    } else if name == "dumpfile" {
                        if let Some(IrExpr::String(fname)) = ir_args.first() {
                            let path = fname.clone();
                            let design = &self.design;
                            let state = &self.state.signals;
                            if let Some(ref mut vcd) = self.vcd {
                                let _ = vcd.reopen(&path, design, state);
                            } else {
                                match VcdWriter::new(&path, design) {
                                    Ok(v) => self.vcd = Some(v),
                                    Err(e) => eprintln!("VCD: cannot create '{}': {}", path, e),
                                }
                            }
                        }
                    } else if name == "dumpall" {
                        if let Some(ref mut vcd) = self.vcd {
                            vcd.write_time_header(self.state.time)?;
                            let design = &self.design;
                            let state = &self.state.signals;
                            vcd.dump_all(design, state)?;
                        }
                    } else if name == "dumplimit" {
                        if let Some(limit) = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64()))
                        {
                            if let Some(ref mut vcd) = self.vcd {
                                vcd.max_dump_size = Some(limit);
                            }
                        }
                    } else if name == "dumpvars" {
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
                        let fname = ir_args.first().and_then(|a| {
                            if let IrExpr::String(s) = a {
                                Some(s.clone())
                            } else {
                                None
                            }
                        });
                        if let Some(fname) = fname {
                            let mode = ir_args.get(1).and_then(|a| {
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
                                    let sig_id = ir_args.get(1).and_then(|a| {
                                        if let IrExpr::Signal(id, _) = a {
                                            Some(*id)
                                        } else {
                                            None
                                        }
                                    });
                                    if let Some(sid) = sig_id {
                                        self.state.write_signal(
                                            sid,
                                            LogicVec::from_u64(handle as u64, 32),
                                        );
                                    }
                                }
                                Err(_) => {
                                    let sig_id = ir_args.get(1).and_then(|a| {
                                        if let IrExpr::Signal(id, _) = a {
                                            Some(*id)
                                        } else {
                                            None
                                        }
                                    });
                                    if let Some(sid) = sig_id {
                                        self.state.write_signal(sid, LogicVec::from_u64(0, 32));
                                    }
                                }
                            }
                        }
                    } else if name == "fdisplay" {
                        let handle = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let msg = format_display(
                                    &self.state,
                                    &self.design.top.signals,
                                    &self.design.hier_signal_map,
                                    &self.assoc_data,
                                    &ir_args[1..],
                                );
                                let _ = write!(f, "{}", msg);
                            }
                        }
                    } else if name == "fwrite" {
                        let handle = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let msg = format_display(
                                    &self.state,
                                    &self.design.top.signals,
                                    &self.design.hier_signal_map,
                                    &self.assoc_data,
                                    &ir_args[1..],
                                );
                                let _ = write!(f, "{}", msg);
                            }
                        }
                    } else if name == "fscanf" {
                        let handle = ir_args
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
                                let fmt = ir_args.get(1).and_then(|a| {
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
                                                            if let Some(arg) = ir_args.get(out_idx)
                                                            {
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
                                }
                            }
                        }
                    } else if name == "fread" {
                        let target = ir_args.first().and_then(|a| {
                            if let IrExpr::Signal(id, _) = a {
                                Some(*id)
                            } else {
                                None
                            }
                        });
                        let src = ir_args.get(1);
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
                    } else if name == "fclose" {
                        let handle = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            self.file_handles.remove(&h);
                        }
                    } else if name == "fflush" {
                        let handle = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let _ = f.flush();
                            }
                        }
                    } else if name == "fseek" {
                        let handle = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        let offset = ir_args
                            .get(1)
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as i64));
                        let op = ir_args
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
                    } else if name == "__dpi_stmt" {
                        if let Some(arg) = ir_args.first() {
                            self.evaluate_expr(arg)?;
                        }
                    } else if name == "value$plusargs" {
                        let pattern = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok())
                            .map(|v| logicvec_to_string(&v))
                            .unwrap_or_default();
                        let plusarg_name = pattern
                            .split('%')
                            .next()
                            .unwrap_or(&pattern)
                            .trim_end_matches('=');
                        let plusargs = self.plusargs.clone();
                        for (key, val) in &plusargs {
                            if key == plusarg_name {
                                if let Some(var_arg) = ir_args.get(1) {
                                    let num = if let Some(hex) =
                                        val.strip_prefix("0x").or_else(|| val.strip_prefix("0X"))
                                    {
                                        u64::from_str_radix(hex, 16).unwrap_or(0)
                                    } else {
                                        val.parse::<u64>().unwrap_or(0)
                                    };
                                    if let IrExpr::Signal(id, _) = var_arg {
                                        self.state.write_signal(*id, LogicVec::from_u64(num, 32));
                                    }
                                }
                                break;
                            }
                        }
                    } else if name == "asserton" {
                        // $asserton — re-enable all assertions
                        self.assert_off_all = false;
                    } else if name == "assertoff" {
                        // $assertoff — disable all assertions
                        self.assert_off_all = true;
                        // If scope argument provided, disable assertions in that scope
                        if let Some(scope_arg) = ir_args.first() {
                            if let Ok(scope_val) = self.evaluate_expr(scope_arg) {
                                let scope_name = logicvec_to_string(&scope_val);
                                self.assert_modules_off.insert(Symbol::intern(&scope_name));
                            }
                        }
                    } else if name == "assertkill" {
                        // $assertkill — disable and kill all assertions
                        self.assert_kill_all = true;
                        self.assert_off_all = true;
                        if let Some(scope_arg) = ir_args.first() {
                            if let Ok(scope_val) = self.evaluate_expr(scope_arg) {
                                let scope_name = logicvec_to_string(&scope_val);
                                self.assert_modules_off.insert(Symbol::intern(&scope_name));
                            }
                        }
                    } else if name == "assertpasson" {
                        // $assertpasson — re-enable assertion pass action (stub)
                    } else if name == "assertfailon" {
                        // $assertfailon — re-enable assertion fail action (stub)
                    } else if name == "assertnonvacuouson" {
                        // $assertnonvacuouson — stub
                    } else if name == "isunbounded" {
                        // $isunbounded — always returns false for bounded simulations
                        if let Some(sig_arg) = ir_args.first() {
                            if let IrExpr::Signal(id, _) = sig_arg {
                                self.state.write_signal(*id, LogicVec::from_u64(0, 1));
                            }
                        }
                    } else if name == "coverage_control" {
                        // $coverage_control - control coverage collection
                        if let Some(arg) = ir_args.first() {
                            if let Ok(val) = self.evaluate_expr(arg) {
                                let bitmask = val.to_u64();
                                // Bit 0: coverage on/off
                                self.coverage_enabled = (bitmask & 1) == 0;
                                self.coverage_options.insert("control".to_string(), bitmask.to_string());
                            }
                        }
                    } else if name == "coverage_get" {
                        // $coverage_get - get current coverage level
                        let mut total = 0u64;
                        let mut hit = 0u64;
                        for cg in &self.design.covergroups {
                            for cp in &cg.coverpoints {
                                let key = format!("{}.{}", cg.name, cp.name);
                                let key_sym = Symbol::intern(&key);
                                if let Some(t) = self.cover_total.get(&key_sym) {
                                    total += t;
                                }
                                if let Some(h) = self.cover_hits.get(&key_sym) {
                                    hit += h;
                                }
                            }
                        }
                        let pct = if total > 0 {
                            (hit as f64 / total as f64) * 100.0
                        } else {
                            0.0
                        };
                        if let Some(sig_arg) = ir_args.first() {
                            if let IrExpr::Signal(id, _) = sig_arg {
                                self.state
                                    .write_signal(*id, LogicVec::from_u64(pct as u64, 64));
                            }
                        }
                    } else if name == "coverage_save" {
                        // $coverage_save — save coverage data to a file
                        let path = ir_args
                            .first()
                            .and_then(|a| {
                                if let IrExpr::String(s) = a {
                                    Some(s.clone())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_else(|| "coverage.ucis".to_string());
                        let _ = self.export_coverage_ucis(&path);
                    } else if name == "coverage_model" {
                        // $coverage_model — get coverage model handle for a covergroup
                        // Usage: $coverage_model(output_signal [, "covergroup_name"])
                        let cg_name = ir_args.get(1).and_then(|a| {
                            if let IrExpr::String(s) = a {
                                Some(s.clone())
                            } else {
                                None
                            }
                        });
                        let handle: u32 = if let Some(ref name) = cg_name {
                            let exists = self
                                .design
                                .covergroups
                                .iter()
                                .any(|cg| cg.name.as_str() == name.as_str());
                            if exists {
                                if let Some((&h, _)) = self
                                    .coverage_model_handles
                                    .iter()
                                    .find(|(_, n)| n.as_str() == name.as_str())
                                {
                                    h as u32
                                } else {
                                    let h = self.next_coverage_model_handle;
                                    self.next_coverage_model_handle += 1;
                                    self.coverage_model_handles.insert(h, Symbol::intern(&name));
                                    h as u32
                                }
                            } else {
                                eprintln!(
                                    "warning: $coverage_model: covergroup '{}' not found",
                                    name
                                );
                                0
                            }
                        } else if let Some(first_cg) = self.design.covergroups.first() {
                            if let Some((&h, _)) = self
                                .coverage_model_handles
                                .iter()
                                .find(|(_, n)| n.as_str() == first_cg.name.as_str())
                            {
                                h as u32
                            } else {
                                let h = self.next_coverage_model_handle;
                                self.next_coverage_model_handle += 1;
                                self.coverage_model_handles.insert(h, first_cg.name);
                                h as u32
                            }
                        } else {
                            0
                        };
                        if let Some(sig_arg) = ir_args.first() {
                            if let IrExpr::Signal(id, _) = sig_arg {
                                self.state
                                    .write_signal(*id, LogicVec::from_u64(handle as u64, 32));
                            }
                        }
                    } else if name == "load_coverage_db" {
                        // $load_coverage_db — stub: acknowledge but do nothing
                        eprintln!("warning: $load_coverage_db not yet implemented");
                    } else if name == "swrite" || name == "sformat" {
                        // $swrite/$sformat — format values into string variable
                        // Format: $swrite(output_str, format, args...)
                        // Note: $swrite appends newline, $sformat does not
                        if let Some(IrExpr::Signal(out_id, _)) = ir_args.first() {
                            let format_args = &ir_args[1..];
                            let mut msg = format_display(
                                &self.state,
                                &self.design.top.signals,
                                &self.design.hier_signal_map,
                                &self.assoc_data,
                                format_args,
                            );
                            if name == "swrite" {
                                msg.push('\n');
                            }
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
                            self.state.write_signal(
                                *out_id,
                                LogicVec {
                                    width: bits.len(),
                                    bits,
                                },
                            );
                        }
                    } else if name == "sscanf" {
                        // $sscanf — scan values from string
                        // Format: $sscanf(input_str, format, output_args...)
                        if let Some(input_arg) = ir_args.first() {
                            let input_str = if let IrExpr::String(s) = input_arg {
                                s.clone()
                            } else if let Ok(val) = self.evaluate_expr(input_arg) {
                                logicvec_to_string(&val)
                            } else {
                                String::new()
                            };
                            let fmt = ir_args.get(1).and_then(|a| {
                                if let IrExpr::String(s) = a {
                                    Some(s.clone())
                                } else {
                                    None
                                }
                            });
                            if let Some(ref fmt_str) = fmt {
                                let tokens: Vec<&str> = input_str.split_whitespace().collect();
                                let mut ti = 0;
                                let mut ai = 0;
                                let mut chars = fmt_str.chars().peekable();
                                while let Some(c) = chars.next() {
                                    if c == '%' {
                                        if let Some(spec) = chars.next() {
                                            if spec == 'd'
                                                || spec == 'h'
                                                || spec == 'b'
                                                || spec == 'o'
                                            {
                                                if let Some(tok) = tokens.get(ti) {
                                                    let radix = if spec == 'h' {
                                                        16
                                                    } else if spec == 'o' {
                                                        8
                                                    } else if spec == 'b' {
                                                        2
                                                    } else {
                                                        10
                                                    };
                                                    if let Ok(val) = i64::from_str_radix(tok, radix)
                                                    {
                                                        if let Some(out_arg) = ir_args.get(2 + ai) {
                                                            if let IrExpr::Signal(sid, _) = out_arg
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
                                            } else if spec == 's' {
                                                // String format: consume all remaining tokens
                                                if let Some(out_arg) = ir_args.get(2 + ai) {
                                                    if let IrExpr::Signal(sid, _) = out_arg {
                                                        let s = tokens[ti..].join(" ");
                                                        let mut bits =
                                                            Vec::with_capacity(s.len() * 8);
                                                        for c in s.chars() {
                                                            let byte = c as u8;
                                                            for i in 0..8 {
                                                                bits.push(
                                                                    if (byte >> i) & 1 == 1 {
                                                                        LogicVal::One
                                                                    } else {
                                                                        LogicVal::Zero
                                                                    },
                                                                );
                                                            }
                                                        }
                                                        self.state.write_signal(
                                                            *sid,
                                                            LogicVec {
                                                                width: bits.len(),
                                                                bits,
                                                            },
                                                        );
                                                    }
                                                }
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if name == "test$plusargs" {
                        // $test$plusargs in statement context — return value ignored
                    } else {
                        eprintln!("warning: unknown system call '{}' ignored", name);
                    }
                }
                IrStmt::SysFinish => {
                    // Flush all pending await continuations before stopping
                    for (_, pi) in self.process_map.iter_mut() {
                        if pi.status == ProcessStatus::Running
                            || pi.status == ProcessStatus::Waiting
                        {
                            pi.status = ProcessStatus::Finished;
                        }
                        pi.await_continuations.clear();
                    }
                    self.running = false;
                    return Ok(true);
                }
                IrStmt::Null => {}
                IrStmt::Assert {
                    cond,
                    pass_stmt,
                    fail_stmt,
                    clock_event,
                    disable_iff,
                    sequence,
                } => {
                    let should_check = match clock_event {
                        Some(ref ce) => self.check_concurrent_clock_event(ce),
                        None => !self.assert_off_all,
                    };
                    if should_check {
                        let disabled = match disable_iff {
                            Some(ref di) => self.evaluate_expr(di)?.to_bool().unwrap_or(false),
                            None => false,
                        };
                        if !disabled && !self.assert_kill_all {
                            if let Some(seq) = &sequence {
                                // Concurrent assertion with temporal sequence: start a new attempt
                                self.sequence_attempts.push(SequenceAttempt {
                                    sequence: seq.clone(),
                                    cycles: 0,
                                    pass_stmt: pass_stmt.clone(),
                                    fail_stmt: fail_stmt.clone(),
                                    clock_event: clock_event.clone().unwrap(),
                                });
                            } else {
                                // Immediate assertion: evaluate condition now
                                let ok = self.evaluate_expr(cond)?.to_bool().unwrap_or(false);
                                if ok {
                                    if !pass_stmt.is_empty() {
                                        self.evaluate_block_with_delay_fork(pass_stmt, fork_id)?;
                                    }
                                } else {
                                    eprintln!("assertion failed");
                                    if !fail_stmt.is_empty() {
                                        self.evaluate_block_with_delay_fork(fail_stmt, fork_id)?;
                                    }
                                }
                            }
                        }
                    }
                }
                IrStmt::Assume {
                    cond,
                    pass_stmt,
                    fail_stmt,
                    clock_event,
                    disable_iff,
                    sequence: _,
                } => {
                    let should_check = match clock_event {
                        Some(ref ce) => self.check_concurrent_clock_event(ce),
                        None => !self.assert_off_all,
                    };
                    if should_check {
                        let disabled = match disable_iff {
                            Some(ref di) => self.evaluate_expr(di)?.to_bool().unwrap_or(false),
                            None => false,
                        };
                        if !disabled && !self.assert_kill_all {
                            let ok = self.evaluate_expr(cond)?.to_bool().unwrap_or(false);
                            if ok {
                                if !pass_stmt.is_empty() {
                                    self.evaluate_block_with_delay_fork(pass_stmt, fork_id)?;
                                }
                            } else {
                                eprintln!("assumption violated");
                                if !fail_stmt.is_empty() {
                                    self.evaluate_block_with_delay_fork(fail_stmt, fork_id)?;
                                }
                            }
                        }
                    }
                }
                IrStmt::Cover {
                    cond,
                    pass_stmt,
                    clock_event,
                    disable_iff,
                    sequence: _,
                } => {
                    let should_check = match clock_event {
                        Some(ref ce) => self.check_concurrent_clock_event(ce),
                        None => !self.assert_off_all,
                    };
                    if should_check {
                        let disabled = match disable_iff {
                            Some(ref di) => self.evaluate_expr(di)?.to_bool().unwrap_or(false),
                            None => false,
                        };
                        if !disabled && !self.assert_kill_all {
                            let ok = self.evaluate_expr(cond)?.to_bool().unwrap_or(false);
                            if ok {
                                eprintln!("cover point hit");
                                if !pass_stmt.is_empty() {
                                    self.evaluate_block_with_delay_fork(pass_stmt, fork_id)?;
                                }
                            }
                        }
                    }
                }
                IrStmt::Break => {
                    self.control_flow = Some(FlowControl::Break);
                    return Ok(true);
                }
                IrStmt::Continue => {
                    self.control_flow = Some(FlowControl::Continue);
                    return Ok(true);
                }
                IrStmt::LoopFor {
                    init,
                    cond,
                    step,
                    body,
                } => {
                    if let Some(init_stmt) = init {
                        self.evaluate_block_with_delay_fork(&[*init_stmt.clone()], fork_id)?;
                    }
                    let mut iter_count = 0usize;
                    loop {
                        if iter_count >= MAX_LOOP_ITER {
                            eprintln!(
                                "warning: for loop exceeded {} iterations, breaking",
                                MAX_LOOP_ITER
                            );
                            break;
                        }
                        iter_count += 1;
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if self.control_flow.is_some() {
                            self.control_flow = None;
                            break;
                        }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) {
                            break;
                        }
                        self.evaluate_block_with_delay_fork(body, fork_id)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            if let Some(step_stmt) = step {
                                self.evaluate_block_with_delay_fork(
                                    &[*step_stmt.clone()],
                                    fork_id,
                                )?;
                            }
                            continue;
                        }
                        if cf == Some(FlowControl::Break) {
                            break;
                        }
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if let Some(step_stmt) = step {
                            self.evaluate_block_with_delay_fork(&[*step_stmt.clone()], fork_id)?;
                        }
                    }
                }
                IrStmt::LoopWhile { cond, body } => {
                    let mut iter_count = 0usize;
                    loop {
                        if iter_count >= MAX_LOOP_ITER {
                            eprintln!(
                                "warning: while loop exceeded {} iterations, breaking",
                                MAX_LOOP_ITER
                            );
                            break;
                        }
                        iter_count += 1;
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if self.control_flow.is_some() {
                            self.control_flow = None;
                            break;
                        }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) {
                            break;
                        }
                        let old_loop_cont = self.loop_continuation.take();
                        self.loop_continuation = Some(vec![IrStmt::LoopWhile {
                            cond: cond.clone(),
                            body: body.clone(),
                        }]);
                        let completed = self.evaluate_block_with_delay_fork(body, fork_id)?;
                        self.loop_continuation = old_loop_cont;
                        if !completed {
                            return Ok(false);
                        }
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            continue;
                        }
                        if cf == Some(FlowControl::Break) {
                            break;
                        }
                    }
                }
                IrStmt::LoopDoWhile { cond, body } => {
                    let mut iter_count = 0usize;
                    loop {
                        if iter_count >= MAX_LOOP_ITER {
                            eprintln!(
                                "warning: do-while loop exceeded {} iterations, breaking",
                                MAX_LOOP_ITER
                            );
                            break;
                        }
                        iter_count += 1;
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if self.control_flow.is_some() {
                            self.control_flow = None;
                            break;
                        }
                        let old_loop_cont = self.loop_continuation.take();
                        self.loop_continuation = Some(vec![IrStmt::LoopDoWhile {
                            cond: cond.clone(),
                            body: body.clone(),
                        }]);
                        let completed = self.evaluate_block_with_delay_fork(body, fork_id)?;
                        self.loop_continuation = old_loop_cont;
                        if !completed {
                            return Ok(false);
                        }
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            continue;
                        }
                        if cf == Some(FlowControl::Break) {
                            break;
                        }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) {
                            break;
                        }
                    }
                }
                IrStmt::Repeat { count, body } => {
                    let count_val = self.evaluate_expr(count)?;
                    let n = (count_val.to_u64() as usize).min(MAX_LOOP_ITER);
                    for _ in 0..n {
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if self.control_flow.is_some() {
                            self.control_flow = None;
                            break;
                        }
                        self.evaluate_block_with_delay_fork(body, fork_id)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            continue;
                        }
                        if cf == Some(FlowControl::Break) {
                            break;
                        }
                    }
                }
                IrStmt::Foreach {
                    array_var,
                    index_var,
                    body,
                } => {
                    let lv = self.evaluate_expr(array_var)?;
                    let sig_info = if let IrExpr::Signal(id, _) = array_var {
                        self.design.top.signals.get(*id)
                    } else {
                        None
                    };
                    let elem_width = sig_info.map(|s| s.elem_width).unwrap_or(1);
                    let count = if elem_width > 0 {
                        lv.width / elem_width
                    } else {
                        0
                    };
                    for i in 0..count {
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if self.control_flow.is_some() {
                            self.control_flow = None;
                            break;
                        }
                        let idx_val = LogicVec::from_u64(i as u64, 32);
                        let mut scope = HashMap::new();
                        scope.insert(index_var.clone(), idx_val);
                        let depth = self.method_locals.len();
                        self.method_locals.push(scope);
                        self.evaluate_block_with_delay_fork(body, fork_id)?;
                        self.method_locals.truncate(depth);
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            continue;
                        }
                        if cf == Some(FlowControl::Break) {
                            break;
                        }
                    }
                }
                IrStmt::MethodCallStmt {
                    obj,
                    method,
                    args,
                    with_clause,
                } => {
                    if let IrExpr::Signal(id, _) = obj {
                        let sig_info = self.design.top.signals.get(*id).cloned();
                        if let Some(ref sig) = sig_info {
                            if sig.is_dynamic || sig.is_queue || sig.is_associative {
                                let _ =                                    self.evaluate_array_method(
                                        *id,
                                        sig,
                                        method.as_str(),
                                        args,
                                    with_clause.as_deref(),
                                )?;
                                continue;
                            }
                            // Auto-create object for class/covergroup variables
                            if let Some(ref cn) = sig.class_name {
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
                                        self.execute_method(new_id, method.as_str(), &arg_vals)?;
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    let obj_val = self.evaluate_expr(obj)?;
                    let obj_id = obj_val.to_u64() as ObjId;
                    let arg_vals: Vec<LogicVec> = args
                        .iter()
                        .map(|a| self.evaluate_expr(a))
                        .collect::<Result<_, _>>()?;
                    self.execute_method(obj_id, method.as_str(), &arg_vals)?;
                }
                IrStmt::Fork {
                    processes,
                    join_type,
                } => {
                    let fid = self.fork_groups.len();
                    let remaining: Vec<IrStmt> = stmts[i + 1..].to_vec();
                    let count = processes.len();
                    self.fork_groups.push(ForkGroup {
                        remaining: count,
                        continuation: remaining.clone(),
                    });
                    match join_type {
                        IrJoinType::Join => {
                            for p in processes {
                                if p.is_empty() {
                                    if self.fork_groups[fid].remaining > 0 {
                                        self.fork_groups[fid].remaining -= 1;
                                    }
                                } else {
                                    let all_consumed =
                                        self.evaluate_block_with_delay_fork(&p, Some(fid))?;
                                    if all_consumed && self.fork_groups[fid].remaining > 0 {
                                        self.fork_groups[fid].remaining -= 1;
                                    }
                                }
                            }
                            if self.fork_groups[fid].remaining == 0 && !remaining.is_empty() {
                                let group = self.fork_groups[fid].clone();
                                self.evaluate_block_with_delay_fork(&group.continuation, None)?;
                            }
                        }
                        IrJoinType::JoinAny => {
                            self.fork_groups[fid].remaining = 1;
                            let mut any_immediate = false;
                            for p in processes {
                                if p.is_empty() {
                                    any_immediate = true;
                                } else {
                                    let all_consumed =
                                        self.evaluate_block_with_delay_fork(&p, Some(fid))?;
                                    if all_consumed {
                                        any_immediate = true;
                                    }
                                }
                            }
                            if any_immediate && self.fork_groups[fid].remaining > 0 {
                                self.fork_groups[fid].remaining -= 1;
                            }
                            if self.fork_groups[fid].remaining == 0 && !remaining.is_empty() {
                                let group = self.fork_groups[fid].clone();
                                self.evaluate_block_with_delay_fork(&group.continuation, None)?;
                            }
                        }
                        IrJoinType::JoinNone => {
                            for p in processes {
                                if !p.is_empty() {
                                    self.evaluate_block_with_delay_fork(&p, Some(fid))?;
                                }
                            }
                            if !remaining.is_empty() {
                                self.evaluate_block_with_delay(&remaining)?;
                            }
                        }
                    }
                    return Ok(true);
                }
            }
            // Post-statement check: if process::await() was called on a running process,
            // capture remaining statements as await continuation and yield
            if let Some(target_id) = self.pending_await_target.take() {
                let remaining: Vec<IrStmt> = stmts[i + 1..].to_vec();
                let mut cont = remaining;
                if let Some(lc) = &self.loop_continuation {
                    cont.extend(lc.clone());
                }
                if let Some(pi) = self.process_map.get_mut(&target_id) {
                    pi.await_continuations.push(cont);
                }
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(crate) fn evaluate_ast_block_with_delay_fork(
        &mut self,
        stmts: &[crate::ast::Stmt],
        fork_id: Option<usize>,
    ) -> Result<bool, SimError> {
        for (i, stmt) in stmts.iter().enumerate() {
            if self.disable_pending.is_some() {
                return Ok(true);
            }
            if self.control_flow.is_some() {
                return Ok(true);
            }
            match stmt {
                crate::ast::Stmt::Block { stmts: inner } => {
                    self.evaluate_ast_block_with_delay_fork(inner, fork_id)?;
                }
                crate::ast::Stmt::NamedBlock {
                    name,
                    stmts: inner,
                    decls: _,
                } => {
                    if self.disable_pending == Some(*name) {
                        self.disable_pending = None;
                        return Ok(true);
                    }
                    let old = self.disable_pending.take();
                    self.evaluate_ast_block_with_delay_fork(inner, fork_id)?;
                    if let Some(ref n) = self.disable_pending {
                        if *n == *name {
                            self.disable_pending = None;
                        }
                    }
                    self.disable_pending = self.disable_pending.take().or(old);
                }
                crate::ast::Stmt::BlockingAssign { lhs, rhs, delay: _ } => {
                    let val = self.evaluate_ast_expr(rhs)?;
                    self.write_ast_lvalue(lhs, val)?;
                }
                crate::ast::Stmt::NonBlockingAssign { lhs, rhs, delay: _ } => {
                    let val = self.evaluate_ast_expr(rhs)?;
                    // Convert AST lvalue to IrLValue for nba tracking
                    if let Some(ir_lv) = self.ast_lvalue_to_ir(lhs) {
                        self.nba_pending.push((ir_lv, val));
                    } else {
                        self.write_ast_lvalue(lhs, val)?;
                    }
                }
                crate::ast::Stmt::IfElse {
                    cond,
                    true_branch,
                    false_branch,
                } => {
                    let cond_val = self.evaluate_ast_expr(cond)?;
                    if cond_val.to_bool().unwrap_or(false) {
                        self.evaluate_ast_block_with_delay_fork(&[*true_branch.clone()], fork_id)?;
                    } else if let Some(fb) = false_branch {
                        self.evaluate_ast_block_with_delay_fork(&[*fb.clone()], fork_id)?;
                    }
                }
                crate::ast::Stmt::Case {
                    expr,
                    items,
                    default,
                } => {
                    let case_val = self.evaluate_ast_expr(expr)?;
                    let mut matched = false;
                    for item in items {
                        let mut item_matched = false;
                        for pat in &item.labels {
                            let pat_val = self.evaluate_ast_expr(pat)?;
                            if case_val.eq(&pat_val) {
                                self.evaluate_ast_block_with_delay_fork(
                                    &[*item.stmt.clone()],
                                    fork_id,
                                )?;
                                if self.disable_pending.is_some() {
                                    return Ok(true);
                                }
                                item_matched = true;
                                matched = true;
                                break;
                            }
                        }
                        if item_matched {
                            break;
                        }
                    }
                    if !matched {
                        if let Some(def) = default {
                            self.evaluate_ast_block_with_delay_fork(&[*def.clone()], fork_id)?;
                        }
                    }
                }
                crate::ast::Stmt::CaseX {
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
                                self.evaluate_ast_block_with_delay_fork(
                                    &[*item.stmt.clone()],
                                    fork_id,
                                )?;
                                matched = true;
                                break;
                            }
                        }
                        if matched {
                            break;
                        }
                    }
                    if !matched {
                        if let Some(def) = default {
                            self.evaluate_ast_block_with_delay_fork(&[*def.clone()], fork_id)?;
                        }
                    }
                }
                crate::ast::Stmt::CaseZ {
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
                                self.evaluate_ast_block_with_delay_fork(
                                    &[*item.stmt.clone()],
                                    fork_id,
                                )?;
                                matched = true;
                                break;
                            }
                        }
                        if matched {
                            break;
                        }
                    }
                    if !matched {
                        if let Some(def) = default {
                            self.evaluate_ast_block_with_delay_fork(&[*def.clone()], fork_id)?;
                        }
                    }
                }
                crate::ast::Stmt::LoopForever { stmts: inner } => loop {
                    if self.disable_pending.is_some() {
                        break;
                    }
                    if self.control_flow.is_some() {
                        self.control_flow = None;
                        break;
                    }
                    self.evaluate_ast_block_with_delay_fork(inner, fork_id)?;
                    let cf = self.control_flow.take();
                    if cf == Some(FlowControl::Break) {
                        break;
                    }
                    if cf == Some(FlowControl::Continue) {
                        continue;
                    }
                },
                crate::ast::Stmt::LoopWhile { cond, stmts: inner } => loop {
                    if self.disable_pending.is_some() {
                        break;
                    }
                    if self.control_flow.is_some() {
                        self.control_flow = None;
                        break;
                    }
                    let cond_val = self.evaluate_ast_expr(cond)?;
                    if !cond_val.to_bool().unwrap_or(false) {
                        break;
                    }
                    self.evaluate_ast_block_with_delay_fork(inner, fork_id)?;
                    let cf = self.control_flow.take();
                    if cf == Some(FlowControl::Break) {
                        break;
                    }
                    if cf == Some(FlowControl::Continue) {
                        continue;
                    }
                },
                crate::ast::Stmt::DoWhile { cond, stmts: inner } => loop {
                    if self.disable_pending.is_some() {
                        break;
                    }
                    if self.control_flow.is_some() {
                        self.control_flow = None;
                        break;
                    }
                    self.evaluate_ast_block_with_delay_fork(inner, fork_id)?;
                    let cf = self.control_flow.take();
                    if cf == Some(FlowControl::Continue) {
                        continue;
                    }
                    if cf == Some(FlowControl::Break) {
                        break;
                    }
                    let cond_val = self.evaluate_ast_expr(cond)?;
                    if !cond_val.to_bool().unwrap_or(false) {
                        break;
                    }
                },
                crate::ast::Stmt::LoopFor {
                    init,
                    cond,
                    step,
                    stmts: inner,
                } => {
                    if let Some(init_stmt) = init {
                        self.evaluate_ast_block_with_delay_fork(&[*init_stmt.clone()], fork_id)?;
                    }
                    loop {
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if self.control_flow.is_some() {
                            self.control_flow = None;
                            break;
                        }
                        if let Some(ref c) = cond {
                            let cv = self.evaluate_ast_expr(c)?;
                            if !cv.to_bool().unwrap_or(false) {
                                break;
                            }
                        }
                        self.evaluate_ast_block_with_delay_fork(inner, fork_id)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            if let Some(s) = step {
                                self.evaluate_ast_block_with_delay_fork(&[*s.clone()], fork_id)?;
                            }
                            continue;
                        }
                        if cf == Some(FlowControl::Break) {
                            break;
                        }
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if let Some(s) = step {
                            self.evaluate_ast_block_with_delay_fork(&[*s.clone()], fork_id)?;
                        }
                    }
                }
                crate::ast::Stmt::Repeat {
                    count,
                    stmts: inner,
                } => {
                    let count_val = self.evaluate_ast_expr(count)?;
                    let n = count_val.to_u64() as usize;
                    for _ in 0..n {
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if self.control_flow.is_some() {
                            self.control_flow = None;
                            break;
                        }
                        self.evaluate_ast_block_with_delay_fork(inner, fork_id)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            continue;
                        }
                        if cf == Some(FlowControl::Break) {
                            break;
                        }
                    }
                }
                crate::ast::Stmt::Delay { delay, stmt: body } => {
                    let delay_val = self.evaluate_ast_expr(delay)?;
                    let d = delay_val.to_u64() as usize;
                    let delay_t = self.state.time as usize + d;
                    if delay_t < self.events.len() {
                        let remaining: Vec<crate::ast::Stmt> = {
                            let mut v = Vec::new();
                            v.push(*body.clone());
                            if i + 1 < stmts.len() {
                                v.extend(stmts[i + 1..].iter().cloned());
                            }
                            v
                        };
                        let region = if d == 0 {
                            EventRegion::Inactive
                        } else {
                            EventRegion::Active
                        };
                        self.events[delay_t].push(RegionEvent {
                            region,
                            event: EventKind::ContinueAstBlock(remaining, fork_id),
                        });
                    }
                    return Ok(false);
                }
                crate::ast::Stmt::EventControl { events, stmt: body } => {
                    // For class tasks, handle event control by evaluating signal
                    // For now, execute immediately (simple edge handling)
                    // In a full implementation, we would schedule a continuation
                    if let Some(event) = events.first() {
                        let triggered = match event {
                            crate::ast::SensitivityEvent::PosEdge(expr) => {
                                if let Some(id) = self.find_ast_signal_id(expr) {
                                    let sig_val = self.state.read_signal(id);
                                    sig_val.to_bool() == Some(true)
                                } else {
                                    true
                                }
                            }
                            crate::ast::SensitivityEvent::NegEdge(expr) => {
                                if let Some(id) = self.find_ast_signal_id(expr) {
                                    let sig_val = self.state.read_signal(id);
                                    sig_val.to_bool() == Some(false)
                                } else {
                                    true
                                }
                            }
                            _ => true,
                        };
                        if triggered {
                            if let Some(b) = body {
                                self.evaluate_ast_block_with_delay_fork(&[*b.clone()], fork_id)?;
                            }
                            if i + 1 < stmts.len() {
                                self.evaluate_ast_block_with_delay_fork(&stmts[i + 1..], fork_id)?;
                            }
                        } else {
                            // Not triggered — schedule a wake-up when the signal changes
                            // For now: just don't execute and return
                            return Ok(true);
                        }
                    }
                }
                crate::ast::Stmt::Wait { cond, stmt: body } => {
                    let cond_val = self.evaluate_ast_expr(cond)?;
                    if cond_val.to_bool().unwrap_or(false) {
                        if let Some(b) = body {
                            self.evaluate_ast_block_with_delay_fork(&[*b.clone()], fork_id)?;
                        }
                        if i + 1 < stmts.len() {
                            self.evaluate_ast_block_with_delay_fork(&stmts[i + 1..], fork_id)?;
                        }
                    } else {
                        // Condition not met yet — skip
                        return Ok(true);
                    }
                }
                crate::ast::Stmt::SysCall { name, args } => {
                    // For task context, delegate to SysCall handler
                    self.handle_ast_syscall(name.as_str(), args)?;
                }
                crate::ast::Stmt::SysFinish => {
                    self.running = false;
                    return Ok(true);
                }
                crate::ast::Stmt::Expr { expr } => {
                    self.evaluate_ast_expr(expr)?;
                }
                crate::ast::Stmt::Break => {
                    self.control_flow = Some(FlowControl::Break);
                    return Ok(true);
                }
                crate::ast::Stmt::Continue => {
                    self.control_flow = Some(FlowControl::Continue);
                    return Ok(true);
                }
                crate::ast::Stmt::Return(_) => {
                    return Ok(true);
                }
                crate::ast::Stmt::Null => {}
                crate::ast::Stmt::Force { lhs, rhs } => {
                    let val = self.evaluate_ast_expr(rhs)?;
                    self.write_ast_lvalue(lhs, val)?;
                }
                crate::ast::Stmt::Release { expr: _ } => {
                    // Release variable — just a no-op in AST context
                }
                crate::ast::Stmt::EventTrigger { name } => {
                    // Find signal by name and toggle it
                    if let Some(id) = self.find_signal(name.as_str()) {
                        let val = self.state.read_signal(id);
                        let toggled = if val.to_bool().unwrap_or(false) {
                            LogicVec::from_u64(0, val.width.max(1))
                        } else {
                            LogicVec::from_u64(1, val.width.max(1))
                        };
                        self.state.write_signal(id, toggled);
                    }
                }
                crate::ast::Stmt::Disable { name } => {
                    self.disable_pending = Some(name.clone());
                    return Ok(true);
                }
                crate::ast::Stmt::Fork {
                    processes,
                    join_type,
                } => {
                    let fid = self.fork_groups.len();
                    let remaining: Vec<crate::ast::Stmt> = if i + 1 < stmts.len() {
                        stmts[i + 1..].to_vec()
                    } else {
                        Vec::new()
                    };
                    // Convert join type
                    let _ir_join = match join_type {
                        crate::ast::JoinType::Join => IrJoinType::Join,
                        crate::ast::JoinType::JoinAny => IrJoinType::JoinAny,
                        crate::ast::JoinType::JoinNone => IrJoinType::JoinNone,
                    };
                    // We need to work with IR Fork here — for AST fork inside a task, we execute immediately
                    // This is a simplification — full fork support in AST tasks would need more work
                    // processes is Vec<Stmt> (each branch is a Stmt::Block or single stmt)
                    for p in processes {
                        self.evaluate_ast_block_with_delay_fork(&[p.clone()], Some(fid))?;
                    }
                    if !remaining.is_empty() {
                        self.evaluate_ast_block_with_delay_fork(&remaining, None)?;
                    }
                    return Ok(true);
                }
                crate::ast::Stmt::Assert {
                    cond,
                    pass_stmt,
                    fail_stmt,
                    ..
                } => {
                    let ok = self.evaluate_ast_expr(cond)?.to_bool().unwrap_or(false);
                    if ok {
                        if let Some(ps) = pass_stmt {
                            self.evaluate_ast_block_with_delay_fork(&[*ps.clone()], fork_id)?;
                        }
                    } else {
                        eprintln!("assertion failed");
                        if let Some(fs) = fail_stmt {
                            self.evaluate_ast_block_with_delay_fork(&[*fs.clone()], fork_id)?;
                        }
                    }
                }
                crate::ast::Stmt::Assume {
                    cond,
                    pass_stmt,
                    fail_stmt,
                    ..
                } => {
                    let ok = self.evaluate_ast_expr(cond)?.to_bool().unwrap_or(false);
                    if ok {
                        if let Some(ps) = pass_stmt {
                            self.evaluate_ast_block_with_delay_fork(&[*ps.clone()], fork_id)?;
                        }
                    } else {
                        eprintln!("assumption violated");
                        if let Some(fs) = fail_stmt {
                            self.evaluate_ast_block_with_delay_fork(&[*fs.clone()], fork_id)?;
                        }
                    }
                }
                crate::ast::Stmt::Cover {
                    cond, pass_stmt, ..
                } => {
                    let ok = self.evaluate_ast_expr(cond)?.to_bool().unwrap_or(false);
                    if ok {
                        if let Some(ps) = pass_stmt {
                            self.evaluate_ast_block_with_delay_fork(&[*ps.clone()], fork_id)?;
                        }
                    }
                }
                _ => {
                    // Unhandled statement types in task method context
                }
            }
        }
        Ok(true)
    }

    pub(crate) fn evaluate_stmt_block(&mut self, stmts: &[IrStmt]) -> Result<(), SimError> {
        for (i, stmt) in stmts.iter().enumerate() {
            if self.disable_pending.is_some() {
                return Ok(());
            }
            if self.control_flow.is_some() {
                return Ok(());
            }
            match stmt {
                IrStmt::BlockingAssign { lhs, rhs, delay: _ } => {
                    if !self.is_forced(lhs) {
                        let val = self.eval_assign_rhs(rhs, lhs)?;
                        self.write_lvalue(lhs, val)?;
                    }
                }
                IrStmt::NonBlockingAssign { lhs, rhs, delay: _ } => {
                    if !self.is_forced(lhs) {
                        let val = self.eval_assign_rhs(rhs, lhs)?;
                        self.nba_pending.push((lhs.clone(), val));
                    }
                }
                IrStmt::Force { lvalue, rhs } => {
                    let val = self.eval_assign_rhs(rhs, lvalue)?;
                    self.write_lvalue(lvalue, val)?;
                    if let Some(id) = self.signal_id_from_lvalue(lvalue) {
                        self.forced_signals.insert(id);
                    }
                }
                IrStmt::If {
                    cond,
                    true_branch: then_stmts,
                    false_branch: else_stmts,
                } => {
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
                IrStmt::NamedBlock {
                    name, stmts: inner, ..
                } => {
                    if self.disable_pending == Some(*name) {
                        self.disable_pending = None;
                        return Ok(());
                    }
                    let old = self.disable_pending.take();
                    self.evaluate_stmt_block(inner)?;
                    if let Some(ref n) = self.disable_pending {
                        if *n == *name {
                            self.disable_pending = None;
                        }
                    }
                    self.disable_pending = self.disable_pending.take().or(old);
                }
                IrStmt::SysCall {
                    name,
                    args: ir_args,
                } => {
                    if name == "display" || name == "write" {
                        let msg = format_display(
                            &self.state,
                            &self.design.top.signals,
                            &self.design.hier_signal_map,
                            &self.assoc_data,
                            ir_args,
                        );
                        print!("{}", msg);
                    } else if name == "strobe" {
                        self.strobe_events.push(ir_args.clone());
                    } else if name == "fstrobe" {
                        let handle = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            self.fstrobe_events.push((h, ir_args[1..].to_vec()));
                        }
                    } else if name == "fmonitor" {
                        let handle = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            let vals: Vec<LogicVec> = ir_args[1..]
                                .iter()
                                .map(|a| self.evaluate_expr(a).unwrap_or(LogicVec::from_u64(0, 32)))
                                .collect();
                            self.fmonitor_map.insert(h, (ir_args[1..].to_vec(), vals));
                        }
                    } else if name == "monitor" {
                        let vals: Vec<LogicVec> = ir_args
                            .iter()
                            .map(|a| self.evaluate_expr(a).unwrap_or(LogicVec::from_u64(0, 32)))
                            .collect();
                        self.monitor_args = Some(ir_args.clone());
                        self.monitor_last_values = Some(vals);
                    } else if name == "urandom" {
                        let val: u32 = self.rng.gen();
                        let sig_id = ir_args.first().and_then(|a| {
                            if let IrExpr::Signal(id, _) = a {
                                Some(*id)
                            } else {
                                None
                            }
                        });
                        if let Some(sid) = sig_id {
                            self.state
                                .write_signal(sid, LogicVec::from_u64(val as u64, 32));
                        }
                    } else if name == "urandom_range" {
                        let args_eval: Vec<LogicVec> = ir_args
                            .iter()
                            .map(|a| self.evaluate_expr(a).unwrap_or(LogicVec::from_u64(0, 32)))
                            .collect();
                        let maxval = args_eval.first().map(|v| v.to_u64()).unwrap_or(0);
                        let minval = args_eval.get(1).map(|v| v.to_u64()).unwrap_or(0);
                        let val = if maxval <= minval {
                            minval
                        } else {
                            let range = maxval - minval + 1;
                            if range <= 1 {
                                minval
                            } else {
                                minval + (self.rng.gen::<u64>() % range)
                            }
                        };
                        let sig_id = ir_args.first().and_then(|a| {
                            if let IrExpr::Signal(id, _) = a {
                                Some(*id)
                            } else {
                                None
                            }
                        });
                        if let Some(sid) = sig_id {
                            self.state.write_signal(sid, LogicVec::from_u64(val, 32));
                        }
                    } else if name == "random" {
                        // If seed argument provided (second arg after dest signal),
                        // reseed RNG for reproducibility
                        if let Some(seed_arg) = ir_args.get(1) {
                            if let Ok(seed_val) = self.evaluate_expr(seed_arg) {
                                let seed = seed_val.to_u64();
                                self.rng = rand::rngs::StdRng::seed_from_u64(seed);
                            }
                        }
                        let val: i32 = self.rng.gen();
                        let sig_id = ir_args.first().and_then(|a| {
                            if let IrExpr::Signal(id, _) = a {
                                Some(*id)
                            } else {
                                None
                            }
                        });
                        if let Some(sid) = sig_id {
                            self.state
                                .write_signal(sid, LogicVec::from_u64(val as u64, 32));
                        }
                    } else if name == "dumpfile" {
                        if let Some(IrExpr::String(fname)) = ir_args.first() {
                            let path = fname.clone();
                            let design = &self.design;
                            let state = &self.state.signals;
                            if let Some(ref mut vcd) = self.vcd {
                                let _ = vcd.reopen(&path, design, state);
                            } else {
                                match VcdWriter::new(&path, design) {
                                    Ok(v) => self.vcd = Some(v),
                                    Err(e) => eprintln!("VCD: cannot create '{}': {}", path, e),
                                }
                            }
                        }
                    } else if name == "dumpall" {
                        if let Some(ref mut vcd) = self.vcd {
                            vcd.write_time_header(self.state.time)?;
                            let design = &self.design;
                            let state = &self.state.signals;
                            vcd.dump_all(design, state)?;
                        }
                    } else if name == "dumplimit" {
                        if let Some(limit) = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64()))
                        {
                            if let Some(ref mut vcd) = self.vcd {
                                vcd.max_dump_size = Some(limit);
                            }
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
                        let fname = ir_args.first().and_then(|a| {
                            if let IrExpr::String(s) = a {
                                Some(s.clone())
                            } else {
                                None
                            }
                        });
                        if let Some(fname) = fname {
                            let mode = ir_args.get(1).and_then(|a| {
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
                                    let sig_id = ir_args.get(1).and_then(|a| {
                                        if let IrExpr::Signal(id, _) = a {
                                            Some(*id)
                                        } else {
                                            None
                                        }
                                    });
                                    if let Some(sid) = sig_id {
                                        self.state.write_signal(
                                            sid,
                                            LogicVec::from_u64(handle as u64, 32),
                                        );
                                    }
                                }
                                Err(_) => {
                                    let sig_id = ir_args.get(1).and_then(|a| {
                                        if let IrExpr::Signal(id, _) = a {
                                            Some(*id)
                                        } else {
                                            None
                                        }
                                    });
                                    if let Some(sid) = sig_id {
                                        self.state.write_signal(sid, LogicVec::from_u64(0, 32));
                                    }
                                }
                            }
                        }
                    } else if name == "fdisplay" {
                        let handle = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let msg = format_display(
                                    &self.state,
                                    &self.design.top.signals,
                                    &self.design.hier_signal_map,
                                    &self.assoc_data,
                                    &ir_args[1..],
                                );
                                let _ = write!(f, "{}", msg);
                            }
                        }
                    } else if name == "fwrite" {
                        let handle = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let msg = format_display(
                                    &self.state,
                                    &self.design.top.signals,
                                    &self.design.hier_signal_map,
                                    &self.assoc_data,
                                    &ir_args[1..],
                                );
                                let _ = write!(f, "{}", msg);
                            }
                        }
                    } else if name == "fscanf" {
                        let handle = ir_args
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
                                let fmt = ir_args.get(1).and_then(|a| {
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
                                                            if let Some(arg) = ir_args.get(out_idx)
                                                            {
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
                                }
                            }
                        }
                    } else if name == "fread" {
                        let target = ir_args.first().and_then(|a| {
                            if let IrExpr::Signal(id, _) = a {
                                Some(*id)
                            } else {
                                None
                            }
                        });
                        let src = ir_args.get(1);
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
                    } else if name == "fclose" {
                        let handle = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            self.file_handles.remove(&h);
                            self.file_read_pos.remove(&h);
                        }
                    } else if name == "fflush" {
                        let handle = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let _ = f.flush();
                            }
                        }
                    } else if name == "__dpi_stmt" {
                        if let Some(arg) = ir_args.first() {
                            self.evaluate_expr(arg)?;
                        }
                    } else if name == "value$plusargs" {
                        let pattern = ir_args
                            .first()
                            .and_then(|a| self.evaluate_expr(a).ok())
                            .map(|v| logicvec_to_string(&v))
                            .unwrap_or_default();
                        let plusarg_name = pattern
                            .split('%')
                            .next()
                            .unwrap_or(&pattern)
                            .trim_end_matches('=');
                        let plusargs = self.plusargs.clone();
                        for (key, val) in &plusargs {
                            if key == plusarg_name {
                                if let Some(var_arg) = ir_args.get(1) {
                                    let num = if let Some(hex) =
                                        val.strip_prefix("0x").or_else(|| val.strip_prefix("0X"))
                                    {
                                        u64::from_str_radix(hex, 16).unwrap_or(0)
                                    } else {
                                        val.parse::<u64>().unwrap_or(0)
                                    };
                                    if let IrExpr::Signal(id, _) = var_arg {
                                        self.state.write_signal(*id, LogicVec::from_u64(num, 32));
                                    }
                                }
                                break;
                            }
                        }
                    } else if name == "test$plusargs" {
                    } else {
                        eprintln!("warning: unknown system call '{}' ignored", name);
                    }
                }
                IrStmt::SysFinish => {
                    self.running = false;
                    return Ok(());
                }
                IrStmt::Case {
                    case_type,
                    expr: case_expr,
                    items,
                    default,
                } => {
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
                                if self.disable_pending.is_some() {
                                    return Ok(());
                                }
                                item_matched = true;
                                matched = true;
                                break;
                            }
                        }
                        if item_matched {
                            break;
                        }
                    }
                    if !matched && !default.is_empty() {
                        self.evaluate_stmt_block(default)?;
                    }
                }
                IrStmt::Null => {}
                IrStmt::Assert {
                    cond,
                    pass_stmt,
                    fail_stmt,
                    clock_event,
                    disable_iff,
                    sequence: _,
                } => {
                    let should_check = match clock_event {
                        Some(ref ce) => self.check_concurrent_clock_event(ce),
                        None => true,
                    };
                    if should_check {
                        let disabled = match disable_iff {
                            Some(ref di) => self.evaluate_expr(di)?.to_bool().unwrap_or(false),
                            None => false,
                        };
                        if !disabled {
                            let ok = self.evaluate_expr(cond)?.to_bool().unwrap_or(false);
                            if ok {
                                if !pass_stmt.is_empty() {
                                    self.evaluate_stmt_block(pass_stmt)?;
                                }
                            } else {
                                eprintln!("assertion failed");
                                if !fail_stmt.is_empty() {
                                    self.evaluate_stmt_block(fail_stmt)?;
                                }
                            }
                        }
                    }
                }
                IrStmt::Assume {
                    cond,
                    pass_stmt,
                    fail_stmt,
                    clock_event,
                    disable_iff,
                    sequence: _,
                } => {
                    let should_check = match clock_event {
                        Some(ref ce) => self.check_concurrent_clock_event(ce),
                        None => true,
                    };
                    if should_check {
                        let disabled = match disable_iff {
                            Some(ref di) => self.evaluate_expr(di)?.to_bool().unwrap_or(false),
                            None => false,
                        };
                        if !disabled {
                            let ok = self.evaluate_expr(cond)?.to_bool().unwrap_or(false);
                            if ok {
                                if !pass_stmt.is_empty() {
                                    self.evaluate_stmt_block(pass_stmt)?;
                                }
                            } else {
                                eprintln!("assumption violated");
                                if !fail_stmt.is_empty() {
                                    self.evaluate_stmt_block(fail_stmt)?;
                                }
                            }
                        }
                    }
                }
                IrStmt::Cover {
                    cond,
                    pass_stmt,
                    clock_event,
                    disable_iff,
                    sequence: _,
                } => {
                    let should_check = match clock_event {
                        Some(ref ce) => self.check_concurrent_clock_event(ce),
                        None => true,
                    };
                    if should_check {
                        let disabled = match disable_iff {
                            Some(ref di) => self.evaluate_expr(di)?.to_bool().unwrap_or(false),
                            None => false,
                        };
                        if !disabled {
                            let ok = self.evaluate_expr(cond)?.to_bool().unwrap_or(false);
                            if ok {
                                eprintln!("cover point hit");
                                if !pass_stmt.is_empty() {
                                    self.evaluate_stmt_block(pass_stmt)?;
                                }
                            }
                        }
                    }
                }
                IrStmt::Break => {
                    self.control_flow = Some(FlowControl::Break);
                    return Ok(());
                }
                IrStmt::Continue => {
                    self.control_flow = Some(FlowControl::Continue);
                    return Ok(());
                }
                IrStmt::LoopFor {
                    init,
                    cond,
                    step,
                    body,
                } => {
                    if let Some(init_stmt) = init {
                        self.evaluate_stmt_block(&[*init_stmt.clone()])?;
                    }
                    let mut iter_count = 0usize;
                    loop {
                        if iter_count >= MAX_LOOP_ITER {
                            eprintln!(
                                "warning: for loop exceeded {} iterations, breaking",
                                MAX_LOOP_ITER
                            );
                            break;
                        }
                        iter_count += 1;
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if self.control_flow.is_some() {
                            self.control_flow = None;
                            break;
                        }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) {
                            break;
                        }
                        self.evaluate_stmt_block(body)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            if let Some(step_stmt) = step {
                                self.evaluate_stmt_block(&[*step_stmt.clone()])?;
                            }
                            continue;
                        }
                        if cf == Some(FlowControl::Break) {
                            break;
                        }
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if let Some(step_stmt) = step {
                            self.evaluate_stmt_block(&[*step_stmt.clone()])?;
                        }
                    }
                }
                IrStmt::LoopWhile { cond, body } => {
                    let mut iter_count = 0usize;
                    loop {
                        if iter_count >= MAX_LOOP_ITER {
                            eprintln!(
                                "warning: while loop exceeded {} iterations, breaking",
                                MAX_LOOP_ITER
                            );
                            break;
                        }
                        iter_count += 1;
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if self.control_flow.is_some() {
                            self.control_flow = None;
                            break;
                        }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) {
                            break;
                        }
                        self.evaluate_stmt_block(body)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            continue;
                        }
                        if cf == Some(FlowControl::Break) {
                            break;
                        }
                    }
                }
                IrStmt::LoopDoWhile { cond, body } => {
                    let mut iter_count = 0usize;
                    loop {
                        if iter_count >= MAX_LOOP_ITER {
                            eprintln!(
                                "warning: do-while loop exceeded {} iterations, breaking",
                                MAX_LOOP_ITER
                            );
                            break;
                        }
                        iter_count += 1;
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if self.control_flow.is_some() {
                            self.control_flow = None;
                            break;
                        }
                        self.evaluate_stmt_block(body)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            continue;
                        }
                        if cf == Some(FlowControl::Break) {
                            break;
                        }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) {
                            break;
                        }
                    }
                }
                IrStmt::Repeat { count, body } => {
                    let count_val = self.evaluate_expr(count)?;
                    let n = (count_val.to_u64() as usize).min(MAX_LOOP_ITER);
                    for _ in 0..n {
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if self.control_flow.is_some() {
                            self.control_flow = None;
                            break;
                        }
                        self.evaluate_stmt_block(body)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            continue;
                        }
                        if cf == Some(FlowControl::Break) {
                            break;
                        }
                    }
                }
                IrStmt::RandCase { items } => {
                    let total: u64 = items
                        .iter()
                        .map(|(w_expr, _)| {
                            self.evaluate_expr(w_expr)
                                .unwrap_or(LogicVec::from_u64(1, 32))
                                .to_u64()
                        })
                        .sum();
                    if total > 0 {
                        let r = self.rng.gen::<u64>() % total;
                        let mut cumulative = 0u64;
                        for (w_expr, body) in items {
                            let weight = self
                                .evaluate_expr(w_expr)
                                .unwrap_or(LogicVec::from_u64(1, 32))
                                .to_u64();
                            cumulative += weight;
                            if r < cumulative {
                                self.evaluate_stmt_block(body)?;
                                break;
                            }
                        }
                    }
                }
                IrStmt::RandSequence { productions } => {
                    if let Some((_, items)) = productions.first() {
                        let total: u64 = items
                            .iter()
                            .map(|(w, _)| {
                                self.evaluate_expr(w)
                                    .unwrap_or(LogicVec::from_u64(1, 32))
                                    .to_u64()
                            })
                            .sum();
                        if total > 0 {
                            let r = self.rng.gen::<u64>() % total;
                            let mut acc = 0u64;
                            for (w, body) in items {
                                acc += self
                                    .evaluate_expr(w)
                                    .unwrap_or(LogicVec::from_u64(1, 32))
                                    .to_u64();
                                if r < acc {
                                    self.evaluate_stmt_block(body)?;
                                    break;
                                }
                            }
                        }
                    }
                }
                IrStmt::Foreach {
                    array_var,
                    index_var,
                    body,
                } => {
                    let lv = self.evaluate_expr(array_var)?;
                    let sig_info = if let IrExpr::Signal(id, _) = array_var {
                        self.design.top.signals.get(*id)
                    } else {
                        None
                    };
                    let elem_width = sig_info.map(|s| s.elem_width).unwrap_or(1);
                    let count = if elem_width > 0 {
                        lv.width / elem_width
                    } else {
                        0
                    };
                    for i in 0..count {
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if self.control_flow.is_some() {
                            self.control_flow = None;
                            break;
                        }
                        let idx_val = LogicVec::from_u64(i as u64, 32);
                        let mut scope = HashMap::new();
                        scope.insert(index_var.clone(), idx_val);
                        let depth = self.method_locals.len();
                        self.method_locals.push(scope);
                        self.evaluate_stmt_block(body)?;
                        self.method_locals.truncate(depth);
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            continue;
                        }
                        if cf == Some(FlowControl::Break) {
                            break;
                        }
                    }
                }
                IrStmt::MethodCallStmt {
                    obj,
                    method,
                    args,
                    with_clause,
                } => {
                    if let IrExpr::Signal(id, _) = obj {
                        let sig_info = self.design.top.signals.get(*id).cloned();
                        if let Some(ref sig) = sig_info {
                            if sig.is_dynamic || sig.is_queue || sig.is_associative {
                                let _ =                                    self.evaluate_array_method(
                                        *id,
                                        sig,
                                        method.as_str(),
                                        args,
                                    with_clause.as_deref(),
                                )?;
                                continue;
                            }
                            if let Some(ref cn) = sig.class_name {
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
                                        self.execute_method(new_id, method.as_str(), &arg_vals)?;
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    let obj_val = self.evaluate_expr(obj)?;
                    let obj_id = obj_val.to_u64() as ObjId;
                    let arg_vals: Vec<LogicVec> = args
                        .iter()
                        .map(|a| self.evaluate_expr(a))
                        .collect::<Result<_, _>>()?;
                    self.execute_method(obj_id, method.as_str(), &arg_vals)?;
                }
                IrStmt::Delay { delay, body } => {
                    let delay_val = *delay as usize;
                    let delay_t = self.state.time as usize + delay_val;
                    if delay_t < self.events.len() {
                        let mut later: Vec<IrStmt> = body.clone();
                        let remaining: Vec<IrStmt> = stmts[i + 1..].to_vec();
                        later.extend(remaining);
                        if !later.is_empty() {
                            let region = if delay_val == 0 {
                                EventRegion::Inactive
                            } else {
                                EventRegion::Active
                            };
                            let pid = self.current_process_id;
                            self.events[delay_t].push(RegionEvent {
                                region,
                                event: EventKind::ContinueBlock(Continuation {
                                    stmts_to_exec: later,
                                    stmts_remaining: vec![],
                                    fork_id: None,
                                    process_id: pid,
                                }),
                            });
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
                IrStmt::WaitOrder {
                    events,
                    failure_stmts,
                } => {
                    let continuation: Vec<IrStmt> = stmts[i + 1..].to_vec();
                    self.pending_wait_orders.push(WaitOrderState {
                        events: events.clone(),
                        expected_idx: 0,
                        continuation,
                        failure_stmts: failure_stmts.clone(),
                    });
                    return Ok(());
                }
                IrStmt::Disable { name } => {
                    self.disable_pending = Some(name.clone());
                    return Ok(());
                }
                IrStmt::Release { lvalue } => {
                    if let Some(id) = self.signal_id_from_lvalue(lvalue) {
                        self.forced_signals.remove(&id);
                    }
                }
                IrStmt::Deassign { lvalue } => {
                    if let Some(id) = self.signal_id_from_lvalue(lvalue) {
                        self.forced_signals.remove(&id);
                    }
                }
                IrStmt::Fork {
                    processes,
                    join_type,
                } => {
                    let fid = self.fork_groups.len();
                    let remaining: Vec<IrStmt> = stmts[i + 1..].to_vec();
                    let count = processes.len();
                    self.fork_groups.push(ForkGroup {
                        remaining: count,
                        continuation: remaining.clone(),
                    });
                    match join_type {
                        IrJoinType::Join => {
                            for p in processes {
                                if p.is_empty() {
                                    if self.fork_groups[fid].remaining > 0 {
                                        self.fork_groups[fid].remaining -= 1;
                                    }
                                } else {
                                    let all_consumed =
                                        self.evaluate_block_with_delay_fork(&p, Some(fid))?;
                                    if all_consumed && self.fork_groups[fid].remaining > 0 {
                                        self.fork_groups[fid].remaining -= 1;
                                    }
                                }
                            }
                            if self.fork_groups[fid].remaining == 0 && !remaining.is_empty() {
                                let group = self.fork_groups[fid].clone();
                                self.evaluate_stmt_block(&group.continuation)?;
                            }
                        }
                        IrJoinType::JoinAny => {
                            self.fork_groups[fid].remaining = 1;
                            let mut any_immediate = false;
                            for p in processes {
                                if p.is_empty() {
                                    any_immediate = true;
                                } else {
                                    let all_consumed =
                                        self.evaluate_block_with_delay_fork(&p, Some(fid))?;
                                    if all_consumed {
                                        any_immediate = true;
                                    }
                                }
                            }
                            if any_immediate && self.fork_groups[fid].remaining > 0 {
                                self.fork_groups[fid].remaining -= 1;
                            }
                            if self.fork_groups[fid].remaining == 0 && !remaining.is_empty() {
                                let group = self.fork_groups[fid].clone();
                                self.evaluate_stmt_block(&group.continuation)?;
                            }
                        }
                        IrJoinType::JoinNone => {
                            for p in processes {
                                if !p.is_empty() {
                                    self.evaluate_block_with_delay_fork(&p, Some(fid))?;
                                }
                            }
                            if !remaining.is_empty() {
                                self.evaluate_stmt_block(&remaining)?;
                            }
                        }
                    }
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    pub(crate) fn process_pending_waits(&mut self, deltas: &[SignalId]) -> Result<bool, SimError> {
        let mut matched = false;
        let mut remaining = Vec::new();
        let waits = std::mem::take(&mut self.pending_waits);
        for (deps, stmts) in waits {
            if deltas.iter().any(|d| deps.contains(d)) {
                matched = true;
                self.evaluate_block_with_delay(&stmts)?;
            } else {
                remaining.push((deps, stmts));
            }
        }
        for item in remaining {
            self.pending_waits.push(item);
        }
        Ok(matched)
    }

    pub(crate) fn process_pending_wait_orders(&mut self, deltas: &[SignalId]) -> Result<bool, SimError> {
        let mut any_done = false;
        let mut remaining = Vec::new();
        let orders = std::mem::take(&mut self.pending_wait_orders);
        'order: for mut order in orders {
            let mut changed_in_order = Vec::new();
            for d in deltas {
                if let Some(pos) = order.events.iter().position(|e| e == d) {
                    changed_in_order.push(pos);
                }
            }
            changed_in_order.sort();
            for &pos in &changed_in_order {
                if pos == order.expected_idx {
                    order.expected_idx += 1;
                    if order.expected_idx == order.events.len() {
                        if !order.continuation.is_empty() {
                            self.evaluate_block_with_delay(&order.continuation)?;
                        }
                        any_done = true;
                        continue 'order;
                    }
                } else if pos > order.expected_idx {
                    if !order.failure_stmts.is_empty() {
                        self.evaluate_stmt_block(&order.failure_stmts)?;
                    }
                    any_done = true;
                    continue 'order;
                }
            }
            remaining.push(order);
        }
        for item in remaining {
            self.pending_wait_orders.push(item);
        }
        Ok(any_done)
    }

    pub(crate) fn trigger_sensitive_processes(
        &mut self,
        changed: &[(usize, LogicVec, LogicVec)],
        _t: usize,
    ) -> Result<(), SimError> {
        let processes = self.design.top.processes.clone();

        // Collect triggered combinational processes for potential parallel execution
        // Skip fused processes — they're evaluated as part of clock domain fusion
        let mut comb_indices: Vec<usize> = Vec::new();
        for (pid, process) in processes.iter().enumerate() {
            if let Process::Combinational { sensitivity, .. } = process {
                // Skip if this process is fused into a clock domain
                if self.use_cycle_fusion
                    && self.clock_analysis.as_ref()
                        .map(|a| a.fused_processes.contains(&pid))
                        .unwrap_or(false)
                {
                    continue;
                }
                let should_trigger = sensitivity.is_empty()
                    || changed.iter().any(|(id, _, _)| sensitivity.contains(id));
                if should_trigger {
                    comb_indices.push(pid);
                }
            }
        }

        // If enough processes to parallelize and config allows it, use parallel eval
        if comb_indices.len() >= self.parallel_config.min_processes_parallel
            && self.parallel_config.parallel_processes
        {
            use rayon::prelude::*;
            let signal_count = self.state.signals.len();
            let snapshot: Vec<LogicVec> = (0..signal_count)
                .map(|i| self.state.read_signal(i).clone())
                .collect();
            let results: Vec<Result<Vec<(SignalId, LogicVec)>, SimError>> = comb_indices
                .par_iter()
                .map(|&pid| {
                    let process = &processes[pid];
                    if let Process::Combinational { body, .. } = process {
                        let mut local_signals = snapshot.clone();
                        let mut writes = Vec::new();
                        match parallel::evaluate_stmt_block_parallel(
                            body,
                            &mut local_signals,
                            &mut writes,
                        ) {
                            Ok(()) => {
                                // Apply writes from parallel eval
                                Ok(writes)
                            }
                            Err(e) => Err(SimError::runtime(format!("parallel eval error: {}", e))),
                        }
                    } else {
                        Ok(Vec::new())
                    }
                })
                .collect();

            for result in results {
                let writes = result?;
                for (sig_id, val) in writes {
                    self.state.write_signal(sig_id, val);
                }
            }
        } else {
            // Sequential path: evaluate triggered comb processes inline
            for &pid in &comb_indices {
                let process = &processes[pid];
                if let Process::Combinational { body, .. } = process {
                    self.evaluate_stmt_block(body)?;
                }
            }
        }

        // Handle CombReactive, Sequential, and other process types (always sequential)
        for (pid, process) in processes.iter().enumerate() {
            match process {
                Process::CombReactive { sensitivity, .. } => {
                    let should_trigger = sensitivity.is_empty()
                        || changed.iter().any(|(id, _, _)| sensitivity.contains(id));
                    if should_trigger {
                        self.reactive_events.push(EventKind::EvalProcess(pid));
                    }
                }
                Process::Sequential {
                    clock,
                    reset: _reset,
                    body,
                    ..
                } => {
                    let trigger = match clock {
                        ClockEdge::PosEdge(sig_id) => changed.iter().any(|(id, old, new)| {
                            id == sig_id
                                && old.to_bool() != Some(true)
                                && new.to_bool() == Some(true)
                        }),
                        ClockEdge::NegEdge(sig_id) => changed.iter().any(|(id, old, new)| {
                            id == sig_id
                                && old.to_bool() != Some(false)
                                && new.to_bool() == Some(false)
                        }),
                    };
                    if trigger {
                        // ── Cycle-Based Fusion: jika process ini termasuk fused domain,
                        // evaluasi SEMUA process dalam domain sekaligus (sequential + follower comb).
                        // Skip event queue overhead untuk process sinkronus murni. ──
                        // Clone domain upfront untuk hindari borrow conflict
                        // antara self.clock_analysis (immutable) dan self.evaluate_clock_domain (&mut).
                        let fused_domain = if self.use_cycle_fusion {
                            self.clock_analysis.as_ref()
                                .and_then(|a| {
                                    if a.fused_processes.contains(&pid) {
                                        a.domains.iter().find(|d| d.sequential_processes.contains(&pid)).cloned()
                                    } else {
                                        None
                                    }
                                })
                        } else {
                            None
                        };
                        if let Some(domain) = fused_domain {
                            self.evaluate_clock_domain(&domain)?;
                            continue; // Skip individual eval
                        }
                        // Fallback: evaluate only this sequential process
                        self.evaluate_stmt_block(body)?;
                    }
                }
                // Skip fused combinational/reactive processes — they're evaluated
                // as part of their clock domain's follower set
                Process::Combinational { .. } | Process::CombReactive { .. }
                    if self.use_cycle_fusion
                    && self.clock_analysis.as_ref()
                        .map(|a| a.fused_processes.contains(&pid))
                        .unwrap_or(false) => {}
                Process::CombReactive { sensitivity, .. } => {
                    let should_trigger = sensitivity.is_empty()
                        || changed.iter().any(|(id, _, _)| sensitivity.contains(id));
                    if should_trigger {
                        self.reactive_events.push(EventKind::EvalProcess(pid));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn commit_nba(&mut self) {
        let pending = std::mem::take(&mut self.nba_pending);
        for (lvalue, val) in pending {
            if !self.is_forced(&lvalue) {
                let _ = self.write_lvalue(&lvalue, val);
            }
        }
    }

    pub(crate) fn signal_id_from_lvalue(&self, lvalue: &IrLValue) -> Option<SignalId> {
        match lvalue {
            IrLValue::Signal(id, _) => Some(*id),
            IrLValue::RangeSelect(id, _, _) => Some(*id),
            IrLValue::BitSelect(id, _) => Some(*id),
            IrLValue::ArrayIndex { sig_id, .. } => Some(*sig_id),
            IrLValue::ArrayRangeSelect { sig_id, .. } => Some(*sig_id),
            IrLValue::ArrayBitSelect { sig_id, .. } => Some(*sig_id),
            IrLValue::Concat(_) => None,
        }
    }

    fn is_forced(&self, lvalue: &IrLValue) -> bool {
        self.signal_id_from_lvalue(lvalue)
            .map_or(false, |id| self.forced_signals.contains(&id))
    }

}
