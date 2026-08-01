use super::super::SequenceAttempt;
use super::super::SimulationEngine;
use crate::simulator::util::*;
use crate::error::SimError;
use crate::ir::*;
use crate::Symbol;
use crate::simulator::types::*;
use rand::Rng;

/// Cap iterasi loop AST (method/task context) untuk mencegah hang ketika loop
/// berisi blocking event (`@(...)`) yang tidak memajukan waktu simulasi.
const AST_LOOP_ITER_CAP: u64 = 10_000_000;

impl SimulationEngine {
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
            // Line coverage: record every statement execution (also covers fork path)
            if self.coverage_enabled {
                if let Some(pname) = self.current_process_name.clone() {
                    self.record_line_hit(stmt, &pname);
                }
            }
            match stmt {
                IrStmt::Block { stmts: inner } => {
                    if !self.evaluate_block_fork(inner, fork_id)? {
                        return Ok(false);
                    }
                }
                IrStmt::NamedBlock {
                    name, stmts: inner, ..
                } => {
                    if !self.evaluate_named_block_fork(*name, inner, fork_id)? {
                        return Ok(false);
                    }
                }
                IrStmt::If {
                    cond,
                    true_branch: then_stmts,
                    false_branch: else_stmts,
                } => {
                    if !self.evaluate_if_fork(cond, then_stmts, else_stmts, fork_id)? {
                        return Ok(false);
                    }
                }
                IrStmt::Case {
                    case_type,
                    expr: case_expr,
                    items,
                    default,
                } => {
                    if !self.evaluate_case_fork(case_type, case_expr, items, default, fork_id)? {
                        return Ok(false);
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
                    self.ensure_events(delay_t);
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
                            self.push_event(delay_t, RegionEvent {
                                region,
                                event: EventKind::ContinueBlock(Continuation {
                                    stmts_to_exec: later,
                                    stmts_remaining: vec![],
                                    fork_id,
                                    process_id: pid,
                                }),
                            });
                        }
                    return Ok(false);
                }
                IrStmt::EventControl { sigs, body } => {
                    // Blocking event control `@(a or posedge b)`:
                    // SELALU suspend — tunggu perubahan/edge berikutnya. Event
                    // yang sudah lewat (mis. clk sudah high) TIDAK dihitung.
                    // SATU entry mewakili SATU `@(...)` — fire sekali.
                    let mut later: Vec<IrStmt> = body.clone();
                    later.extend(stmts[i + 1..].to_vec());
                    if let Some(lc) = &self.loop_continuation {
                        later.extend(lc.clone());
                    }
                    let armed_vals: Vec<LogicVec> = sigs
                        .iter()
                        .map(|(sid, _)| self.state.read_signal(*sid).clone())
                        .collect();
                    self.pending_events.push(PendingEventControl {
                        sigs: sigs.clone(),
                        armed_vals,
                        continuation: later,
                    });
                    return Ok(false);
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
                                continue;
                            }
                            // System function lain (mis. $display/$write) yang
                            // dibungkus elaborator sbg SysCall{name:"", args:[...]}:
                            // dispatch dgn nama asli (tanpa '$') biar tak tercecer.
                            let dispatch_name = fn_name.as_str().trim_start_matches('$');
                            self.evaluate_syscall(dispatch_name, fn_args, fork_id, stmts, i)?;
                            continue;
                        }
                        continue;
                    }
                    self.evaluate_syscall(name.as_str(), ir_args, fork_id, stmts, i)?;
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
                    self.evaluate_break_fork()?;
                    return Ok(true);
                }
                IrStmt::Continue => {
                    self.evaluate_continue_fork()?;
                    return Ok(true);
                }
                IrStmt::LoopFor {
                    init,
                    cond,
                    step,
                    body,
                } => {
                    if !self.evaluate_loop_for_fork(init, cond, step, body, fork_id)? {
                        return Ok(false);
                    }
                }
                IrStmt::LoopWhile { cond, body } => {
                    let saved_tail = std::mem::take(&mut self.post_loop_tail);
                    self.post_loop_tail = stmts[i + 1..].to_vec();
                    let completed = self.evaluate_loop_while_fork(cond, body, fork_id)?;
                    if !completed {
                        self.post_loop_tail = saved_tail;
                        return Ok(false);
                    }
                    self.post_loop_tail = saved_tail;
                }
                IrStmt::LoopDoWhile { cond, body } => {
                    let saved_tail = std::mem::take(&mut self.post_loop_tail);
                    self.post_loop_tail = stmts[i + 1..].to_vec();
                    let completed = self.evaluate_loop_do_while_fork(cond, body, fork_id)?;
                    if !completed {
                        self.post_loop_tail = saved_tail;
                        return Ok(false);
                    }
                    self.post_loop_tail = saved_tail;
                }
                IrStmt::Repeat { count, body } => {
                    if !self.evaluate_repeat_fork(count, body, fork_id)? {
                        return Ok(false);
                    }
                }
                IrStmt::Foreach {
                    array_var,
                    index_var,
                    body,
                } => {
                    if !self.evaluate_foreach_fork(array_var, index_var, body, fork_id)? {
                        return Ok(false);
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
                                        && !self.state.objects.is_empty()
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
                    let remaining: Vec<IrStmt> = stmts[i + 1..].to_vec();
                    let count = processes.len();
                    // JoinNone tidak perlu menyimpan continuation (dieksekusi
                    // sekali di sini) → hindari clone sisa statement.
                    let cont = if matches!(join_type, IrJoinType::JoinNone) {
                        Vec::new()
                    } else {
                        remaining.clone()
                    };
                    let reclaimable = !matches!(join_type, IrJoinType::JoinAny);
                    let fid = self.alloc_fork_group(count, cont, reclaimable);
                    match join_type {
                        IrJoinType::Join => {
                            for p in processes {
                                if p.is_empty() {
                                    self.fork_decrement(fid)?;
                                } else {
                                    let all_consumed =
                                        self.evaluate_block_with_delay_fork(p, Some(fid))?;
                                    if all_consumed {
                                        self.fork_decrement(fid)?;
                                    }
                                }
                            }
                            self.fork_finish(fid)?;
                        }
                        IrJoinType::JoinAny => {
                            self.fork_groups[fid].remaining = 1;
                            let mut any_immediate = false;
                            for p in processes {
                                if p.is_empty() {
                                    any_immediate = true;
                                } else {
                                    let all_consumed =
                                        self.evaluate_block_with_delay_fork(p, Some(fid))?;
                                    if all_consumed {
                                        any_immediate = true;
                                    }
                                }
                            }
                            if any_immediate {
                                self.fork_decrement(fid)?;
                            }
                            self.fork_finish(fid)?;
                        }
                        IrJoinType::JoinNone => {
                            for p in processes {
                                if p.is_empty() {
                                    self.fork_decrement(fid)?;
                                } else {
                                    let all_consumed =
                                        self.evaluate_block_with_delay_fork(p, Some(fid))?;
                                    if all_consumed {
                                        self.fork_decrement(fid)?;
                                    }
                                }
                            }
                            // continuation join_none dieksekusi sekali sekarang
                            self.fork_groups[fid].fired = true;
                            if !remaining.is_empty() {
                                self.evaluate_block_with_delay_fork(&remaining, None)?;
                            }
                            self.fork_finish(fid)?;
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
                    if !self.evaluate_ast_block_with_delay_fork(inner, fork_id)? {
                        return Ok(false);
                    }
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
                    let completed = self.evaluate_ast_block_with_delay_fork(inner, fork_id)?;
                    if let Some(ref n) = self.disable_pending {
                        if *n == *name {
                            self.disable_pending = None;
                        }
                    }
                    self.disable_pending = self.disable_pending.take().or(old);
                    if !completed {
                        return Ok(false);
                    }
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
                        if !self.evaluate_ast_block_with_delay_fork(&[*true_branch.clone()], fork_id)? {
                            return Ok(false);
                        }
                    } else if let Some(fb) = false_branch {
                        if !self.evaluate_ast_block_with_delay_fork(&[*fb.clone()], fork_id)? {
                            return Ok(false);
                        }
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
                                if !self.evaluate_ast_block_with_delay_fork(
                                    &[*item.stmt.clone()],
                                    fork_id,
                                )? {
                                    return Ok(false);
                                }
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
                            if !self.evaluate_ast_block_with_delay_fork(&[*def.clone()], fork_id)? {
                                return Ok(false);
                            }
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
                                if !self.evaluate_ast_block_with_delay_fork(
                                    &[*item.stmt.clone()],
                                    fork_id,
                                )? {
                                    return Ok(false);
                                }
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
                            if !self.evaluate_ast_block_with_delay_fork(&[*def.clone()], fork_id)? {
                                return Ok(false);
                            }
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
                                if !self.evaluate_ast_block_with_delay_fork(
                                    &[*item.stmt.clone()],
                                    fork_id,
                                )? {
                                    return Ok(false);
                                }
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
                            if !self.evaluate_ast_block_with_delay_fork(&[*def.clone()], fork_id)? {
                                return Ok(false);
                            }
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
                    self.ast_loop_iters += 1;
                    if self.ast_loop_iters > AST_LOOP_ITER_CAP {
                        self.emit_warning(
                            crate::diagnostics::diagnostic::DiagCode::NotImplemented,
                            "AST while/forever loop exceeded iteration cap (blocking event in loop without time advance); breaking out to avoid hang",
                        );
                        break;
                    }
                    if !self.evaluate_ast_block_with_delay_fork(inner, fork_id)? {
                        break;
                    }
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
                    self.ast_loop_iters += 1;
                    if self.ast_loop_iters > AST_LOOP_ITER_CAP {
                        self.emit_warning(
                            crate::diagnostics::diagnostic::DiagCode::NotImplemented,
                            "AST while loop exceeded iteration cap (blocking event in loop without time advance); breaking out to avoid hang",
                        );
                        break;
                    }
                    let cond_val = self.evaluate_ast_expr(cond)?;
                    if !cond_val.to_bool().unwrap_or(false) {
                        break;
                    }
                    if !self.evaluate_ast_block_with_delay_fork(inner, fork_id)? {
                        break;
                    }
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
                    self.ast_loop_iters += 1;
                    if self.ast_loop_iters > AST_LOOP_ITER_CAP {
                        self.emit_warning(
                            crate::diagnostics::diagnostic::DiagCode::NotImplemented,
                            "AST do-while loop exceeded iteration cap (blocking event in loop without time advance); breaking out to avoid hang",
                        );
                        break;
                    }
                    if !self.evaluate_ast_block_with_delay_fork(inner, fork_id)? {
                        break;
                    }
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
                        if !self.evaluate_ast_block_with_delay_fork(&[*init_stmt.clone()], fork_id)? {
                            break;
                        }
                    }
                    loop {
                        if self.disable_pending.is_some() {
                            break;
                        }
                        if self.control_flow.is_some() {
                            self.control_flow = None;
                            break;
                        }
                        self.ast_loop_iters += 1;
                        if self.ast_loop_iters > AST_LOOP_ITER_CAP {
                            self.emit_warning(
                                crate::diagnostics::diagnostic::DiagCode::NotImplemented,
                                "AST for loop exceeded iteration cap (blocking event in loop without time advance); breaking out to avoid hang",
                            );
                            break;
                        }
                        if let Some(ref c) = cond {
                            let cv = self.evaluate_ast_expr(c)?;
                            if !cv.to_bool().unwrap_or(false) {
                                break;
                            }
                        }
                        if !self.evaluate_ast_block_with_delay_fork(inner, fork_id)? {
                            break;
                        }
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            if let Some(s) = step {
                                if !self.evaluate_ast_block_with_delay_fork(&[*s.clone()], fork_id)? {
                                    break;
                                }
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
                            if !self.evaluate_ast_block_with_delay_fork(&[*s.clone()], fork_id)? {
                                break;
                            }
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
                        if !self.evaluate_ast_block_with_delay_fork(inner, fork_id)? {
                            break;
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
                crate::ast::Stmt::Delay { delay, stmt: body } => {
                    let delay_val = self.evaluate_ast_expr(delay)?;
                    let d = delay_val.to_u64() as usize;
                    let delay_t = self.state.time as usize + d;
                    self.ensure_events(delay_t);
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
                        self.push_event(delay_t, RegionEvent {
                            region,
                            event: EventKind::ContinueAstBlock(remaining, fork_id),
                        });
                    return Ok(false);
                }
                crate::ast::Stmt::EventControl { events, stmt: body } => {
                    // Blocking event control di task/method UVM: daftarkan pending
                    // wake-up (dengan konteks method) lalu suspend. Event yang sudah
                    // lewat tidak dihitung. Signal yang tak ter-resolve (mis. vif
                    // tanpa clk di desain) → lanjut segera (tanpa block).
                    if events
                        .iter()
                        .any(|e| matches!(e, crate::ast::SensitivityEvent::Wildcard))
                    {
                        if let Some(b) = body {
                            self.evaluate_ast_block_with_delay_fork(&[*b.clone()], fork_id)?;
                        }
                        if i + 1 < stmts.len() {
                            self.evaluate_ast_block_with_delay_fork(&stmts[i + 1..], fork_id)?;
                        }
                        return Ok(true);
                    }
                    let mut sigs: Vec<(SignalId, Option<ClockEdge>)> = Vec::new();
                    for event in events {
                        match event {
                            crate::ast::SensitivityEvent::PosEdge(expr) => {
                                if let Some(id) = self.find_ast_signal_id(expr) {
                                    sigs.push((id, Some(ClockEdge::PosEdge(id))));
                                }
                            }
                            crate::ast::SensitivityEvent::NegEdge(expr) => {
                                if let Some(id) = self.find_ast_signal_id(expr) {
                                    sigs.push((id, Some(ClockEdge::NegEdge(id))));
                                }
                            }
                            crate::ast::SensitivityEvent::Level(expr) => {
                                if let Some(id) = self.find_ast_signal_id(expr) {
                                    sigs.push((id, None));
                                }
                            }
                            crate::ast::SensitivityEvent::Wildcard => unreachable!("handled above"),
                        }
                    }
                    if sigs.is_empty() {
                        return Ok(true);
                    }
                    let mut later: Vec<crate::ast::Stmt> = Vec::new();
                    if let Some(b) = body {
                        later.push(*b.clone());
                    }
                    later.extend(stmts[i + 1..].iter().cloned());
                    let base_len = self.method_locals.len();
                    let armed_vals: Vec<LogicVec> = sigs
                        .iter()
                        .map(|(sid, _)| self.state.read_signal(*sid).clone())
                        .collect();
                    self.pending_ast_events.push(PendingAstEventControl {
                        sigs,
                        armed_vals,
                        continuation: later,
                        this: self.current_this,
                        method: self.current_method,
                        locals: self.method_locals.clone(),
                        base_len,
                    });
                    return Ok(false);
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
                    self.disable_pending = Some(*name);
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
                        self.evaluate_ast_block_with_delay_fork(std::slice::from_ref(p), Some(fid))?;
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
            // Line coverage: record every statement execution
            // Clone pname first to avoid borrow conflict with self.record_line_hit
            if self.coverage_enabled {
                if let Some(pname) = self.current_process_name.clone() {
                    self.record_line_hit(stmt, &pname);
                }
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
                    self.evaluate_if_stmt(cond, then_stmts, else_stmts)?;
                }
                IrStmt::Block { stmts: inner } => {
                    self.evaluate_block_stmt(inner)?;
                }
                IrStmt::NamedBlock {
                    name, stmts: inner, ..
                } => {
                    self.evaluate_named_block_stmt(*name, inner)?;
                }
                IrStmt::SysCall {
                    name,
                    args: ir_args,
                } => {
                    if name.is_empty() {
                        if let Some(IrExpr::SysFunc {
                            name: fn_name,
                            args: fn_args,
                        }) = ir_args.first()
                        {
                            self.evaluate_syscall_stmt(
                                fn_name.as_str().trim_start_matches('$'),
                                fn_args,
                            )?;
                        }
                        continue;
                    }
                    self.evaluate_syscall_stmt(name.as_str(), ir_args)?;
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
                    self.evaluate_case_stmt(case_type, case_expr, items, default)?;
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
                    self.evaluate_break_stmt()?;
                    return Ok(());
                }
                IrStmt::Continue => {
                    self.evaluate_continue_stmt()?;
                    return Ok(());
                }
                IrStmt::LoopFor {
                    init,
                    cond,
                    step,
                    body,
                } => {
                    self.evaluate_loop_for_stmt(init, cond, step, body)?;
                }
                IrStmt::LoopWhile { cond, body } => {
                    self.evaluate_loop_while_stmt(cond, body)?;
                }
                IrStmt::LoopDoWhile { cond, body } => {
                    self.evaluate_loop_do_while_stmt(cond, body)?;
                }
                IrStmt::Repeat { count, body } => {
                    self.evaluate_repeat_stmt(count, body)?;
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
                    self.evaluate_foreach_stmt(array_var, index_var, body)?;
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
                                        && !self.state.objects.is_empty()
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
                    self.ensure_events(delay_t);
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
                            self.push_event(delay_t, RegionEvent {
                                region,
                                event: EventKind::ContinueBlock(Continuation {
                                    stmts_to_exec: later,
                                    stmts_remaining: vec![],
                                    fork_id: None,
                                    process_id: pid,
                                }),
                            });
                        }
                    return Ok(());
                }
                IrStmt::EventControl { sigs, body } => {
                    // Blocking event control (evaluate_stmt_block context):
                    // daftarkan pending wake-up dan berhenti memproses blok ini.
                    // SATU entry mewakili SATU `@(...)` — fire sekali.
                    let mut later: Vec<IrStmt> = body.clone();
                    later.extend(stmts[i + 1..].to_vec());
                    if let Some(lc) = &self.loop_continuation {
                        later.extend(lc.clone());
                    }
                    let armed_vals: Vec<LogicVec> = sigs
                        .iter()
                        .map(|(sid, _)| self.state.read_signal(*sid).clone())
                        .collect();
                    self.pending_events.push(PendingEventControl {
                        sigs: sigs.clone(),
                        armed_vals,
                        continuation: later,
                    });
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
                    self.disable_pending = Some(*name);
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
                    let remaining: Vec<IrStmt> = stmts[i + 1..].to_vec();
                    let count = processes.len();
                    let cont = if matches!(join_type, IrJoinType::JoinNone) {
                        Vec::new()
                    } else {
                        remaining.clone()
                    };
                    let reclaimable = !matches!(join_type, IrJoinType::JoinAny);
                    let fid = self.alloc_fork_group(count, cont, reclaimable);
                    match join_type {
                        IrJoinType::Join => {
                            for p in processes {
                                if p.is_empty() {
                                    self.fork_decrement(fid)?;
                                } else {
                                    let all_consumed =
                                        self.evaluate_block_with_delay_fork(p, Some(fid))?;
                                    if all_consumed {
                                        self.fork_decrement(fid)?;
                                    }
                                }
                            }
                            if self.fork_groups[fid].active
                                && self.fork_groups[fid].remaining == 0
                                && !remaining.is_empty()
                            {
                                self.fork_groups[fid].fired = true;
                                let cont = std::mem::take(&mut self.fork_groups[fid].continuation);
                                self.evaluate_stmt_block(&cont)?;
                                self.retire_fork_group(fid);
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
                                        self.evaluate_block_with_delay_fork(p, Some(fid))?;
                                    if all_consumed {
                                        any_immediate = true;
                                    }
                                }
                            }
                            if any_immediate {
                                self.fork_decrement(fid)?;
                            }
                            if self.fork_groups[fid].active
                                && self.fork_groups[fid].remaining == 0
                                && !remaining.is_empty()
                            {
                                self.fork_groups[fid].fired = true;
                                let cont = std::mem::take(&mut self.fork_groups[fid].continuation);
                                self.evaluate_stmt_block(&cont)?;
                            }
                            self.fork_groups[fid].continuation.clear();
                        }
                        IrJoinType::JoinNone => {
                            for p in processes {
                                if p.is_empty() {
                                    self.fork_decrement(fid)?;
                                } else {
                                    let all_consumed =
                                        self.evaluate_block_with_delay_fork(p, Some(fid))?;
                                    if all_consumed {
                                        self.fork_decrement(fid)?;
                                    }
                                }
                            }
                            self.fork_groups[fid].fired = true;
                            if !remaining.is_empty() {
                                self.evaluate_stmt_block(&remaining)?;
                            }
                            self.fork_finish(fid)?;
                        }
                    }
                    return Ok(());
                }
            }
        }
        Ok(())
    }

}
