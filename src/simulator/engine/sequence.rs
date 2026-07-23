use super::SimulationEngine;
use crate::error::SimError;
use crate::ir::*;
use crate::simulator::types::*;

impl SimulationEngine {
    pub(crate) fn eval_sequence_at_cycle(&mut self, seq: &IrSequence, cycles: u64) -> Result<bool, SimError> {
        match seq {
            IrSequence::Expr(expr) => {
                if cycles == 0 {
                    let val = self.evaluate_expr(expr)?;
                    Ok(val.to_bool() == Some(true))
                } else {
                    Ok(false)
                }
            }
            IrSequence::Delay(n) => Ok(cycles == *n),
            IrSequence::DelayRange(min, max) => Ok(cycles >= *min && cycles <= *max),
            IrSequence::Concat(a, b) => {
                for k in 0..cycles {
                    if self.eval_sequence_at_cycle(a, k)?
                        && self.eval_sequence_at_cycle(b, cycles - k - 1)?
                    {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            IrSequence::Or(a, b) => Ok(self.eval_sequence_at_cycle(a, cycles)?
                || self.eval_sequence_at_cycle(b, cycles)?),
            IrSequence::And(a, b) => Ok(self.eval_sequence_at_cycle(a, cycles)?
                && self.eval_sequence_at_cycle(b, cycles)?),
            IrSequence::Repeat(seq, n) => {
                if *n == 0 {
                    return Ok(true);
                }
                if *n == 1 {
                    return self.eval_sequence_at_cycle(seq, cycles);
                }
                for k in 0..=cycles {
                    if self.eval_sequence_at_cycle(seq, k)? {
                        let remaining = IrSequence::Repeat(Box::new((**seq).clone()), n - 1);
                        if self.eval_sequence_at_cycle(&remaining, cycles - k)? {
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            }
        }
    }

    pub(crate) fn evaluate_sequence_attempts(&mut self) -> Result<(), SimError> {
        let firing_events: Vec<bool> = self
            .sequence_attempts
            .iter()
            .map(|a| self.check_concurrent_clock_event(&a.clock_event))
            .collect();

        let seqs: Vec<(Box<IrSequence>, u64)> = self
            .sequence_attempts
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx < firing_events.len() && firing_events[*idx])
            .map(|(_, a)| (a.sequence.clone(), a.cycles))
            .collect();

        let mut results: Vec<bool> = Vec::new();
        for (seq, cycles) in &seqs {
            results.push(self.eval_sequence_at_cycle(seq, *cycles)?);
        }

        let mut completed = Vec::new();
        let mut result_idx = 0;
        for (idx, attempt) in self.sequence_attempts.iter_mut().enumerate() {
            if idx < firing_events.len() && firing_events[idx] {
                let matched = if result_idx < results.len() {
                    results[result_idx]
                } else {
                    false
                };
                result_idx += 1;
                let max_cycles = attempt.sequence.max_cycles().unwrap_or(u64::MAX);
                if matched {
                    completed.push((idx, true));
                } else if attempt.cycles >= max_cycles {
                    completed.push((idx, false));
                }
                attempt.cycles += 1;
            }
        }

        for (idx, success) in completed.into_iter().rev() {
            if let Some(attempt) = self.sequence_attempts.get(idx) {
                let stmts = if success {
                    attempt.pass_stmt.clone()
                } else {
                    attempt.fail_stmt.clone()
                };
                if !stmts.is_empty() {
                    self.evaluate_block_with_delay_fork(&stmts, None)?;
                }
            }
            self.sequence_attempts.remove(idx);
        }
        Ok(())
    }

    pub(crate) fn check_concurrent_clock_event(&self, ce: &crate::ast::types::ClockEvent) -> bool {
        let sig_name = match ce {
            crate::ast::types::ClockEvent::Posedge(s) => s,
            crate::ast::types::ClockEvent::Negedge(s) => s,
            crate::ast::types::ClockEvent::Edge(s) => s,
        };
        let sig_id = match self.find_signal(sig_name.as_str()) {
            Some(id) => id,
            None => return true,
        };
        let curr = self.state.read_signal(sig_id);
        match ce {
            crate::ast::types::ClockEvent::Posedge(_) => {
                if let Some(ref snap) = self.signal_snapshot {
                    let old = snap
                        .get(sig_id)
                        .cloned()
                        .unwrap_or_else(|| LogicVec::new(1));
                    old.to_bool() != Some(true) && curr.to_bool() == Some(true)
                } else {
                    curr.to_bool() == Some(true)
                }
            }
            crate::ast::types::ClockEvent::Negedge(_) => {
                if let Some(ref snap) = self.signal_snapshot {
                    let old = snap
                        .get(sig_id)
                        .cloned()
                        .unwrap_or_else(|| LogicVec::new(1));
                    old.to_bool() != Some(false) && curr.to_bool() == Some(false)
                } else {
                    curr.to_bool() == Some(false)
                }
            }
            crate::ast::types::ClockEvent::Edge(_) => {
                if let Some(ref snap) = self.signal_snapshot {
                    let old = snap
                        .get(sig_id)
                        .cloned()
                        .unwrap_or_else(|| LogicVec::new(1));
                    old.to_bool() != curr.to_bool()
                } else {
                    true
                }
            }
        }
    }
}
