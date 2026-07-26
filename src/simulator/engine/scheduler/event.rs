use super::super::SequenceAttempt;
use super::super::SimulationEngine;
use super::super::MAX_LOOP_ITER;
use crate::error::SimError;
use crate::ir::*;
use crate::ast::*;
use crate::Symbol;
use crate::simulator::state::SimulationState;
use crate::simulator::types::*;
use crate::simulator::value::*;
use crate::simulator::util::*;
use crate::simulator::parallel;
use std::collections::{HashMap, HashSet};

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

    pub(crate) fn is_forced(&self, lvalue: &IrLValue) -> bool {
        self.signal_id_from_lvalue(lvalue)
            .map_or(false, |id| self.forced_signals.contains(&id))
    }

}
