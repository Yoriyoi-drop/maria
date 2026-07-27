use std::collections::HashMap;
use super::Elaborator;
use crate::ast::*;
use super::super::util::const_eval_params;
use crate::diagnostics::diagnostic::DiagCode;
use crate::error::SimError;
use crate::intern::Symbol;
use crate::ir::*;
use super::super::util::{infer_comb_sensitivity, resolve_expr_signal, detect_sync_reset};

impl Elaborator {
    pub(crate) fn elaborate_always(
        &self,
        always: &AlwaysBlock,
        signal_map: &HashMap<Symbol, SignalId>,
        signals: &[SignalInfo],
    ) -> Result<Process, SimError> {
        let name = Symbol::intern(&format!("always_{}", 0));

        match always.kind {
            AlwaysKind::AlwaysComb | AlwaysKind::AlwaysLatch => {
                let body = self.elaborate_stmt_block(&always.stmts, signal_map, &[], signals)?;
                let sensitivity = infer_comb_sensitivity(&body);
                Ok(Process::CombReactive {
                    name,
                    sensitivity,
                    body,
                })
            }
            AlwaysKind::AlwaysFF => {
                let (clock, reset) = self.extract_clock_reset(&always.sensitivity, signal_map)?;
                let body = self.elaborate_stmt_block(&always.stmts, signal_map, &[], signals)?;
                let reset = reset.or_else(|| detect_sync_reset(&body));
                Ok(Process::Sequential {
                    name,
                    clock,
                    reset,
                    body,
                })
            }
            AlwaysKind::Always => {
                // Check if body starts with a delay (always #N pattern)
                if always.sensitivity.is_none() && always.stmts.len() == 1 {
                    if let Stmt::Delay { delay, stmt } = &always.stmts[0] {
                        if let Ok(d) = const_eval_params(delay, &self.param_vals) {
                            let body = self.elaborate_stmt_block(
                                &[stmt.as_ref().clone()],
                                signal_map,
                                &[],
                                signals,
                            )?;
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
                    if sl.events.iter().any(|e| {
                        matches!(
                            e,
                            SensitivityEvent::PosEdge(_) | SensitivityEvent::NegEdge(_)
                        )
                    }) {
                        match self.extract_clock_reset(&always.sensitivity, signal_map) {
                            Ok((clock, reset)) => {
                                let body = self.elaborate_stmt_block(
                                    &always.stmts,
                                    signal_map,
                                    &[],
                                    signals,
                                )?;
                                let reset = reset.or_else(|| detect_sync_reset(&body));
                                return Ok(Process::Sequential {
                                    name,
                                    clock,
                                    reset,
                                    body,
                                });
                            }
                            Err(_) => {} // fall through to combinational
                        }
                    }
                }
                let body = self.elaborate_stmt_block(&always.stmts, signal_map, &[], signals)?;
                let sensitivity = match &always.sensitivity {
                    Some(sl) => {
                        let has_wildcard = sl
                            .events
                            .iter()
                            .any(|e| matches!(e, SensitivityEvent::Wildcard));
                        if has_wildcard {
                            infer_comb_sensitivity(&body)
                        } else {
                            sl.events
                                .iter()
                                .filter_map(|e| match e {
                                    SensitivityEvent::Level(expr) => {
                                        resolve_expr_signal(expr, signal_map)
                                    }
                                    _ => None,
                                })
                                .collect()
                        }
                    }
                    None => Vec::new(),
                };
                Ok(Process::Combinational {
                    name,
                    sensitivity,
                    body,
                })
            }
        }
    }

    fn extract_clock_reset(
        &self,
        sensitivity: &Option<SensitivityList>,
        signal_map: &HashMap<Symbol, SignalId>,
    ) -> Result<(ClockEdge, Option<ResetInfo>), SimError> {
        let events = match sensitivity {
            Some(sl) => &sl.events,
            None => return Err(self.elab_diag(DiagCode::ModuleNotFound, "always_ff requires sensitivity list")),
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

        clock_edge
            .ok_or_else(|| self.elab_diag(DiagCode::ModuleNotFound, "always_ff must have at least one clock edge"))
            .map(|ce| (ce, reset))
    }
}
