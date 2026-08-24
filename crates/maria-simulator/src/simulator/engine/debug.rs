use crate::simulator::types::*;
/// Debug and breakpoint functionality for SimulationEngine.
/// Contains signal history tracking, breakpoint checking, and watchpoint logic.
use maria_core::error::SimError;

use super::SimulationEngine;

/// Cap jumlah entry event_log — watchpoint/breakpoint aktif tiap cycle bisa
/// menghasilkan ribuan event; jangan biarkan event_log numpuk tanpa batas.
const MAX_EVENT_LOG: usize = 100_000;

impl SimulationEngine {
    /// Check all breakpoints and watchpoints, update signal history and snapshots.
    /// Called at the end of each simulation cycle when debug mode is enabled.
    pub(crate) fn debug_check(&mut self) -> Result<(), SimError> {
        let time = self.state.time;

        // Save snapshot for reverse debug
        if self.debug_mode == DebugMode::DeepDebug && time.is_multiple_of(self.snapshot_interval) {
            self.snapshots.push(StateSnapshot {
                time,
                signals: self.state.signals.clone(),
                next_signals: self.state.next_signals.clone(),
                changed: self.state.changed.clone(),
            });
            if self.snapshots.len() > 10000 {
                self.snapshots.remove(0);
            }
        }

        // Update signal history via SignalHistoryStore
        for sig in &self.design.top.signals {
            let id = self.find_signal(sig.name.as_str());
            if let Some(id) = id {
                let val = self.state.read_signal(id).clone();
                self.signal_history.record(time, sig.name, val);
            }
        }

        // Check breakpoints
        for bp in &self.breakpoints {
            match bp {
                Breakpoint::Cycle(c) => {
                    if *c == time {
                        self.paused = true;
                        self.event_log.push(DebugEvent {
                            kind: DebugEventKind::BreakpointHit,
                            time,
                            message: format!("breakpoint cycle {} hit", c),
                        });
                    }
                }
                Breakpoint::SignalEq(name, expected) => {
                    let id = self.find_signal(name);
                    if let Some(id) = id {
                        let val = self.state.read_signal(id);
                        if val == expected {
                            self.paused = true;
                            self.event_log.push(DebugEvent {
                                kind: DebugEventKind::BreakpointHit,
                                time,
                                message: format!("breakpoint {} == {} hit", name, expected),
                            });
                        }
                    }
                }
                Breakpoint::SignalNeq(name, expected) => {
                    let id = self.find_signal(name);
                    if let Some(id) = id {
                        let val = self.state.read_signal(id);
                        if val != expected {
                            self.paused = true;
                            self.event_log.push(DebugEvent {
                                kind: DebugEventKind::BreakpointHit,
                                time,
                                message: format!("breakpoint {} != {} hit", name, expected),
                            });
                        }
                    }
                }
                Breakpoint::SignalChange(name) => {
                    let sym = maria_core::Symbol::intern(name);
                    let history = self.signal_history.get_history(&sym);
                    if history.len() >= 2 {
                        let last = &history[history.len() - 1];
                        let prev = &history[history.len() - 2];
                        if last.1 != prev.1 {
                            self.paused = true;
                            self.event_log.push(DebugEvent {
                                kind: DebugEventKind::BreakpointHit,
                                time,
                                message: format!(
                                    "breakpoint change {} hit: {} → {}",
                                    name, prev.1, last.1
                                ),
                            });
                        }
                    }
                }
                Breakpoint::Module(name) => {
                    if self.design.top.name == *name {
                        self.paused = true;
                        self.event_log.push(DebugEvent {
                            kind: DebugEventKind::BreakpointHit,
                            time,
                            message: format!("breakpoint module {} hit", name),
                        });
                    }
                }
            }
        }

        // Check watchpoints
        for wp in &self.watchpoints {
            match wp {
                Watchpoint::Signal(name) => {
                    let sym = maria_core::Symbol::intern(name);
                    let history = self.signal_history.get_history(&sym);
                    if history.len() >= 2 {
                        let last = &history[history.len() - 1];
                        let prev = &history[history.len() - 2];
                        if last.1 != prev.1 {
                            self.event_log.push(DebugEvent {
                                kind: DebugEventKind::WatchpointHit,
                                time,
                                message: format!(
                                    "WATCH: {} changed\n  old = {}\n  new = {}\n  cycle = {}",
                                    name, prev.1, last.1, time
                                ),
                            });
                            self.paused = true;
                        }
                    }
                }
                Watchpoint::MemAddr(addr) => {
                    self.event_log.push(DebugEvent {
                        kind: DebugEventKind::WatchpointHit,
                        time,
                        message: format!("WATCH: mem[0x{:X}] polled at cycle {}", addr, time),
                    });
                }
            }
        }

        // Cap event_log (anti-leak: watchpoint/breakpoint aktif tiap cycle
        // menghasilkan entry baru; pertahankan MAX_EVENT_LOG terbaru saja).
        if self.event_log.len() > MAX_EVENT_LOG {
            let excess = self.event_log.len() - MAX_EVENT_LOG;
            self.event_log.drain(..excess);
        }

        Ok(())
    }
}
