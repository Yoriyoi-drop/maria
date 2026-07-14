use crate::ir::*;
use crate::simulator::state::SimulationState;
use crate::simulator::value::*;
use crate::simulator::types::*;
use super::util::*;
use crate::waveform::VcdWriter;
use crate::ast::*;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use rand::Rng;
use std::fmt;

pub struct SimulationEngine {
    pub state: SimulationState,
    pub design: IrDesign,
    pub max_time: u64,
    pub running: bool,
    events: Vec<Vec<RegionEvent>>,
    nba_pending: Vec<(IrLValue, LogicVec)>,
    vcd: Option<VcdWriter>,
    current_this: Option<ObjId>,
    method_locals: Vec<HashMap<String, LogicVec>>,
    current_method: Option<String>,
    rng: rand::rngs::ThreadRng,
    file_handles: HashMap<u32, std::fs::File>,
    file_read_pos: HashMap<u32, u64>,
    next_file_handle: u32,
    monitor_args: Option<Vec<IrExpr>>,
    monitor_last_values: Option<Vec<LogicVec>>,
    disable_pending: Option<String>,
    control_flow: Option<FlowControl>,
    forced_signals: HashSet<SignalId>,
    signal_snapshot: Option<Vec<LogicVec>>,
    pending_waits: Vec<(Vec<SignalId>, Vec<IrStmt>)>,
    pending_wait_orders: Vec<WaitOrderState>,
    loop_continuation: Option<Vec<IrStmt>>,
    current_time: usize,
    fork_groups: Vec<ForkGroup>,
    reactive_events: Vec<EventKind>,
    strobe_events: Vec<Vec<IrExpr>>,
    fstrobe_events: Vec<(u32, Vec<IrExpr>)>,
    fmonitor_map: HashMap<u32, (Vec<IrExpr>, Vec<LogicVec>)>,
    mailbox_queues: HashMap<SignalId, Vec<LogicVec>>,
    semaphore_counts: HashMap<SignalId, u32>,
    assoc_data: HashMap<SignalId, HashMap<LogicVec, LogicVec>>,
    uvm_object_data: HashMap<ObjId, UvmObjectData>,
    uvm_component_data: HashMap<ObjId, UvmComponentData>,
    uvm_sequencer_data: HashMap<ObjId, UvmSequencerData>,
    uvm_driver_data: HashMap<ObjId, UvmDriverData>,
    uvm_analysis_port_data: HashMap<ObjId, UvmAnalysisPortData>,
    uvm_analysis_imp_data: HashMap<ObjId, UvmAnalysisImpData>,
    uvm_config_db_data: HashMap<(String, String), LogicVec>,
    uvm_resource_db_data: HashMap<(String, String), LogicVec>,
    factory_type_overrides: HashMap<String, String>,
    root_test_obj_id: Option<ObjId>,
    process_map: HashMap<ObjId, ProcessInfo>,
    next_process_id: ObjId,
    current_process_id: Option<ObjId>,
    pub(crate) cover_hits: HashMap<String, u64>,
    pub(crate) cover_total: HashMap<String, u64>,
    pub(crate) cover_bins: HashMap<String, HashMap<String, u64>>,
    pub debug_mode: DebugMode,
    pub breakpoints: Vec<Breakpoint>,
    pub watchpoints: Vec<Watchpoint>,
    pub signal_history: HashMap<String, Vec<(u64, LogicVec)>>,
    pub snapshots: Vec<StateSnapshot>,
    pub paused: bool,
    pub step_mode: StepMode,
    pub event_log: Vec<DebugEvent>,
    pub snapshot_interval: u64,
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
            file_read_pos: HashMap::new(),
            next_file_handle: 1,
            monitor_args: None,
            monitor_last_values: None,
            disable_pending: None,
            control_flow: None,
            forced_signals: HashSet::new(),
            signal_snapshot: None,
            pending_waits: Vec::new(),
            pending_wait_orders: Vec::new(),
            loop_continuation: None,
            current_time: 0,
            fork_groups: Vec::new(),
            reactive_events: Vec::new(),
            strobe_events: Vec::new(),
            fstrobe_events: Vec::new(),
            fmonitor_map: HashMap::new(),
            mailbox_queues: HashMap::new(),
            semaphore_counts: HashMap::new(),
            assoc_data: HashMap::new(),
            uvm_object_data: HashMap::new(),
            uvm_component_data: HashMap::new(),
            uvm_sequencer_data: HashMap::new(),
            uvm_driver_data: HashMap::new(),
            uvm_analysis_port_data: HashMap::new(),
            uvm_analysis_imp_data: HashMap::new(),
            uvm_config_db_data: HashMap::new(),
            uvm_resource_db_data: HashMap::new(),
            factory_type_overrides: HashMap::new(),
            root_test_obj_id: None,
            process_map: HashMap::new(),
            next_process_id: 1,
            current_process_id: None,
            cover_hits: HashMap::new(),
            cover_total: HashMap::new(),
            cover_bins: HashMap::new(),
            debug_mode: DebugMode::Normal,
            breakpoints: Vec::new(),
            watchpoints: Vec::new(),
            signal_history: HashMap::new(),
            snapshots: Vec::new(),
            paused: false,
            step_mode: StepMode::Running,
            event_log: Vec::new(),
            snapshot_interval: 1000,
        }
    }

    pub fn set_vcd(&mut self, vcd: VcdWriter) {
        self.vcd = Some(vcd);
    }

    pub fn run(&mut self) -> Result<(), String> {
        self.initialize_time_zero()?;
        self.execute_phases()?;

        while self.running && self.state.time <= self.max_time {
            let t = self.state.time as usize;

            // ── Preponed region: snapshot all signals (once per time slot) ──
            let num_sigs = self.state.signals.len();
            let mut snapshot = Vec::with_capacity(num_sigs);
            for i in 0..num_sigs {
                snapshot.push(self.state.read_signal(i).clone());
            }
            self.signal_snapshot = Some(snapshot);

            self.dump_vcd_time()?;

            // ── IEEE 1800 stratified event loop ──
            let mut delta_count = 0u64;
            loop {
                let mut activity = false;
                let mut deltas: Vec<SignalId> = Vec::new();

                for &region in &IEEE_REGIONS[..] {
                    match region {
                        EventRegion::Preponed => {
                            // Already handled above — skip during re-circulation
                        }
                        EventRegion::PreActive
                        | EventRegion::PreNba
                        | EventRegion::PostNba
                        | EventRegion::PreObserved
                        | EventRegion::PostObserved
                        | EventRegion::PostReactive => {
                            // PLI regions: process any events in this region
                            if t < self.events.len() {
                                let mut matched = true;
                                while matched {
                                    matched = false;
                                    let mut to_process = Vec::new();
                                    self.events[t].retain(|re| {
                                        if re.region == region {
                                            to_process.push(re.event.clone());
                                            false
                                        } else { true }
                                    });
                                    if !to_process.is_empty() {
                                        activity = true;
                                        matched = true;
                                        for event in to_process {
                                            self.process_event(event, t)?;
                                        }
                                    }
                                }
                            }
                        }
                        EventRegion::Observed => {
                            // Observed region: evaluate concurrent assertions (SVA).
                            // Process any assertion-evaluation events scheduled here.
                            if t < self.events.len() {
                                let mut matched = true;
                                while matched {
                                    matched = false;
                                    let mut to_process = Vec::new();
                                    self.events[t].retain(|re| {
                                        if re.region == EventRegion::Observed {
                                            to_process.push(re.event.clone());
                                            false
                                        } else { true }
                                    });
                                    if !to_process.is_empty() {
                                        activity = true;
                                        matched = true;
                                        for event in to_process {
                                            self.process_event(event, t)?;
                                        }
                                    }
                                }
                            }
                        }
                        EventRegion::Active | EventRegion::Inactive => {
                            if t < self.events.len() {
                                loop {
                                    let events: Vec<RegionEvent> = self.events[t].drain(..)
                                        .filter(|re| re.region == region)
                                        .collect();
                                    if events.is_empty() { break; }
                                    activity = true;
                                    for re in events {
                                        self.process_event(re.event, t)?;
                                    }
                                    // Inactive re-drains; Active drains once (outer loop
                                    // re-circulates if new events appear later)
                                    if region == EventRegion::Active { break; }
                                }
                            }
                        }
                        EventRegion::Nba => {
                            // NBA region: commit pending non-blocking assignments
                            self.commit_nba();
                            if t < self.events.len() {
                                let events: Vec<RegionEvent> = self.events[t].drain(..)
                                    .filter(|re| re.region == EventRegion::Nba)
                                    .collect();
                                if !events.is_empty() {
                                    activity = true;
                                    for re in events {
                                        self.process_event(re.event, t)?;
                                    }
                                }
                            }
                        }
                        EventRegion::Reactive => {
                            // Commit changes and trigger sensitive processes
                            let changed = self.state.commit_changes();
                            if !changed.is_empty() {
                                activity = true;
                                for (id, _, _) in &changed {
                                    if !deltas.contains(id) {
                                        deltas.push(*id);
                                    }
                                }
                                self.trigger_sensitive_processes(&changed, t)?;
                            }
                            // Process Reactive events (from events[t] and reactive_events buffer)
                            if t < self.events.len() {
                                let events: Vec<RegionEvent> = self.events[t].drain(..)
                                    .filter(|re| re.region == EventRegion::Reactive)
                                    .collect();
                                if !events.is_empty() {
                                    activity = true;
                                    for re in events {
                                        self.process_event(re.event, t)?;
                                    }
                                }
                            }
                            let buffered: Vec<EventKind> = self.reactive_events.drain(..).collect();
                            if !buffered.is_empty() {
                                activity = true;
                                for event in buffered {
                                    self.process_event(event, t)?;
                                }
                            }
                        }
                    }
                }

                if delta_count > 10_000_000 {
                    return Err("simulation exceeded max delta cycles per time step (10M)".to_string());
                }
                delta_count += 1;

                // Check pending $wait conditions
                if !self.pending_waits.is_empty() && !deltas.is_empty() {
                    if self.process_pending_waits(&deltas)? {
                        activity = true;
                    }
                }

                // Check pending wait_order conditions
                if !self.pending_wait_orders.is_empty() && !deltas.is_empty() {
                    if self.process_pending_wait_orders(&deltas)? {
                        activity = true;
                    }
                }

                // Re-circulate if any events remain or NBA is pending
                let has_remaining = t < self.events.len()
                    && self.events[t].iter().any(|re| {
                        matches!(re.region, EventRegion::PreActive | EventRegion::Active
                            | EventRegion::Inactive | EventRegion::PreNba | EventRegion::Nba
                            | EventRegion::PostNba | EventRegion::PreObserved
                            | EventRegion::Observed | EventRegion::PostObserved
                            | EventRegion::Reactive | EventRegion::PostReactive)
                    })
                    || !self.nba_pending.is_empty();

                if has_remaining {
                    activity = true;
                }

                if !activity {
                    break;
                }
            }

            // ── Postponed region: $strobe, $monitor, VCD ──
            self.process_strobe()?;
            self.dump_vcd_state()?;
            self.check_monitor()?;

            // ── Debug check at start of cycle ──
            if self.debug_mode != DebugMode::Normal {
                self.debug_check()?;
                if self.paused { break; }
                if self.step_mode == StepMode::StepCycle {
                    self.paused = true;
                    break;
                }
            }

            self.state.time += 1;
            if self.state.time > self.max_time {
                break;
            }

            if self.state.time as usize >= self.events.len() {
                self.events.push(Vec::new());
            }
        }

        if !self.paused {
            self.execute_final_blocks()?;
            self.report_coverage();
        }

        Ok(())
    }

    fn debug_check(&mut self) -> Result<(), String> {
        let time = self.state.time;

        // Save snapshot for reverse debug
        if self.debug_mode == DebugMode::DeepDebug && time % self.snapshot_interval == 0 {
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

        // Update signal history
        for sig in &self.design.top.signals {
            let id = self.find_signal(&sig.name);
            if let Some(id) = id {
                let val = self.state.read_signal(id).clone();
                self.signal_history.entry(sig.name.clone())
                    .or_insert_with(Vec::new)
                    .push((time, val));
                if let Some(hist) = self.signal_history.get(&sig.name) {
                    if hist.len() > 100000 {
                        self.signal_history.get_mut(&sig.name).unwrap().remove(0);
                    }
                }
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
                    if let Some(history) = self.signal_history.get(name) {
                        if history.len() >= 2 {
                            let last = &history[history.len() - 1];
                            let prev = &history[history.len() - 2];
                            if last.1 != prev.1 {
                                self.paused = true;
                                self.event_log.push(DebugEvent {
                                    kind: DebugEventKind::BreakpointHit,
                                    time,
                                    message: format!("breakpoint change {} hit: {} → {}", name, prev.1, last.1),
                                });
                            }
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
                    if let Some(history) = self.signal_history.get(name) {
                        if history.len() >= 2 {
                            let last = &history[history.len() - 1];
                            let prev = &history[history.len() - 2];
                            if last.1 != prev.1 {
                                self.event_log.push(DebugEvent {
                                    kind: DebugEventKind::WatchpointHit,
                                    time,
                                    message: format!("WATCH: {} changed\n  old = {}\n  new = {}\n  cycle = {}", name, prev.1, last.1, time),
                                });
                                self.paused = true;
                            }
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
                let msg = format_display(&self.state, &self.design.top.signals, &self.design.hier_signal_map, &self.assoc_data, args);
                print!("{}", msg);
                self.monitor_last_values = Some(new_vals);
            }
        }
        let fmonitor: Vec<(u32, Vec<IrExpr>, Vec<LogicVec>)> = self.fmonitor_map.iter()
            .map(|(h, (args, last))| (*h, args.clone(), last.clone()))
            .collect();
        for (handle, args, last) in fmonitor {
            let new_vals: Vec<LogicVec> = args.iter()
                .map(|a| self.evaluate_expr(a).unwrap_or(LogicVec::from_u64(0, 32)))
                .collect();
            if new_vals != last {
                if let Some(f) = self.file_handles.get_mut(&handle) {
                    let msg = format_display(&self.state, &self.design.top.signals, &self.design.hier_signal_map, &self.assoc_data, &args);
                    let _ = write!(f, "{}", msg);
                }
                self.fmonitor_map.insert(handle, (args, new_vals));
            }
        }
        Ok(())
    }

    fn process_strobe(&mut self) -> Result<(), String> {
        let events = std::mem::take(&mut self.strobe_events);
        for args in &events {
            let msg = format_display(&self.state, &self.design.top.signals, &self.design.hier_signal_map, &self.assoc_data, args);
            print!("{}", msg);
        }
        let fstrobe = std::mem::take(&mut self.fstrobe_events);
        for (handle, args) in &fstrobe {
            if let Some(f) = self.file_handles.get_mut(handle) {
                let msg = format_display(&self.state, &self.design.top.signals, &self.design.hier_signal_map, &self.assoc_data, args);
                let _ = write!(f, "{}", msg);
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
                    self.events[t].push(RegionEvent { region: EventRegion::Active, event: EventKind::EvalProcess(pid) });
                }
                Process::Final { .. } => {
                    // Final blocks execute only at $finish, not at time zero
                }
                Process::AlwaysWithDelay { .. } => {
                    self.events[t].push(RegionEvent { region: EventRegion::Active, event: EventKind::EvalProcess(pid) });
                }
                Process::Combinational { .. } | Process::CombReactive { .. } => {
                    // Evaluate at time zero via event, not inline, so initial/always
                    // blocks run first and signals have settled
                    self.events[t].push(RegionEvent { region: EventRegion::Active, event: EventKind::EvalProcess(pid) });
                }
                Process::Sequential { .. } => {}
            }
        }
        // Initialize coverage tracking
        for cg in &self.design.covergroups {
            for cp in &cg.coverpoints {
                let key = format!("{}.{}", cg.name, cp.name);
                self.cover_total.insert(key.clone(), 0);
                self.cover_hits.insert(key.clone(), 0);
                self.cover_bins.insert(key, HashMap::new());
            }
            for cross in &cg.crosses {
                let key = format!("{}.{}", cg.name, cross.name);
                self.cover_total.insert(key.clone(), 0);
                self.cover_hits.insert(key.clone(), 0);
                self.cover_bins.insert(key, HashMap::new());
            }
        }
        Ok(())
    }

    fn sample_covergroup(&mut self, cg_name: &str) -> Result<(), String> {
        // Clone covergroup data to avoid borrow conflict with evaluate_expr
        let cg = self.design.covergroups.iter()
            .find(|c| c.name == cg_name)
            .cloned();
        if let Some(cg) = cg {
            let mut cp_values: HashMap<String, u64> = HashMap::new();
            for cp in &cg.coverpoints {
                let key = format!("{}.{}", cg.name, cp.name);
                let total = self.cover_total.entry(key.clone()).or_insert(0);
                *total += 1;
                let val = self.evaluate_expr(&cp.expr).unwrap_or(LogicVec::from_u64(0, 32));
                cp_values.insert(cp.name.clone(), val.to_u64());
                let bin_key = format!("{}={}", cp.name, val.to_u64());
                let bins = self.cover_bins.entry(key.clone()).or_insert_with(HashMap::new);
                let entry = bins.entry(bin_key).or_insert(0);
                *entry += 1;
                let hits = self.cover_hits.entry(key).or_insert(0);
                *hits += 1;
            }
            // Cross coverage
            for cross in &cg.crosses {
                let key = format!("{}.{}", cg.name, cross.name);
                let total = self.cover_total.entry(key.clone()).or_insert(0);
                *total += 1;
                let mut parts: Vec<String> = Vec::new();
                for cp_name in &cross.coverpoints {
                    let val = cp_values.get(cp_name).copied().unwrap_or(0);
                    parts.push(format!("{}={}", cp_name, val));
                }
                let bin_key = parts.join(" x ");
                let bins = self.cover_bins.entry(key.clone()).or_insert_with(HashMap::new);
                let entry = bins.entry(bin_key).or_insert(0);
                *entry += 1;
                let hits = self.cover_hits.entry(key).or_insert(0);
                *hits += 1;
            }
        }
        Ok(())
    }

    fn report_coverage(&self) {
        if self.design.covergroups.is_empty() {
            return;
        }
        eprintln!("\n=== Coverage Report ===");
        for cg in &self.design.covergroups {
            eprintln!("Covergroup: {}", cg.name);
            for cp in &cg.coverpoints {
                let key = format!("{}.{}", cg.name, cp.name);
                let total = self.cover_total.get(&key).copied().unwrap_or(0);
                let hits = self.cover_hits.get(&key).copied().unwrap_or(0);
                let bins = self.cover_bins.get(&key);
                let pct = if total > 0 { (hits as f64 / total as f64) * 100.0 } else { 0.0 };
                eprintln!("  {}: {} hits / {} samples ({:.1}%)", cp.name, hits, total, pct);
                if let Some(bins) = bins {
                    for (bin_key, count) in bins.iter() {
                        eprintln!("    - {}: {} hits", bin_key, count);
                    }
                }
            }
            for cross in &cg.crosses {
                let key = format!("{}.{}", cg.name, cross.name);
                let total = self.cover_total.get(&key).copied().unwrap_or(0);
                let hits = self.cover_hits.get(&key).copied().unwrap_or(0);
                let bins = self.cover_bins.get(&key);
                let pct = if total > 0 { (hits as f64 / total as f64) * 100.0 } else { 0.0 };
                eprintln!("  {} (cross): {} hits / {} samples ({:.1}%)", cross.name, hits, total, pct);
                if let Some(bins) = bins {
                    for (bin_key, count) in bins.iter() {
                        eprintln!("    - {}: {} hits", bin_key, count);
                    }
                }
            }
        }
    }

    fn execute_final_blocks(&mut self) -> Result<(), String> {
        let bodies: Vec<Vec<IrStmt>> = self.design.top.processes.iter()
            .filter_map(|p| {
                if let Process::Final { body, .. } = p {
                    Some(body.clone())
                } else {
                    None
                }
            })
            .collect();
        for body in &bodies {
            self.evaluate_stmt_block(body)?;
        }
        Ok(())
    }

    fn process_event(&mut self, event: EventKind, t: usize) -> Result<(), String> {
        self.current_time = t;
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
                                self.events[next_t].push(RegionEvent { region: EventRegion::Active, event: EventKind::EvalProcess(pid) });
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
                    let all_consumed = self.evaluate_block_with_delay_fork(&cont.stmts_to_exec, cont.fork_id)?;
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
            EventKind::NbaCommit => {
                self.commit_nba();
            }
        }
        Ok(())
    }

    fn evaluate_block_with_delay(
        &mut self, stmts: &[IrStmt]
    ) -> Result<bool, String> {
        self.evaluate_block_with_delay_fork(stmts, None)
    }

    fn evaluate_block_with_delay_fork(
        &mut self, stmts: &[IrStmt], fork_id: Option<usize>
    ) -> Result<bool, String> {
        for (i, stmt) in stmts.iter().enumerate() {
            if self.disable_pending.is_some() { return Ok(true); }
            if self.control_flow.is_some() { return Ok(true); }
            match stmt {
                IrStmt::Block { stmts: inner } => {
                    self.evaluate_block_with_delay_fork(inner, fork_id)?;
                }
                IrStmt::NamedBlock { name, stmts: inner } => {
                    if self.disable_pending.as_deref() == Some(name) {
                        self.disable_pending = None;
                        return Ok(true);
                    }
                    let old = self.disable_pending.take();
                    self.evaluate_block_with_delay_fork(inner, fork_id)?;
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
                        self.evaluate_block_with_delay_fork(then_stmts, fork_id)?;
                    } else if !else_stmts.is_empty() {
                        self.evaluate_block_with_delay_fork(else_stmts, fork_id)?;
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
                                self.evaluate_block_with_delay_fork(&case_item.body, fork_id)?;
                                if self.disable_pending.is_some() { return Ok(true); }
                                item_matched = true;
                                matched = true;
                                break;
                            }
                        }
                        if item_matched { break; }
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
                            self.events[delay_t].push(RegionEvent {
                                region,
                                event: EventKind::ContinueBlock(Continuation {
                                    stmts_to_exec: later,
                                    stmts_remaining: vec![],
                                    fork_id,
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
                                let old_val = snap.get(*sig_id).cloned().unwrap_or_else(|| LogicVec::new(1));
                                old_val.to_bool() != Some(true) && sig_val.to_bool() == Some(true)
                            } else {
                                sig_val.to_bool() == Some(true)
                            }
                        }
                        Some(ClockEdge::NegEdge(_)) => {
                            if let Some(ref snap) = self.signal_snapshot {
                                let old_val = snap.get(*sig_id).cloned().unwrap_or_else(|| LogicVec::new(1));
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
                    self.disable_pending = Some(name.clone());
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
                IrStmt::WaitOrder { events, failure_stmts } => {
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
                    let total: u64 = items.iter().map(|(w_expr, _)| {
                        self.evaluate_expr(w_expr).unwrap_or(LogicVec::from_u64(1, 32)).to_u64()
                    }).sum();
                    if total > 0 {
                        let r = self.rng.gen::<u64>() % total;
                        let mut cumulative = 0u64;
                        for (w_expr, body) in items {
                            let weight = self.evaluate_expr(w_expr).unwrap_or(LogicVec::from_u64(1, 32)).to_u64();
                            cumulative += weight;
                            if r < cumulative {
                                let completed = self.evaluate_block_with_delay_fork(body, fork_id)?;
                                if !completed { return Ok(false); }
                                break;
                            }
                        }
                    }
                }
                IrStmt::SysCall { name, args: ir_args } => {
                    if name == "display" || name == "write" {
                        let msg = format_display(&self.state, &self.design.top.signals, &self.design.hier_signal_map, &self.assoc_data, ir_args);
                        print!("{}", msg);
                    } else if name == "strobe" {
                        self.strobe_events.push(ir_args.clone());
                    } else if name == "fstrobe" {
                        let handle = ir_args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            self.fstrobe_events.push((h, ir_args[1..].to_vec()));
                        }
                    } else if name == "fmonitor" {
                        let handle = ir_args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            let vals: Vec<LogicVec> = ir_args[1..].iter()
                                .map(|a| self.evaluate_expr(a).unwrap_or(LogicVec::from_u64(0, 32)))
                                .collect();
                            self.fmonitor_map.insert(h, (ir_args[1..].to_vec(), vals));
                        }
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
                        let val: i32 = self.rng.gen();
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
                    } else if name == "urandom_range" {
                        let args_eval: Vec<LogicVec> = ir_args.iter()
                            .map(|a| self.evaluate_expr(a).unwrap_or(LogicVec::from_u64(0, 32)))
                            .collect();
                        let maxval = args_eval.first().map(|v| v.to_u64()).unwrap_or(0);
                        let minval = args_eval.get(1).map(|v| v.to_u64()).unwrap_or(0);
                        let val = if maxval <= minval {
                            minval
                        } else {
                            let range = maxval - minval + 1;
                            if range <= 1 { minval }
                            else { minval + (self.rng.gen::<u64>() % range) }
                        };
                        let sig_id = ir_args.first().and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
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
                        if let Some(limit) = ir_args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64())) {
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
                        let fname = ir_args.first().and_then(|a| if let IrExpr::String(s) = a { Some(s.clone()) } else { None });
                        if let Some(fname) = fname {
                            let mode = ir_args.get(1).and_then(|a| if let IrExpr::String(s) = a { Some(s.as_str()) } else { None });
                            let open_result = match mode {
                                Some("r") | Some("rb") => std::fs::File::open(&fname),
                                _ => std::fs::OpenOptions::new()
                                    .read(true).write(true).create(true).truncate(true)
                                    .open(&fname),
                            };
                            match open_result {
                                Ok(f) => {
                                    let handle = self.next_file_handle;
                                    self.next_file_handle += 1;
                                    self.file_handles.insert(handle, f);
                                    self.file_read_pos.insert(handle, 0);
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
                        let handle = ir_args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let msg = format_display(&self.state, &self.design.top.signals, &self.design.hier_signal_map, &self.assoc_data, &ir_args[1..]);
                                let _ = write!(f, "{}", msg);
                            }
                        }
                    } else if name == "fwrite" {
                        let handle = ir_args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let msg = format_display(&self.state, &self.design.top.signals, &self.design.hier_signal_map, &self.assoc_data, &ir_args[1..]);
                                let _ = write!(f, "{}", msg);
                            }
                        }
                    } else if name == "fscanf" {
                        let handle = ir_args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                use std::io::{Seek, Read};
                                let read_pos = self.file_read_pos.entry(h).or_insert(0);
                                f.seek(std::io::SeekFrom::Start(*read_pos)).ok();
                                let mut content = String::new();
                                let _bytes_read = f.read_to_string(&mut content).unwrap_or(0);
                                *read_pos = f.stream_position().unwrap_or(0);
                                let fmt = ir_args.get(1).and_then(|a| if let IrExpr::String(s) = a { Some(s.clone()) } else { None });
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
                                                        if let Ok(val) = if spec == 'h' { i64::from_str_radix(tok, 16) } else if spec == 'b' { i64::from_str_radix(tok, 2) } else { tok.parse::<i64>() } {
                                                            let out_idx = 2 + ai;
                                                            if let Some(arg) = ir_args.get(out_idx) {
                                                                if let IrExpr::Signal(sid, _) = arg {
                                                                    self.state.write_signal(*sid, LogicVec::from_u64(val as u64, 32));
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
                        let target = ir_args.first().and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                        let src = ir_args.get(1);
                        let data = if let Some(IrExpr::String(fname)) = src {
                            std::fs::read(fname).ok()
                        } else if let Some(arg) = src {
                            let handle = self.evaluate_expr(arg).ok().map(|v| v.to_u64() as u32).unwrap_or(0);
                            if handle > 0 {
                                use std::io::Read;
                                self.file_handles.get_mut(&handle).and_then(|f| {
                                    let mut buf = Vec::new();
                                    f.read_to_end(&mut buf).ok().map(|_| buf)
                                })
                            } else { None }
                        } else { None };
                        if let (Some(sid), Some(bytes)) = (target, data) {
                            let mut bits = Vec::with_capacity(bytes.len() * 8);
                            for byte in bytes {
                                for i in 0..8 {
                                    bits.push(if (byte >> i) & 1 == 1 { LogicVal::One } else { LogicVal::Zero });
                                }
                            }
                            self.state.write_signal(sid, LogicVec { width: bits.len(), bits });
                        }
                    } else if name == "fclose" {
                        let handle = ir_args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            self.file_handles.remove(&h);
                        }
                    } else if name == "__dpi_stmt" {
                        if let Some(arg) = ir_args.first() {
                            self.evaluate_expr(arg)?;
                        }
                    } else {
                        eprintln!("warning: unknown system call '{}' ignored", name);
                    }
                }
                IrStmt::SysFinish => {
                    self.running = false;
                    return Ok(true);
                }
                IrStmt::Null => {}
                IrStmt::Assert { cond, pass_stmt, fail_stmt } => {
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
                IrStmt::Assume { cond, pass_stmt, fail_stmt } => {
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
                IrStmt::Cover { cond, pass_stmt } => {
                    let ok = self.evaluate_expr(cond)?.to_bool().unwrap_or(false);
                    if ok {
                        eprintln!("cover point hit");
                        if !pass_stmt.is_empty() {
                            self.evaluate_block_with_delay_fork(pass_stmt, fork_id)?;
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
                IrStmt::LoopFor { init, cond, step, body } => {
                    if let Some(init_stmt) = init {
                        self.evaluate_block_with_delay_fork(&[*init_stmt.clone()], fork_id)?;
                    }
                    loop {
                        if self.disable_pending.is_some() { break; }
                        if self.control_flow.is_some() { self.control_flow = None; break; }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) { break; }
                        self.evaluate_block_with_delay_fork(body, fork_id)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) {
                            if let Some(step_stmt) = step {
                                self.evaluate_block_with_delay_fork(&[*step_stmt.clone()], fork_id)?;
                            }
                            continue;
                        }
                        if cf == Some(FlowControl::Break) { break; }
                        if self.disable_pending.is_some() { break; }
                        if let Some(step_stmt) = step {
                            self.evaluate_block_with_delay_fork(&[*step_stmt.clone()], fork_id)?;
                        }
                    }
                }
                IrStmt::LoopWhile { cond, body } => {
                    loop {
                        if self.disable_pending.is_some() { break; }
                        if self.control_flow.is_some() { self.control_flow = None; break; }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) { break; }
                        let old_loop_cont = self.loop_continuation.take();
                        self.loop_continuation = Some(vec![IrStmt::LoopWhile { cond: cond.clone(), body: body.clone() }]);
                        let completed = self.evaluate_block_with_delay_fork(body, fork_id)?;
                        self.loop_continuation = old_loop_cont;
                        if !completed { return Ok(false); }
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) { continue; }
                        if cf == Some(FlowControl::Break) { break; }
                    }
                }
                IrStmt::LoopDoWhile { cond, body } => {
                    loop {
                        if self.disable_pending.is_some() { break; }
                        if self.control_flow.is_some() { self.control_flow = None; break; }
                        let old_loop_cont = self.loop_continuation.take();
                        self.loop_continuation = Some(vec![IrStmt::LoopDoWhile { cond: cond.clone(), body: body.clone() }]);
                        let completed = self.evaluate_block_with_delay_fork(body, fork_id)?;
                        self.loop_continuation = old_loop_cont;
                        if !completed { return Ok(false); }
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) { continue; }
                        if cf == Some(FlowControl::Break) { break; }
                        let cond_val = self.evaluate_expr(cond)?;
                        if !cond_val.to_bool().unwrap_or(false) { break; }
                    }
                }
                IrStmt::Repeat { count, body } => {
                    let count_val = self.evaluate_expr(count)?;
                    let n = count_val.to_u64() as usize;
                    for _ in 0..n {
                        if self.disable_pending.is_some() { break; }
                        if self.control_flow.is_some() { self.control_flow = None; break; }
                        self.evaluate_block_with_delay_fork(body, fork_id)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) { continue; }
                        if cf == Some(FlowControl::Break) { break; }
                    }
                }
                IrStmt::Foreach { array_var, index_var, body } => {
                    let lv = self.evaluate_expr(array_var)?;
                    let sig_info = if let IrExpr::Signal(id, _) = array_var {
                        self.design.top.signals.get(*id)
                    } else { None };
                    let elem_width = sig_info.map(|s| s.elem_width).unwrap_or(1);
                    let count = if elem_width > 0 { lv.width / elem_width } else { 0 };
                    for i in 0..count {
                        if self.disable_pending.is_some() { break; }
                        if self.control_flow.is_some() { self.control_flow = None; break; }
                        let idx_val = LogicVec::from_u64(i as u64, 32);
                        let mut scope = HashMap::new();
                        scope.insert(index_var.clone(), idx_val);
                        let depth = self.method_locals.len();
                        self.method_locals.push(scope);
                        self.evaluate_block_with_delay_fork(body, fork_id)?;
                        self.method_locals.truncate(depth);
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) { continue; }
                        if cf == Some(FlowControl::Break) { break; }
                    }
                }
                IrStmt::MethodCallStmt { obj, method, args, with_clause } => {
                    if let IrExpr::Signal(id, _) = obj {
                        let sig_info = self.design.top.signals.get(*id).cloned();
                        if let Some(ref sig) = sig_info {
                            if sig.is_dynamic || sig.is_queue || sig.is_associative {
                                let _ = self.evaluate_array_method(*id, sig, method, args, with_clause.as_deref())?;
                                continue;
                            }
                            // Auto-create object for class/covergroup variables
                            if let Some(ref cn) = sig.class_name {
                                let is_cg = self.design.covergroups.iter().any(|c| c.name == *cn);
                                if is_cg || self.design.classes.contains_key(cn) {
                                    let obj_val = self.state.read_signal(*id);
                                    let obj_id = obj_val.to_u64() as ObjId;
                                    if obj_id == 0 && self.state.objects.len() > 0 && self.state.objects[0].class_name.is_empty() {
                                        let class_for_obj = if is_cg {
                                            format!("__covergroup_{}", cn)
                                        } else {
                                            cn.clone()
                                        };
                                        let new_id = self.state.alloc_object(&class_for_obj);
                                        self.state.write_signal(*id, LogicVec::from_u64(new_id as u64, 64));
                                        let arg_vals: Vec<LogicVec> = args.iter()
                                            .map(|a| self.evaluate_expr(a))
                                            .collect::<Result<_, _>>()?;
                                        self.execute_method(new_id, method, &arg_vals)?;
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    let obj_val = self.evaluate_expr(obj)?;
                    let obj_id = obj_val.to_u64() as ObjId;
                    let arg_vals: Vec<LogicVec> = args.iter()
                        .map(|a| self.evaluate_expr(a))
                        .collect::<Result<_, _>>()?;
                    self.execute_method(obj_id, method, &arg_vals)?;
                }
                IrStmt::RandCase { items } => {
                    let total: u64 = items.iter().map(|(w_expr, _)| {
                        self.evaluate_expr(w_expr).unwrap_or(LogicVec::from_u64(1, 32)).to_u64()
                    }).sum();
                    if total > 0 {
                        let r = self.rng.gen::<u64>() % total;
                        let mut cumulative = 0u64;
                        for (w_expr, body) in items {
                            let weight = self.evaluate_expr(w_expr).unwrap_or(LogicVec::from_u64(1, 32)).to_u64();
                            cumulative += weight;
                            if r < cumulative {
                                self.evaluate_stmt_block(body)?;
                                break;
                            }
                        }
                    }
                }
                IrStmt::Fork { processes, join_type } => {
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
                                    let all_consumed = self.evaluate_block_with_delay_fork(&p, Some(fid))?;
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
                                    let all_consumed = self.evaluate_block_with_delay_fork(&p, Some(fid))?;
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
        }
        Ok(true)
    }

    fn evaluate_stmt_block(&mut self, stmts: &[IrStmt]) -> Result<(), String> {
        for (i, stmt) in stmts.iter().enumerate() {
            if self.disable_pending.is_some() { return Ok(()); }
            if self.control_flow.is_some() { return Ok(()); }
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
                        let msg = format_display(&self.state, &self.design.top.signals, &self.design.hier_signal_map, &self.assoc_data, ir_args);
                        print!("{}", msg);
                    } else if name == "strobe" {
                        self.strobe_events.push(ir_args.clone());
                    } else if name == "fstrobe" {
                        let handle = ir_args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            self.fstrobe_events.push((h, ir_args[1..].to_vec()));
                        }
                    } else if name == "fmonitor" {
                        let handle = ir_args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            let vals: Vec<LogicVec> = ir_args[1..].iter()
                                .map(|a| self.evaluate_expr(a).unwrap_or(LogicVec::from_u64(0, 32)))
                                .collect();
                            self.fmonitor_map.insert(h, (ir_args[1..].to_vec(), vals));
                        }
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
                    } else if name == "urandom_range" {
                        let args_eval: Vec<LogicVec> = ir_args.iter()
                            .map(|a| self.evaluate_expr(a).unwrap_or(LogicVec::from_u64(0, 32)))
                            .collect();
                        let maxval = args_eval.first().map(|v| v.to_u64()).unwrap_or(0);
                        let minval = args_eval.get(1).map(|v| v.to_u64()).unwrap_or(0);
                        let val = if maxval <= minval {
                            minval
                        } else {
                            let range = maxval - minval + 1;
                            if range <= 1 { minval }
                            else { minval + (self.rng.gen::<u64>() % range) }
                        };
                        let sig_id = ir_args.first().and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                        if let Some(sid) = sig_id {
                            self.state.write_signal(sid, LogicVec::from_u64(val, 32));
                        }
                    } else if name == "random" {
                        let val: i32 = self.rng.gen();
                        let sig_id = ir_args.first().and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                        if let Some(sid) = sig_id {
                            self.state.write_signal(sid, LogicVec::from_u64(val as u64, 32));
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
                        if let Some(limit) = ir_args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64())) {
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
                        let fname = ir_args.first().and_then(|a| if let IrExpr::String(s) = a { Some(s.clone()) } else { None });
                        if let Some(fname) = fname {
                            let mode = ir_args.get(1).and_then(|a| if let IrExpr::String(s) = a { Some(s.as_str()) } else { None });
                            let open_result = match mode {
                                Some("r") | Some("rb") => std::fs::File::open(&fname),
                                _ => std::fs::OpenOptions::new()
                                    .read(true).write(true).create(true).truncate(true)
                                    .open(&fname),
                            };
                            match open_result {
                                Ok(f) => {
                                    let handle = self.next_file_handle;
                                    self.next_file_handle += 1;
                                    self.file_handles.insert(handle, f);
                                    self.file_read_pos.insert(handle, 0);
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
                        let handle = ir_args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let msg = format_display(&self.state, &self.design.top.signals, &self.design.hier_signal_map, &self.assoc_data, &ir_args[1..]);
                                let _ = write!(f, "{}", msg);
                            }
                        }
                    } else if name == "fwrite" {
                        let handle = ir_args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let msg = format_display(&self.state, &self.design.top.signals, &self.design.hier_signal_map, &self.assoc_data, &ir_args[1..]);
                                let _ = write!(f, "{}", msg);
                            }
                        }
                    } else if name == "fscanf" {
                        let handle = ir_args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                use std::io::{Seek, Read};
                                let read_pos = self.file_read_pos.entry(h).or_insert(0);
                                f.seek(std::io::SeekFrom::Start(*read_pos)).ok();
                                let mut content = String::new();
                                let _bytes_read = f.read_to_string(&mut content).unwrap_or(0);
                                *read_pos = f.stream_position().unwrap_or(0);
                                let fmt = ir_args.get(1).and_then(|a| if let IrExpr::String(s) = a { Some(s.clone()) } else { None });
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
                                                        if let Ok(val) = if spec == 'h' { i64::from_str_radix(tok, 16) } else if spec == 'b' { i64::from_str_radix(tok, 2) } else { tok.parse::<i64>() } {
                                                            let out_idx = 2 + ai;
                                                            if let Some(arg) = ir_args.get(out_idx) {
                                                                if let IrExpr::Signal(sid, _) = arg {
                                                                    self.state.write_signal(*sid, LogicVec::from_u64(val as u64, 32));
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
                        let target = ir_args.first().and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                        let src = ir_args.get(1);
                        let data = if let Some(IrExpr::String(fname)) = src {
                            std::fs::read(fname).ok()
                        } else if let Some(arg) = src {
                            let handle = self.evaluate_expr(arg).ok().map(|v| v.to_u64() as u32).unwrap_or(0);
                            if handle > 0 {
                                use std::io::Read;
                                self.file_handles.get_mut(&handle).and_then(|f| {
                                    let mut buf = Vec::new();
                                    f.read_to_end(&mut buf).ok().map(|_| buf)
                                })
                            } else { None }
                        } else { None };
                        if let (Some(sid), Some(bytes)) = (target, data) {
                            let mut bits = Vec::with_capacity(bytes.len() * 8);
                            for byte in bytes {
                                for i in 0..8 {
                                    bits.push(if (byte >> i) & 1 == 1 { LogicVal::One } else { LogicVal::Zero });
                                }
                            }
                            self.state.write_signal(sid, LogicVec { width: bits.len(), bits });
                        }
                    } else if name == "fclose" {
                        let handle = ir_args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            self.file_handles.remove(&h);
                        }
                    } else if name == "__dpi_stmt" {
                        if let Some(arg) = ir_args.first() {
                            self.evaluate_expr(arg)?;
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
                IrStmt::Assert { cond, pass_stmt, fail_stmt } => {
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
                IrStmt::Assume { cond, pass_stmt, fail_stmt } => {
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
                IrStmt::Cover { cond, pass_stmt } => {
                    let ok = self.evaluate_expr(cond)?.to_bool().unwrap_or(false);
                    if ok {
                        eprintln!("cover point hit");
                        if !pass_stmt.is_empty() {
                            self.evaluate_stmt_block(pass_stmt)?;
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
                IrStmt::Repeat { count, body } => {
                    let count_val = self.evaluate_expr(count)?;
                    let n = count_val.to_u64() as usize;
                    for _ in 0..n {
                        if self.disable_pending.is_some() { break; }
                        if self.control_flow.is_some() { self.control_flow = None; break; }
                        self.evaluate_stmt_block(body)?;
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) { continue; }
                        if cf == Some(FlowControl::Break) { break; }
                    }
                }
                IrStmt::RandCase { items } => {
                    let total: u64 = items.iter().map(|(w_expr, _)| {
                        self.evaluate_expr(w_expr).unwrap_or(LogicVec::from_u64(1, 32)).to_u64()
                    }).sum();
                    if total > 0 {
                        let r = self.rng.gen::<u64>() % total;
                        let mut cumulative = 0u64;
                        for (w_expr, body) in items {
                            let weight = self.evaluate_expr(w_expr).unwrap_or(LogicVec::from_u64(1, 32)).to_u64();
                            cumulative += weight;
                            if r < cumulative {
                                self.evaluate_stmt_block(body)?;
                                break;
                            }
                        }
                    }
                }
                IrStmt::Foreach { array_var, index_var, body } => {
                    let lv = self.evaluate_expr(array_var)?;
                    let sig_info = if let IrExpr::Signal(id, _) = array_var {
                        self.design.top.signals.get(*id)
                    } else { None };
                    let elem_width = sig_info.map(|s| s.elem_width).unwrap_or(1);
                    let count = if elem_width > 0 { lv.width / elem_width } else { 0 };
                    for i in 0..count {
                        if self.disable_pending.is_some() { break; }
                        if self.control_flow.is_some() { self.control_flow = None; break; }
                        let idx_val = LogicVec::from_u64(i as u64, 32);
                        let mut scope = HashMap::new();
                        scope.insert(index_var.clone(), idx_val);
                        let depth = self.method_locals.len();
                        self.method_locals.push(scope);
                        self.evaluate_stmt_block(body)?;
                        self.method_locals.truncate(depth);
                        let cf = self.control_flow.take();
                        if cf == Some(FlowControl::Continue) { continue; }
                        if cf == Some(FlowControl::Break) { break; }
                    }
                }
                IrStmt::MethodCallStmt { obj, method, args, with_clause } => {
                if let IrExpr::Signal(id, _) = obj {
                    let sig_info = self.design.top.signals.get(*id).cloned();
                    if let Some(ref sig) = sig_info {
                        if sig.is_dynamic || sig.is_queue || sig.is_associative {
                            let _ = self.evaluate_array_method(*id, sig, method, args, with_clause.as_deref())?;
                            continue;
                        }
                        if let Some(ref cn) = sig.class_name {
                            let is_cg = self.design.covergroups.iter().any(|c| c.name == *cn);
                            if is_cg || self.design.classes.contains_key(cn) {
                                let obj_val = self.state.read_signal(*id);
                                let obj_id = obj_val.to_u64() as ObjId;
                                if obj_id == 0 && self.state.objects.len() > 0 && self.state.objects[0].class_name.is_empty() {
                                    let class_for_obj = if is_cg {
                                        format!("__covergroup_{}", cn)
                                    } else {
                                        cn.clone()
                                    };
                                    let new_id = self.state.alloc_object(&class_for_obj);
                                    self.state.write_signal(*id, LogicVec::from_u64(new_id as u64, 64));
                                    let arg_vals: Vec<LogicVec> = args.iter()
                                        .map(|a| self.evaluate_expr(a))
                                        .collect::<Result<_, _>>()?;
                                    self.execute_method(new_id, method, &arg_vals)?;
                                    continue;
                                }
                            }
                        }
                    }
                }
                let obj_val = self.evaluate_expr(obj)?;
                let obj_id = obj_val.to_u64() as ObjId;
                let arg_vals: Vec<LogicVec> = args.iter()
                    .map(|a| self.evaluate_expr(a))
                    .collect::<Result<_, _>>()?;
                self.execute_method(obj_id, method, &arg_vals)?;
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
                            self.events[delay_t].push(RegionEvent {
                                region,
                                event: EventKind::ContinueBlock(Continuation {
                                    stmts_to_exec: later,
                                    stmts_remaining: vec![],
                                    fork_id: None,
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
                IrStmt::WaitOrder { events, failure_stmts } => {
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
                IrStmt::Fork { processes, join_type } => {
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
                                    let all_consumed = self.evaluate_block_with_delay_fork(&p, Some(fid))?;
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
                                    let all_consumed = self.evaluate_block_with_delay_fork(&p, Some(fid))?;
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

    fn process_pending_waits(&mut self, deltas: &[SignalId]) -> Result<bool, String> {
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

    fn process_pending_wait_orders(&mut self, deltas: &[SignalId]) -> Result<bool, String> {
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

    fn trigger_sensitive_processes(&mut self, changed: &[(usize, LogicVec, LogicVec)], _t: usize) -> Result<(), String> {
        let processes = self.design.top.processes.clone();
        for (pid, process) in processes.iter().enumerate() {
            match process {
                Process::Combinational { sensitivity, body, .. } => {
                    let should_trigger = sensitivity.is_empty()
                        || changed.iter().any(|(id, _, _)| sensitivity.contains(id));
                    if should_trigger {
                        self.evaluate_stmt_block(body)?;
                    }
                }
                Process::CombReactive { sensitivity, .. } => {
                    let should_trigger = sensitivity.is_empty()
                        || changed.iter().any(|(id, _, _)| sensitivity.contains(id));
                    if should_trigger {
                        self.reactive_events.push(EventKind::EvalProcess(pid));
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
            if !self.is_forced(&lvalue) {
                let _ = self.write_lvalue(&lvalue, val);
            }
        }
    }

    fn signal_id_from_lvalue(&self, lvalue: &IrLValue) -> Option<SignalId> {
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
        self.signal_id_from_lvalue(lvalue).map_or(false, |id| self.forced_signals.contains(&id))
    }

    fn eval_assign_rhs(&mut self, expr: &IrExpr, lhs: &IrLValue) -> Result<LogicVec, String> {
        if let IrExpr::FillLit(v) = expr {
            let w = self.get_lvalue_width(lhs);
            Ok(LogicVec::fill(*v, w))
        } else if let IrExpr::Signed(inner) = expr {
            let mut val = self.evaluate_expr(inner)?;
            let target_w = self.get_lvalue_width(lhs);
            if val.width < target_w {
                let msb = val.bits.last().copied().unwrap_or(LogicVal::Zero);
                val.bits.resize(target_w, msb);
                val.width = target_w;
            }
            Ok(val)
        } else if let IrExpr::NewCall { class_name, args } = expr {
            if class_name.is_empty() && args.len() == 1 {
                let size_val = self.evaluate_expr(&args[0])?;
                let size = size_val.to_u64() as usize;
                if let Some(sig_id) = self.signal_id_from_lvalue(lhs) {
                    let elem_width = self.design.top.signals[sig_id].elem_width;
                    Ok(LogicVec::fill(LogicVal::X, size * elem_width))
                } else {
                    self.evaluate_expr(expr)
                }
            } else {
                self.evaluate_expr(expr)
            }
        } else {
            self.evaluate_expr(expr)
        }
    }

    fn evaluate_expr(&mut self, expr: &IrExpr) -> Result<LogicVec, String> {
        match expr {
            IrExpr::Const(val) => Ok(val.clone()),
            IrExpr::FillLit(val) => Ok(LogicVec::fill(*val, 1)),
            IrExpr::Signal(id, _) => {
                let mut val = self.state.read_signal(*id).clone();
                sanitize_for_2state(&self.design.top.signals, *id, &mut val);
                Ok(val)
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
                let key_val = self.evaluate_expr(index)?;
                // Check if this is an associative array
                let sig_info = self.design.top.signals.get(*sig_id);
                if sig_info.map(|s| s.is_associative).unwrap_or(false) {
                    let assoc_map = self.assoc_data.entry(*sig_id).or_insert_with(HashMap::new);
                    if let Some(val) = assoc_map.get(&key_val) {
                        return Ok(val.clone());
                    }
                    return Ok(LogicVec::new(*elem_width));
                }
                let array_val = self.state.read_signal(*sig_id).clone();
                let idx = key_val.to_u64() as usize;
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
                let lhs_is_real = matches!(lhs.as_ref(), IrExpr::Signal(id, _) if self.design.top.signals.get(*id).map(|s| s.is_real).unwrap_or(false));
                let rhs_is_real = matches!(rhs.as_ref(), IrExpr::Signal(id, _) if self.design.top.signals.get(*id).map(|s| s.is_real).unwrap_or(false));
                if lhs_is_real || rhs_is_real {
                    let a = f64::from_bits(lval.to_u64());
                    let b = f64::from_bits(rval.to_u64());
                    let result = match op {
                        BinaryIrOp::Add => a + b,
                        BinaryIrOp::Sub => a - b,
                        BinaryIrOp::Mul => a * b,
                        BinaryIrOp::Div => a / b,
                        BinaryIrOp::Lt => return Ok(LogicVec::from_u64(if a < b { 1 } else { 0 }, 32)),
                        BinaryIrOp::Le => return Ok(LogicVec::from_u64(if a <= b { 1 } else { 0 }, 32)),
                        BinaryIrOp::Gt => return Ok(LogicVec::from_u64(if a > b { 1 } else { 0 }, 32)),
                        BinaryIrOp::Ge => return Ok(LogicVec::from_u64(if a >= b { 1 } else { 0 }, 32)),
                        BinaryIrOp::Eq => return Ok(LogicVec::from_u64(if a == b { 1 } else { 0 }, 32)),
                        BinaryIrOp::Neq => return Ok(LogicVec::from_u64(if a != b { 1 } else { 0 }, 32)),
                        _ => return Ok(eval_binary(op.clone(), &lval, &rval)),
                    };
                    Ok(LogicVec::from_u64(result.to_bits(), 64))
                } else if matches!(op, BinaryIrOp::Lt | BinaryIrOp::Le | BinaryIrOp::Gt | BinaryIrOp::Ge)
                    && (is_signed_expr(lhs.as_ref(), &self.design.top.signals)
                        || is_signed_expr(rhs.as_ref(), &self.design.top.signals))
                {
                    Ok(eval_binary_signed(op.clone(), &lval, &rval))
                } else {
                    Ok(eval_binary(op.clone(), &lval, &rval))
                }
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
                        let val: i32 = self.rng.gen();
                        Ok(LogicVec::from_u64(val as u64, 32))
                    }
                    "$urandom" => {
                        let val: u32 = self.rng.gen();
                        Ok(LogicVec::from_u64(val as u64, 32))
                    }
                    "$urandom_range" => {
                        let args_eval: Vec<LogicVec> = args.iter()
                            .map(|a| self.evaluate_expr(a))
                            .collect::<Result<_, _>>()?;
                        let maxval = args_eval.first().map(|v| v.to_u64()).unwrap_or(0);
                        let minval = args_eval.get(1).map(|v| v.to_u64()).unwrap_or(0);
                        if maxval <= minval {
                            Ok(LogicVec::from_u64(minval, 32))
                        } else {
                            let range = maxval - minval + 1;
                            let val: u64 = if range <= 1 {
                                minval
                            } else {
                                minval + (self.rng.gen::<u64>() % range)
                            };
                            Ok(LogicVec::from_u64(val, 32))
                        }
                    }
                    "$signed" => {
                        if let Some(arg) = args.first() {
                            let val = self.evaluate_expr(arg)?;
                            // Sign-extend: copy the MSB to all higher bits
                            if val.width > 0 {
                                let msb = val.bits.last().copied().unwrap_or(LogicVal::Zero);
                                let new_width = val.width.max(1);
                                let mut bits = val.bits.clone();
                                bits.resize(new_width, msb);
                                Ok(LogicVec { width: new_width, bits })
                            } else {
                                Ok(val)
                            }
                        } else {
                            Err("$signed expects 1 argument".to_string())
                        }
                    }
                    "$unsigned" => {
                        if let Some(arg) = args.first() {
                            let val = self.evaluate_expr(arg)?;
                            // Unsigned: zero-extend (already the default)
                            Ok(val)
                        } else {
                            Err("$unsigned expects 1 argument".to_string())
                        }
                    }
                    "$fopen" => {
                        let fname = args.first().and_then(|a| {
                            if let IrExpr::String(s) = a { Some(s.clone()) }
                            else { None }
                        });
                        if let Some(fname) = fname {
                            let mode = args.get(1).and_then(|a| if let IrExpr::String(s) = a { Some(s.as_str()) } else { None });
                            let open_result = match mode {
                                Some("r") | Some("rb") => std::fs::File::open(&fname),
                                _ => std::fs::OpenOptions::new()
                                    .read(true).write(true).create(true).truncate(true)
                                    .open(&fname),
                            };
                            match open_result {
                                Ok(f) => {
                                    let handle = self.next_file_handle;
                                    self.next_file_handle += 1;
                                    self.file_handles.insert(handle, f);
                                    self.file_read_pos.insert(handle, 0);
                                    Ok(LogicVec::from_u64(handle as u64, 32))
                                }
                                Err(_) => Ok(LogicVec::from_u64(0, 32))
                            }
                        } else {
                            Ok(LogicVec::from_u64(0, 32))
                        }
                    }
                    "$fdisplay" => {
                        let handle = args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                let msg = format_display(&self.state, &self.design.top.signals, &self.design.hier_signal_map, &self.assoc_data, &args[1..]);
                                let _ = write!(f, "{}", msg);
                            }
                        }
                        Ok(LogicVec::from_u64(0, 1))
                    }
                    "$fread" => {
                        let target = args.first().and_then(|a| if let IrExpr::Signal(id, _) = a { Some(*id) } else { None });
                        let src = args.get(1);
                        let data = if let Some(IrExpr::String(fname)) = src {
                            std::fs::read(fname).ok()
                        } else if let Some(arg) = src {
                            let handle = self.evaluate_expr(arg).ok().map(|v| v.to_u64() as u32).unwrap_or(0);
                            if handle > 0 {
                                use std::io::Read;
                                self.file_handles.get_mut(&handle).and_then(|f| {
                                    let mut buf = Vec::new();
                                    f.read_to_end(&mut buf).ok().map(|_| buf)
                                })
                            } else { None }
                        } else { None };
                        if let (Some(sid), Some(bytes)) = (target, data) {
                            let mut bits = Vec::with_capacity(bytes.len() * 8);
                            for byte in bytes {
                                for i in 0..8 {
                                    bits.push(if (byte >> i) & 1 == 1 { LogicVal::One } else { LogicVal::Zero });
                                }
                            }
                            self.state.write_signal(sid, LogicVec { width: bits.len(), bits });
                        }
                        Ok(LogicVec::from_u64(0, 1))
                    }
                    "$fclose" => {
                        let handle = args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            self.file_handles.remove(&h);
                        }
                        Ok(LogicVec::from_u64(0, 1))
                    }
                    "$fscanf" => {
                        let handle = args.first().and_then(|a| self.evaluate_expr(a).ok().map(|v| v.to_u64() as u32));
                        if let Some(h) = handle {
                            if let Some(f) = self.file_handles.get_mut(&h) {
                                use std::io::{Seek, Read};
                                let read_pos = self.file_read_pos.entry(h).or_insert(0);
                                f.seek(std::io::SeekFrom::Start(*read_pos)).ok();
                                let mut content = String::new();
                                let _bytes_read = f.read_to_string(&mut content).unwrap_or(0);
                                *read_pos = f.stream_position().unwrap_or(0);
                                let fmt = args.get(1).and_then(|a| if let IrExpr::String(s) = a { Some(s.clone()) } else { None });
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
                                                        if let Ok(val) = if spec == 'h' { i64::from_str_radix(tok, 16) } else if spec == 'b' { i64::from_str_radix(tok, 2) } else { tok.parse::<i64>() } {
                                                            let out_idx = 2 + ai;
                                                            if let Some(arg) = args.get(out_idx) {
                                                                if let IrExpr::Signal(sid, _) = arg {
                                                                    self.state.write_signal(*sid, LogicVec::from_u64(val as u64, 32));
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
                                    // $fscanf returns number of items matched (or EOF)
                                    return Ok(LogicVec::from_u64(ai as u64, 32));
                                }
                            }
                        }
                        Ok(LogicVec::from_u64(0, 32))
                    }
                    "$sformatf" => {
                        if args.is_empty() {
                            return Ok(LogicVec::new(0));
                        }
            let msg = format_display(&self.state, &self.design.top.signals, &self.design.hier_signal_map, &self.assoc_data, args);
                        let mut bits = Vec::with_capacity(msg.len() * 8);
                        for c in msg.chars() {
                            let byte = c as u8;
                            for i in 0..8 {
                                bits.push(if (byte >> i) & 1 == 1 { LogicVal::One } else { LogicVal::Zero });
                            }
                        }
                        Ok(LogicVec { width: bits.len(), bits })
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
                    "$realtime" => {
                        let t = self.state.time as f64;
                        Ok(LogicVec::from_u64(t.to_bits(), 64))
                    }
                    "process::self" => {
                        let pid = self.current_process_id.unwrap_or(0);
                        if pid == 0 {
                            let pid = self.state.alloc_object("__process");
                            self.process_map.insert(pid, ProcessInfo {
                                status: ProcessStatus::Running,
                                await_continuations: Vec::new(),
                            });
                            self.current_process_id = Some(pid);
                        }
                        Ok(LogicVec::from_u64(self.current_process_id.unwrap_or(0) as u64, 64))
                    }
                    "uvm_config_db::set" => {
                        let arg_vals: Vec<LogicVec> = args.iter()
                            .map(|a| self.evaluate_expr(a))
                            .collect::<Result<_, _>>()?;
                        let inst_name = if arg_vals.len() > 1 { logicvec_to_string(&arg_vals[1]) } else { String::new() };
                        let field_name = if arg_vals.len() > 2 { logicvec_to_string(&arg_vals[2]) } else { String::new() };
                        let value = if arg_vals.len() > 3 { arg_vals[3].clone() } else { LogicVec::new(1) };
                        self.uvm_config_db_data.insert((inst_name, field_name), value);
                        Ok(LogicVec::from_u64(1, 1))
                    }
                    "uvm_config_db::get" => {
                        let arg_vals: Vec<LogicVec> = args.iter()
                            .map(|a| self.evaluate_expr(a))
                            .collect::<Result<_, _>>()?;
                        let inst_name = if arg_vals.len() > 1 { logicvec_to_string(&arg_vals[1]) } else { String::new() };
                        let field_name = if arg_vals.len() > 2 { logicvec_to_string(&arg_vals[2]) } else { String::new() };
                        let key = (inst_name, field_name);
                        let stored = self.uvm_config_db_data.get(&key).cloned();
                        if let Some(val) = stored {
                            if let Some(last_arg) = args.get(3) {
                                if let IrExpr::Signal(sig_id, _) = last_arg {
                                    self.state.write_signal(*sig_id, val);
                                }
                            }
                            Ok(LogicVec::from_u64(1, 1))
                        } else {
                            Ok(LogicVec::from_u64(0, 1))
                        }
                    }
                    "uvm_resource_db::set" => {
                        let arg_vals: Vec<LogicVec> = args.iter()
                            .map(|a| self.evaluate_expr(a))
                            .collect::<Result<_, _>>()?;
                        let scope = if !arg_vals.is_empty() { logicvec_to_string(&arg_vals[0]) } else { String::new() };
                        let name = if arg_vals.len() > 1 { logicvec_to_string(&arg_vals[1]) } else { String::new() };
                        let value = if arg_vals.len() > 2 { arg_vals[2].clone() } else { LogicVec::new(1) };
                        self.uvm_resource_db_data.insert((scope, name), value);
                        Ok(LogicVec::from_u64(1, 1))
                    }
                    "uvm_resource_db::get" => {
                        let arg_vals: Vec<LogicVec> = args.iter()
                            .map(|a| self.evaluate_expr(a))
                            .collect::<Result<_, _>>()?;
                        let scope = if !arg_vals.is_empty() { logicvec_to_string(&arg_vals[0]) } else { String::new() };
                        let rname = if arg_vals.len() > 1 { logicvec_to_string(&arg_vals[1]) } else { String::new() };
                        let key = (scope, rname);
                        let stored = self.uvm_resource_db_data.get(&key).cloned();
                        if let Some(val) = stored {
                            if let Some(last_arg) = args.get(2) {
                                if let IrExpr::Signal(sig_id, _) = last_arg {
                                    self.state.write_signal(*sig_id, val);
                                }
                            }
                            Ok(LogicVec::from_u64(1, 1))
                        } else {
                            Ok(LogicVec::from_u64(0, 1))
                        }
                    }
                    "uvm_factory::set_type_override_by_type" => {
                        let arg_vals: Vec<LogicVec> = args.iter()
                            .map(|a| self.evaluate_expr(a))
                            .collect::<Result<_, _>>()?;
                        let orig = if !arg_vals.is_empty() { logicvec_to_string(&arg_vals[0]) } else { String::new() };
                        let override_type = if arg_vals.len() > 1 { logicvec_to_string(&arg_vals[1]) } else { String::new() };
                        self.factory_type_overrides.insert(orig, override_type);
                        Ok(LogicVec::from_u64(1, 1))
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
                // Check if this is a covergroup instantiation
                let is_cg = self.design.covergroups.iter().any(|c| c.name == *class_name);
                let effective_name = if is_cg {
                    format!("__covergroup_{}", class_name)
                } else if let Some(override_type) = self.factory_type_overrides.get(class_name) {
                    override_type.clone()
                } else {
                    class_name.clone()
                };
                let obj_id = self.state.alloc_object(&effective_name);
                if class_name == "__mailbox" {
                    self.mailbox_queues.insert(obj_id, Vec::new());
                } else if class_name == "__semaphore" {
                    let init = if !arg_vals.is_empty() { arg_vals[0].to_u64() as u32 } else { 0 };
                    self.semaphore_counts.insert(obj_id, init);
                } else if is_cg {
                    // Auto-sample covergroup immediately on new()
                    self.sample_covergroup(&class_name)?;
                } else if !class_name.is_empty() {
                    if let Some(cls) = self.design.classes.get(class_name.as_str()) {
                        if let Some(obj) = self.state.get_object_mut(obj_id) {
                            for field in &cls.fields {
                                obj.fields.entry(field.name.clone()).or_insert_with(|| LogicVec::from_u64(0, field.width));
                            }
                        }
                    }
                    if self.is_uvm_object_hierarchy(&class_name) {
                        self.uvm_object_data.entry(obj_id).or_insert_with(|| UvmObjectData { name: String::new() });
                    }
                    if self.is_uvm_analysis_port_hierarchy(&class_name) {
                        let pname = if !arg_vals.is_empty() { logicvec_to_string(&arg_vals[0]) } else { String::new() };
                        self.uvm_analysis_port_data.entry(obj_id).or_insert_with(|| UvmAnalysisPortData { connections: Vec::new(), name: pname.clone() });
                        self.uvm_object_data.entry(obj_id).or_insert_with(|| UvmObjectData { name: pname });
                    }
                    if self.is_uvm_analysis_imp_hierarchy(&class_name) {
                        let pname = if !arg_vals.is_empty() { logicvec_to_string(&arg_vals[0]) } else { String::new() };
                        let parent_obj = arg_vals.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                        self.uvm_analysis_imp_data.entry(obj_id).or_insert_with(|| UvmAnalysisImpData { parent: if parent_obj != 0 { Some(parent_obj) } else { None }, name: pname.clone() });
                        self.uvm_object_data.entry(obj_id).or_insert_with(|| UvmObjectData { name: pname });
                    }
                    if self.is_uvm_component_hierarchy(&class_name) {
                        let name = logicvec_to_string(&arg_vals[0]);
                        let parent_obj = arg_vals.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                        self.uvm_object_data.insert(obj_id, UvmObjectData { name: name.clone() });
                        let mut cd = UvmComponentData { parent: None, children: Vec::new(), report_verbosity: 2 };
                        if parent_obj != 0 {
                            cd.parent = Some(parent_obj);
                            if let Some(pd) = self.uvm_component_data.get_mut(&parent_obj) {
                                pd.children.push(obj_id);
                            }
                        }
                        self.uvm_component_data.insert(obj_id, cd);
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
            IrExpr::MethodCall { obj, method, args, with_clause } => {
                if let IrExpr::String(s) = obj.as_ref() {
                    let arg_vals: Vec<LogicVec> = args.iter()
                        .map(|a| self.evaluate_expr(a))
                        .collect::<Result<_, _>>()?;
                    let result = evaluate_string_method(s, method, &arg_vals)?;
                    return Ok(result);
                }
                if let IrExpr::Signal(id, _) = obj.as_ref() {
                    if let Some(sig) = self.design.top.signals.get(*id) {
                        if sig.is_string {
                            let lv = self.state.read_signal(*id);
                            let s = logicvec_to_string(lv);
                            let arg_vals: Vec<LogicVec> = args.iter()
                                .map(|a| self.evaluate_expr(a))
                                .collect::<Result<_, _>>()?;
                            let result = evaluate_string_method(&s, method, &arg_vals)?;
                            return Ok(result);
                        }
                    }
                    if let Some(sig) = self.design.top.signals.get(*id) {
                        if let Some(ref cn) = sig.class_name {
                            let is_arr = sig.is_dynamic || sig.is_queue;
                            if !is_arr && !sig.is_string {
                                // Check if this class_name matches a covergroup or class
                                let is_cg = self.design.covergroups.iter().any(|c| c.name == *cn);
                                if is_cg || self.design.classes.contains_key(cn)
                                {
                                    let obj_val = self.state.read_signal(*id);
                                    let obj_id = obj_val.to_u64() as ObjId;
                                    if obj_id == 0 && self.state.objects.len() > 0 && self.state.objects[0].class_name.is_empty() {
                                        let class_for_obj = if is_cg {
                                            format!("__covergroup_{}", cn)
                                        } else {
                                            cn.clone()
                                        };
                                        let new_id = self.state.alloc_object(&class_for_obj);
                                        self.state.write_signal(*id, LogicVec::from_u64(new_id as u64, 64));
                                        let arg_vals: Vec<LogicVec> = args.iter()
                                            .map(|a| self.evaluate_expr(a))
                                            .collect::<Result<_, _>>()?;
                                        return self.execute_method(new_id, method, &arg_vals);
                                    }
                                }
                            }
                        }
                    }
                    let is_arr = self.design.top.signals.get(*id).map(|s| s.is_dynamic || s.is_queue).unwrap_or(false);
                    if is_arr {
                        let sig_info = self.design.top.signals[*id].clone();
                        return self.evaluate_array_method(*id, &sig_info, method, args, with_clause.as_deref());
                    }
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
            IrExpr::DpiCall { name, args, return_width } => {
                self.evaluate_dpi_call(name, args, *return_width)
            }
            IrExpr::HierRef(name) => {
                if let Some(sig_id) = self.find_signal(name) {
                    let mut val = self.state.read_signal(sig_id).clone();
                    sanitize_for_2state(&self.design.top.signals, sig_id, &mut val);
                    Ok(val)
                } else {
                    Err(format!("hierarchical signal '{}' not found", name))
                }
            }
            IrExpr::Inside { expr, list } => {
                let val = self.evaluate_expr(expr)?;
                for item in list {
                    let item_val = self.evaluate_expr(item)?;
                    let eq = val.case_eq(&item_val);
                    if eq == LogicVec::from_u64(1, 1) {
                        return Ok(LogicVec::from_u64(1, 1));
                    }
                }
                Ok(LogicVec::from_u64(0, 1))
            }
            IrExpr::Dist { expr: _expr, items } => {
                // Dist expression in randomize context: use weighted random selection
                if self.current_method.as_deref() == Some("randomize") {
                    let total_weight: i64 = items.iter().map(|item| item.weight).sum();
                    if total_weight > 0 {
                        let r = (self.rng.gen::<u64>() % total_weight as u64) as i64;
                        let mut cumulative = 0i64;
                        for item in items {
                            cumulative += item.weight;
                            if r < cumulative {
                                let v = match (item.range_lo, item.range_hi) {
                                    (Some(lo), Some(hi)) if hi >= lo => {
                                        lo + (self.rng.gen::<u64>() % ((hi - lo + 1) as u64)) as i64
                                    }
                                    (Some(v), _) | (_, Some(v)) => v,
                                    _ => 0i64,
                                };
                                return Ok(LogicVec::from_u64(v as u64, 32));
                            }
                        }
                    }
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            IrExpr::Cast { width, expr } => {
                let val = self.evaluate_expr(expr)?;
                Ok(val.resize(*width))
            }
            IrExpr::StreamingConcat { op, slices } => {
                let mut vals = Vec::new();
                for sl in slices {
                    vals.push(self.evaluate_expr(sl)?);
                }
                if op == ">>" {
                    let mut all_bits = Vec::new();
                    for v in &vals {
                        all_bits.extend(v.bits.iter());
                    }
                    all_bits.reverse();
                    Ok(LogicVec { width: all_bits.len(), bits: all_bits })
                } else {
                    let mut result = LogicVec::new(0);
                    for v in vals.iter().rev() {
                        result = result.extend(v);
                    }
                    Ok(result)
                }
            }
        }
    }

    fn write_lvalue(&mut self, lvalue: &IrLValue, mut val: LogicVec) -> Result<(), String> {
        match lvalue {
            IrLValue::Signal(id, _) => {
                sanitize_for_2state(&self.design.top.signals, *id, &mut val);
                let is_str = self.design.top.signals.get(*id).map(|s| s.is_string).unwrap_or(false);
                let sig_info = self.design.top.signals.get(*id).cloned();
                let is_dyn = sig_info.as_ref().map(|s| s.is_dynamic || s.is_queue).unwrap_or(false);
                let resized = if is_str || is_dyn {
                    val
                } else {
                    let target_width = self.state.read_signal(*id).width;
                    if val.width != target_width {
                        val.resize(target_width)
                    } else {
                        val
                    }
                };
                // Apply resolution for multi-driver nets
                if let Some(ref info) = sig_info {
                    if info.multi_driver && (info.kind == SignalKind::Wire || info.kind == SignalKind::Inout) {
                        let current = self.state.read_signal(*id).clone();
                        let resolved = resolve_net_values(info.net_type, &current, &resized);
                        self.state.write_signal(*id, resolved);
                        return Ok(());
                    }
                }
                self.state.write_signal(*id, resized);
            }
            IrLValue::RangeSelect(sig_id, msb, lsb) => {
                sanitize_for_2state(&self.design.top.signals, *sig_id, &mut val);
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
                sanitize_for_2state(&self.design.top.signals, *sig_id, &mut val);
                let mut existing = self.state.read_signal(*sig_id).clone();
                if let Some(b) = val.bits.first() {
                    if *idx < existing.bits.len() {
                        existing.bits[*idx] = *b;
                    }
                }
                self.state.write_signal(*sig_id, existing);
            }
            IrLValue::ArrayIndex { sig_id, index, elem_width } => {
                let key_val = self.evaluate_expr(index)?;
                // Check if this is an associative array
                let sig_info = self.design.top.signals.get(*sig_id);
                if sig_info.map(|s| s.is_associative).unwrap_or(false) {
                    sanitize_for_2state(&self.design.top.signals, *sig_id, &mut val);
                    let assoc_map = self.assoc_data.entry(*sig_id).or_insert_with(HashMap::new);
                    assoc_map.insert(key_val, val);
                    return Ok(());
                }
                sanitize_for_2state(&self.design.top.signals, *sig_id, &mut val);
                let mut existing = self.state.read_signal(*sig_id).clone();
                let idx = key_val.to_u64() as usize;
                let start = idx * elem_width;
                let needed = start + elem_width;
                if needed > existing.width {
                    existing.bits.resize(needed, LogicVal::X);
                    existing.width = needed;
                }
                for (i, b) in val.bits.iter().enumerate() {
                    if start + i < needed {
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

    fn evaluate_dpi_call(&mut self, name: &str, args: &[IrExpr], return_width: usize) -> Result<LogicVec, String> {
        // Check if we have a matching DPI import
        let dpi = self.design.dpi_imports.iter()
            .find(|d| d.name == name)
            .ok_or_else(|| format!("DPI function '{}' not found in imports", name))?;
        if dpi.is_task {
            return Ok(LogicVec::new(0));
        }
        let arg_vals: Vec<LogicVec> = args.iter()
            .map(|a| self.evaluate_expr(a))
            .collect::<Result<_, _>>()?;
        // Known DPI functions
        match name {
            "svBitToInt" | "svToInt" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(LogicVec::from_u64(val.to_u64(), return_width));
                }
                return Ok(LogicVec::from_u64(0, return_width));
            }
            "svBitToLong" | "svToLong" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(LogicVec::from_u64(val.to_u64(), return_width));
                }
                return Ok(LogicVec::from_u64(0, return_width));
            }
            "svToShortReal" | "svToReal" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(val.clone());
                }
                return Ok(LogicVec::from_u64(0, return_width));
            }
            "svIntToBit" | "svToBit" | "svToLogic" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(val.clone());
                }
                return Ok(LogicVec::from_u64(0, return_width));
            }
            "svBitToBitVal" | "svBitToLogicVal" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(val.clone());
                }
                return Ok(LogicVec::from_u64(0, return_width));
            }
            "svRandomize" | "sv$random" | "svUrandom" | "svUrandomRange" => {
                let r: u64 = self.rng.gen();
                Ok(LogicVec::from_u64(r, return_width))
            }
            "$test$plusargs" | "svTestPlusArgs" => {
                // Plusargs not supported — return 0
                Ok(LogicVec::from_u64(0, return_width))
            }
            "$value$plusargs" | "svValuePlusArgs" => {
                Ok(LogicVec::from_u64(0, return_width))
            }
            _ => {
                // Unknown DPI — return 0
                Ok(LogicVec::from_u64(0, return_width))
            }
        }
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
                    Value::Binary { bits, .. } => LogicVec::from_bin(bits),
                    Value::Hex { bits, .. } => LogicVec::from_hex(bits),
                    Value::Octal { bits, .. } => LogicVec::from_hex(bits),
                    Value::Real(r) => Ok(LogicVec::from_u64(r.to_bits(), 64)),
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
            Expr::FuncCall { name, args } if name.ends_with("::new") => {
                let raw_name = name.strip_suffix("::new").unwrap().to_string();
                let is_builtin = matches!(raw_name.as_str(),
                    "uvm_object" | "uvm_component" | "uvm_sequence_item" | "uvm_sequence"
                    | "uvm_sequencer" | "uvm_driver" | "uvm_monitor" | "uvm_scoreboard"
                    | "uvm_analysis_port" | "uvm_analysis_imp" | "uvm_test" | "uvm_report_object"
                    | "uvm_factory" | "uvm_resource_db"
                );
                let effective = if is_builtin { format!("__{}", raw_name) } else { raw_name.clone() };
                let effective = self.factory_type_overrides.get(&effective).unwrap_or(&effective).clone();
                let obj_id = self.state.alloc_object(&effective);
                let arg_vals: Vec<LogicVec> = args.iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                // Initialize built-in data
                if is_builtin {
                    if raw_name == "uvm_analysis_port" {
                        let pname = if !arg_vals.is_empty() { logicvec_to_string(&arg_vals[0]) } else { String::new() };
                        self.uvm_analysis_port_data.insert(obj_id, UvmAnalysisPortData { connections: Vec::new(), name: pname.clone() });
                        self.uvm_object_data.entry(obj_id).or_insert_with(|| UvmObjectData { name: pname });
                    } else if raw_name == "uvm_analysis_imp" {
                        let pname = if !arg_vals.is_empty() { logicvec_to_string(&arg_vals[0]) } else { String::new() };
                        let parent_obj = arg_vals.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                        self.uvm_analysis_imp_data.insert(obj_id, UvmAnalysisImpData { parent: if parent_obj != 0 { Some(parent_obj) } else { None }, name: pname.clone() });
                        self.uvm_object_data.entry(obj_id).or_insert_with(|| UvmObjectData { name: pname });
                    }
                }
                if self.find_method_in_hierarchy(&effective, "new").is_ok() {
                    self.execute_method(obj_id, "new", &arg_vals)?;
                }
                Ok(LogicVec::from_u64(obj_id as u64, 64))
            }
            Expr::FuncCall { name, args } if name == "uvm_config_db::set" => {
                let arg_vals: Vec<LogicVec> = args.iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                let inst_name = if arg_vals.len() > 1 { logicvec_to_string(&arg_vals[1]) } else { String::new() };
                let field_name = if arg_vals.len() > 2 { logicvec_to_string(&arg_vals[2]) } else { String::new() };
                let value = if arg_vals.len() > 3 { arg_vals[3].clone() } else { LogicVec::new(1) };
                self.uvm_config_db_data.insert((inst_name, field_name), value);
                Ok(LogicVec::from_u64(1, 1))
            }
            Expr::FuncCall { name, args } if name == "uvm_config_db::get" => {
                let arg_vals: Vec<LogicVec> = args.iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                let inst_name = if arg_vals.len() > 1 { logicvec_to_string(&arg_vals[1]) } else { String::new() };
                let field_name = if arg_vals.len() > 2 { logicvec_to_string(&arg_vals[2]) } else { String::new() };
                let key = (inst_name, field_name);
                let stored = self.uvm_config_db_data.get(&key).cloned();
                if let Some(val) = stored {
                    if let Some(last_arg) = args.get(3) {
                        match last_arg {
                            Expr::Ident(var) => {
                                self.write_local_or_field(var, val.clone())?;
                            }
                            Expr::MemberAccess { obj, field } => {
                                let obj_val = self.evaluate_ast_expr(obj)?;
                                let obj_id = obj_val.to_u64() as ObjId;
                                if let Some(obj) = self.state.get_object_mut(obj_id) {
                                    obj.fields.insert(field.clone(), val.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                    Ok(LogicVec::from_u64(1, 1))
                } else {
                    Ok(LogicVec::from_u64(0, 1))
                }
            }
            Expr::FuncCall { name, args } if name == "uvm_resource_db::set" => {
                let arg_vals: Vec<LogicVec> = args.iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                let scope = if !arg_vals.is_empty() { logicvec_to_string(&arg_vals[0]) } else { String::new() };
                let rname = if arg_vals.len() > 1 { logicvec_to_string(&arg_vals[1]) } else { String::new() };
                let value = if arg_vals.len() > 2 { arg_vals[2].clone() } else { LogicVec::new(1) };
                self.uvm_resource_db_data.insert((scope, rname), value);
                Ok(LogicVec::from_u64(1, 1))
            }
            Expr::FuncCall { name, args } if name == "uvm_resource_db::get" => {
                let arg_vals: Vec<LogicVec> = args.iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                let scope = if !arg_vals.is_empty() { logicvec_to_string(&arg_vals[0]) } else { String::new() };
                let rname = if arg_vals.len() > 1 { logicvec_to_string(&arg_vals[1]) } else { String::new() };
                let key = (scope, rname);
                let stored = self.uvm_resource_db_data.get(&key).cloned();
                if let Some(val) = stored {
                    if let Some(last_arg) = args.get(2) {
                        match last_arg {
                            Expr::Ident(var) => { self.write_local_or_field(var, val.clone())?; }
                            Expr::MemberAccess { obj, field } => {
                                let obj_val = self.evaluate_ast_expr(obj)?;
                                let obj_id = obj_val.to_u64() as ObjId;
                                if let Some(obj) = self.state.get_object_mut(obj_id) {
                                    obj.fields.insert(field.clone(), val.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                    Ok(LogicVec::from_u64(1, 1))
                } else {
                    Ok(LogicVec::from_u64(0, 1))
                }
            }
            Expr::FuncCall { name, args } if name == "uvm_factory::set_type_override_by_type" => {
                let arg_vals: Vec<LogicVec> = args.iter()
                    .map(|a| self.evaluate_ast_expr(a))
                    .collect::<Result<_, _>>()?;
                let orig = if !arg_vals.is_empty() { logicvec_to_string(&arg_vals[0]) } else { String::new() };
                let override_type = if arg_vals.len() > 1 { logicvec_to_string(&arg_vals[1]) } else { String::new() };
                self.factory_type_overrides.insert(orig, override_type);
                Ok(LogicVec::from_u64(1, 1))
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
            Expr::MethodCall { obj, method, args, with_clause } => {
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
                // Try hierarchical signal reference first
                let hier_name = Self::build_hier_name(obj, field);
                if let Some(sig_id) = self.find_signal(&hier_name) {
                    return Ok(self.state.read_signal(sig_id).clone());
                }
                // Fall back to object field access (class objects)
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
            Expr::Inside { expr: inner, range_list } => {
                let val = self.evaluate_ast_expr(inner)?;
                for item in range_list {
                    let item_val = self.evaluate_ast_expr(item)?;
                    let eq = val.case_eq(&item_val);
                    if eq == LogicVec::from_u64(1, 1) {
                        return Ok(LogicVec::from_u64(1, 1));
                    }
                }
                Ok(LogicVec::from_u64(0, 1))
            }
            Expr::StreamingConcat { op, slices } => {
                let mut vals = Vec::new();
                for sl in slices {
                    vals.push(self.evaluate_ast_expr(sl)?);
                }
                let mut result = LogicVec::new(0);
                if op == ">>" {
                    // Right-to-left: reverse bit order of concatenated result
                    let mut all_bits = Vec::new();
                    for v in &vals {
                        all_bits.extend(v.bits.iter());
                    }
                    all_bits.reverse();
                    result = LogicVec { width: all_bits.len(), bits: all_bits };
                } else {
                    // Left-to-right: reverse slice order
                    for v in vals.iter().rev() {
                        result = result.extend(v);
                    }
                }
                Ok(result)
            }
            Expr::Dist { expr: inner, items } => {
                let inner_val = self.evaluate_ast_expr(inner)?;
                let ir_items = items.iter().map(|di| {
                    match di {
                        crate::ast::DistItem::Value(e, crate::ast::DistWeight::Item(w)) => {
                            let ev = self.evaluate_ast_expr(e).unwrap_or(LogicVec::from_u64(0, 32));
                            crate::ir::IrDistItem { range_lo: Some(ev.to_u64() as i64), range_hi: Some(ev.to_u64() as i64), weight_type: crate::ir::DistWeightType::Item, weight: *w as i64 }
                        }
                        crate::ast::DistItem::Value(e, crate::ast::DistWeight::Range(w)) => {
                            let ev = self.evaluate_ast_expr(e).unwrap_or(LogicVec::from_u64(0, 32));
                            crate::ir::IrDistItem { range_lo: Some(ev.to_u64() as i64), range_hi: Some(ev.to_u64() as i64), weight_type: crate::ir::DistWeightType::Range, weight: *w as i64 }
                        }
                        crate::ast::DistItem::Range(lo, hi, crate::ast::DistWeight::Item(w)) => {
                            let lo_v = self.evaluate_ast_expr(lo).ok().map(|v| v.to_u64() as i64);
                            let hi_v = self.evaluate_ast_expr(hi).ok().map(|v| v.to_u64() as i64);
                            crate::ir::IrDistItem { range_lo: lo_v, range_hi: hi_v, weight_type: crate::ir::DistWeightType::Item, weight: *w as i64 }
                        }
                        crate::ast::DistItem::Range(lo, hi, crate::ast::DistWeight::Range(w)) => {
                            let lo_v = self.evaluate_ast_expr(lo).ok().map(|v| v.to_u64() as i64);
                            let hi_v = self.evaluate_ast_expr(hi).ok().map(|v| v.to_u64() as i64);
                            crate::ir::IrDistItem { range_lo: lo_v, range_hi: hi_v, weight_type: crate::ir::DistWeightType::Range, weight: *w as i64 }
                        }
                    }
                }).collect::<Vec<_>>();
                Ok(self.evaluate_expr(&IrExpr::Dist { expr: Box::new(IrExpr::Const(inner_val)), items: ir_items })?)
            }
            Expr::Cast { dtype, expr: inner } => {
                let val = self.evaluate_ast_expr(inner)?;
                let cast_width = match crate::elaboration::elaborator::parse_type_spec_str(dtype) {
                    Some(_) => {
                        // For AST path, compute width from type string
                        match dtype.as_str() {
                            "bit" | "logic" => 1,
                            "byte" => 8,
                            "shortint" => 16,
                            "int" | "integer" => 32,
                            "longint" | "time" => 64,
                            "real" | "realtime" => 64,
                            _ => val.width,
                        }
                    }
                    None => val.width,
                };
                Ok(val.resize(cast_width))
            }
            Expr::ScopedIdent { package, item } => {
                Err(format!("scoped identifier '{}.{}' not resolved at runtime", package, item))
            }
        }
    }

    fn find_signal(&self, name: &str) -> Option<usize> {
        self.design.top.signals.iter().position(|s| s.name == name)
            .or_else(|| self.design.hier_signal_map.get(name).copied())
    }

    fn build_hier_name(obj: &Expr, field: &str) -> String {
        match obj {
            Expr::Ident(prefix) => format!("{}.{}", prefix, field),
            Expr::MemberAccess { obj: inner, field: inner_field } => {
                format!("{}.{}", Self::build_hier_name(inner, inner_field), field)
            }
            _ => String::new(),
        }
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
            Stmt::ForeachLoop { array_var, index_vars, stmts } => {
                let count = self.get_foreach_count(array_var);
                let iv = index_vars.first().cloned().unwrap_or_else(|| "i".to_string());
                for i in 0..count {
                    let idx_val = LogicVec::from_u64(i as u64, 32);
                    let mut scope = HashMap::new();
                    scope.insert(iv.clone(), idx_val);
                    let depth = self.method_locals.len();
                    self.method_locals.push(scope);
                    for s in stmts {
                        self.evaluate_ast_stmt(s)?;
                    }
                    self.method_locals.truncate(depth);
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
            if !self.is_uvm_test_hierarchy(name) { continue; }
            let count = phase_methods.iter()
                .filter(|pm| cls.methods.iter().any(|m| &m.name == *pm))
                .count();
            if count > 0 && best.as_ref().map_or(true, |b| count > b.1) {
                best = Some((name.clone(), count));
            }
        }
        // fallback: any class with phase methods
        if best.is_none() {
            for (name, cls) in &self.design.classes {
                let count = phase_methods.iter()
                    .filter(|pm| cls.methods.iter().any(|m| &m.name == *pm))
                    .count();
                if count > 0 && best.as_ref().map_or(true, |b| count > b.1) {
                    best = Some((name.clone(), count));
                }
            }
        }
        best.map(|(name, _)| name)
    }

    fn is_uvm_test_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_test" { return true; }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn execute_phases(&mut self) -> Result<(), String> {
        let class_name = match self.find_phase_class_name() {
            Some(c) => c,
            None => return Ok(()),
        };
        // Create root test object once, shared across all phases
        let obj_id = self.state.alloc_object(&class_name);
        self.root_test_obj_id = Some(obj_id);

        // build_phase: root then children
        if self.find_method_in_hierarchy(&class_name, "build_phase").is_ok() {
            self.current_this = Some(obj_id);
            self.execute_method(obj_id, "build_phase", &[])?;
            self.current_this = None;
            self.call_phase_on_children(obj_id, "build_phase")?;
        }
        // connect_phase: root then children
        if self.find_method_in_hierarchy(&class_name, "connect_phase").is_ok() {
            self.current_this = Some(obj_id);
            self.execute_method(obj_id, "connect_phase", &[])?;
            self.current_this = None;
            self.call_phase_on_children(obj_id, "connect_phase")?;
        }
        // run_phase: call root's run_phase (blocking since delays in methods are no-ops)
        if self.find_method_in_hierarchy(&class_name, "run_phase").is_ok() {
            self.current_this = Some(obj_id);
            self.execute_method(obj_id, "run_phase", &[])?;
            self.current_this = None;
        }
        Ok(())
    }

    fn call_phase_on_children(&mut self, obj_id: ObjId, phase: &str) -> Result<(), String> {
        if let Some(cdata) = self.uvm_component_data.get(&obj_id) {
            let children = cdata.children.clone();
            for child_id in children {
                if let Some(obj) = self.state.get_object(child_id) {
                    let child_class = &obj.class_name;
                    if self.find_method_in_hierarchy(child_class, phase).is_ok() {
                        self.current_this = Some(child_id);
                        self.execute_method(child_id, phase, &[])?;
                        self.current_this = None;
                    }
                }
                self.call_phase_on_children(child_id, phase)?;
            }
        }
        Ok(())
    }

    fn is_uvm_object_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_object" { return true; }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_component_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_component" { return true; }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_report_object_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_report_object" { return true; }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_sequence_item_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_sequence_item" { return true; }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_sequence_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_sequence" { return true; }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_sequencer_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_sequencer" { return true; }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_monitor_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_monitor" { return true; }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_analysis_port_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_analysis_port" { return true; }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_analysis_imp_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_analysis_imp" { return true; }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    fn is_uvm_driver_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_driver" { return true; }
            match self.design.classes.get(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
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
        if class_name == "__mailbox" {
            return self.execute_mailbox_method(obj_id, method, args);
        }
        if class_name == "__semaphore" {
            return self.execute_semaphore_method(obj_id, method, args);
        }
        if class_name == "__process" {
            return self.execute_process_method(obj_id, method, args);
        }
        // Covergroup support: sample() records coverage data
        if method == "sample" && class_name.starts_with("__covergroup_") {
            let cg_name = &class_name["__covergroup_".len()..];
            if self.design.covergroups.iter().any(|c| c.name == cg_name) {
                return self.sample_covergroup(cg_name).map(|_| LogicVec::from_u64(1, 1));
            }
        }
        // Check uvm_driver hierarchy (most specific first)
        if self.is_uvm_driver_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_driver_method(obj_id, method, args);
            }
        }
        // Check uvm_sequencer hierarchy
        if self.is_uvm_sequencer_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_sequencer_method(obj_id, method, args);
            }
        }
        // Check uvm_sequence hierarchy
        if self.is_uvm_sequence_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_sequence_method(obj_id, method, args);
            }
        }
        // Check uvm_monitor hierarchy
        if self.is_uvm_monitor_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_monitor_method(obj_id, method, args);
            }
        }
        // Check uvm_analysis_port hierarchy
        if self.is_uvm_analysis_port_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_analysis_port_method(obj_id, method, args);
            }
        }
        // Check uvm_analysis_imp hierarchy
        if self.is_uvm_analysis_imp_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_analysis_imp_method(obj_id, method, args);
            }
        }
        // Check uvm_sequence_item hierarchy
        if self.is_uvm_sequence_item_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_sequence_item_method(obj_id, method, args);
            }
        }
        // Check for uvm_component hierarchy methods — only intercept if class doesn't override
        if self.is_uvm_component_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_component_method(obj_id, method, args);
            }
        }
        // Check for uvm_report_object hierarchy methods — only intercept if class doesn't override
        if self.is_uvm_report_object_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_report_object_method(obj_id, method, args);
            }
        }
        // Check for uvm_object hierarchy methods — only intercept if class doesn't override
        if self.is_uvm_object_hierarchy(&class_name) {
            let has_override = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_override {
                return self.execute_uvm_object_method(obj_id, method, args);
            }
        }

        // Check for built-in randomize() — only if no user-defined override exists
        if method == "randomize" {
            let has_user_method = self.find_method_in_hierarchy(&class_name, method).is_ok();
            if !has_user_method {
                return self.execute_randomize(obj_id, &class_name);
            }
        }
        // Normal dispatch: find method in the full class hierarchy (virtual dispatch)
        let method_def = self.find_method_in_hierarchy(&class_name, method)?.clone();
        self.execute_method_body(Some(obj_id), &method_def, args, method)
    }

    fn execute_randomize(&mut self, obj_id: ObjId, class_name: &str) -> Result<LogicVec, String> {
        // Clone all data we need to avoid borrow conflicts
        let class_def = self.design.classes.get(class_name)
            .ok_or_else(|| format!("class '{}' not found", class_name))?.clone();
        if class_def.rand_fields.is_empty() {
            return Ok(LogicVec::from_u64(1, 1));
        }
        let old_this = self.current_this;
        self.current_this = Some(obj_id);

        // Extract solve...before ordering constraints
        let mut before_map: std::collections::HashMap<String, std::collections::HashSet<String>> = std::collections::HashMap::new();
        for (_, body) in &class_def.constraints {
            for item in body {
                if let ConstraintItem::SolveBefore { vars } = item {
                    if vars.len() >= 2 {
                        let first = &vars[0];
                        for later in &vars[1..] {
                            before_map.entry(first.clone())
                                .or_insert_with(std::collections::HashSet::new)
                                .insert(later.clone());
                        }
                    }
                }
            }
        }

        // Order rand_fields: fields in solve-before come first
        let mut ordered_fields: Vec<String> = Vec::new();
        let mut remaining: std::collections::HashSet<String> = class_def.rand_fields.iter().cloned().collect();
        for fname in &class_def.rand_fields {
            if before_map.contains_key(fname) && remaining.contains(fname) {
                ordered_fields.push(fname.clone());
                remaining.remove(fname);
            }
        }
        for fname in &class_def.rand_fields {
            if remaining.contains(fname) {
                ordered_fields.push(fname.clone());
            }
        }

        let max_attempts = 100;
        let mut seed = self.current_time as u64;
        for _ in 0..max_attempts {
            // Generate random values for each rand field in solve-order
            for fname in &ordered_fields {
                let field_info = class_def.fields.iter().find(|f| &f.name == fname);
                let width = field_info.map(|f| f.width).unwrap_or(1);
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let rv = LogicVec::from_u64(seed, width);
                if let Some(obj) = self.state.objects.get_mut(obj_id) {
                    obj.fields.insert(fname.clone(), rv);
                }
            }

            // Evaluate all constraints (skip SolveBefore items)
            let mut all_satisfied = true;
            for (_, body) in &class_def.constraints {
                for item in body {
                    match item {
                        ConstraintItem::Expr(expr) => {
                            let result = self.evaluate_ast_expr(expr)?;
                            if !result.to_bool().unwrap_or(false) {
                                all_satisfied = false;
                                break;
                            }
                        }
                        ConstraintItem::SolveBefore { .. } => {
                            // Just an ordering hint, skip during evaluation
                        }
                    }
                }
                if !all_satisfied { break; }
            }

            if all_satisfied {
                self.current_this = old_this;
                return Ok(LogicVec::from_u64(1, 1));
            }
        }

        self.current_this = old_this;
        Err(format!("randomize failed: could not satisfy all constraints after {} attempts", max_attempts))
    }

    fn execute_mailbox_method(&mut self, obj_id: ObjId, method: &str, args: &[LogicVec]) -> Result<LogicVec, String> {
        match method {
            "new" => Ok(LogicVec::from_u64(1, 1)),
            "put" => {
                if args.is_empty() { return Err("mailbox::put expects 1 argument".into()); }
                self.mailbox_queues.entry(obj_id).or_default().push(args[0].clone());
                Ok(LogicVec::from_u64(1, 1))
            }
            "get" => {
                let q = self.mailbox_queues.get_mut(&obj_id)
                    .ok_or_else(|| "mailbox not initialized".to_string())?;
                if q.is_empty() { return Ok(LogicVec::default()); }
                Ok(q.remove(0))
            }
            "try_get" => {
                let q = self.mailbox_queues.get_mut(&obj_id)
                    .ok_or_else(|| "mailbox not initialized".to_string())?;
                if q.is_empty() {
                    return Ok(LogicVec::from_u64(0, 1));
                }
                let _ = q.remove(0);
                Ok(LogicVec::from_u64(1, 1))
            }
            "try_put" => {
                if args.is_empty() { return Err("mailbox::try_put expects 1 argument".into()); }
                self.mailbox_queues.entry(obj_id).or_default().push(args[0].clone());
                Ok(LogicVec::from_u64(1, 1))
            }
            "num" => {
                let q = self.mailbox_queues.get(&obj_id)
                    .ok_or_else(|| "mailbox not initialized".to_string())?;
                Ok(LogicVec::from_u64(q.len() as u64, 32))
            }
            _ => Err(format!("unknown mailbox method: {}", method)),
        }
    }

    fn execute_semaphore_method(&mut self, obj_id: ObjId, method: &str, args: &[LogicVec]) -> Result<LogicVec, String> {
        match method {
            "new" => {
                let init = if !args.is_empty() { args[0].to_u64() as u32 } else { 0 };
                self.semaphore_counts.insert(obj_id, init);
                Ok(LogicVec::from_u64(1, 1))
            }
            "get" => {
                let key_count = if !args.is_empty() { args[0].to_u64() as u32 } else { 1 };
                let c = self.semaphore_counts.get_mut(&obj_id)
                    .ok_or_else(|| "semaphore not initialized".to_string())?;
                if *c < key_count { return Err("semaphore::get: insufficient keys".to_string()); }
                *c -= key_count;
                Ok(LogicVec::from_u64(*c as u64, 32))
            }
            "put" => {
                let key_count = if !args.is_empty() { args[0].to_u64() as u32 } else { 1 };
                let c = self.semaphore_counts.get_mut(&obj_id)
                    .ok_or_else(|| "semaphore not initialized".to_string())?;
                *c += key_count;
                Ok(LogicVec::from_u64(*c as u64, 32))
            }
            "try_get" => {
                let key_count = if !args.is_empty() { args[0].to_u64() as u32 } else { 1 };
                let c = self.semaphore_counts.get_mut(&obj_id)
                    .ok_or_else(|| "semaphore not initialized".to_string())?;
                if *c >= key_count {
                    *c -= key_count;
                    Ok(LogicVec::from_u64(1, 1))
                } else {
                    Ok(LogicVec::from_u64(0, 1))
                }
            }
            _ => Err(format!("unknown semaphore method: {}", method)),
        }
    }

    fn execute_process_method(&mut self, _obj_id: ObjId, method: &str, _args: &[LogicVec]) -> Result<LogicVec, String> {
        match method {
            "status" => {
                let status = self.process_map.get(&_obj_id).map(|p| p.status as u64).unwrap_or(0);
                Ok(LogicVec::from_u64(status, 32))
            }
            "kill" => {
                if let Some(pi) = self.process_map.get_mut(&_obj_id) {
                    pi.status = ProcessStatus::Killed;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "await" => {
                let status = self.process_map.get(&_obj_id).map(|p| p.status).unwrap_or(ProcessStatus::Finished);
                if status == ProcessStatus::Finished || status == ProcessStatus::Killed {
                    return Ok(LogicVec::from_u64(1, 1));
                }
                Err("process::await() not yet implemented for non-finished processes".to_string())
            }
            "self" => {
                Ok(LogicVec::from_u64(_obj_id as u64, 64))
            }
            "suspend" => {
                if let Some(pi) = self.process_map.get_mut(&_obj_id) {
                    pi.status = ProcessStatus::Suspended;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "resume" => {
                if let Some(pi) = self.process_map.get_mut(&_obj_id) {
                    pi.status = ProcessStatus::Running;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => Err(format!("unknown process method: {}", method)),
        }
    }

    fn execute_uvm_object_method(&mut self, obj_id: ObjId, method: &str, args: &[LogicVec]) -> Result<LogicVec, String> {
        match method {
            "new" => {
                let name = if !args.is_empty() { logicvec_to_string(&args[0]) } else { String::new() };
                self.uvm_object_data.insert(obj_id, UvmObjectData { name });
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_name" => {
                let data = self.uvm_object_data.get(&obj_id)
                    .ok_or_else(|| "uvm_object not initialized".to_string())?;
                Ok(string_to_logicvec(&data.name))
            }
            "set_name" => {
                let name = if !args.is_empty() { logicvec_to_string(&args[0]) } else { String::new() };
                if let Some(data) = self.uvm_object_data.get_mut(&obj_id) {
                    data.name = name;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_type_name" => {
                let class_name = self.state.get_object(obj_id)
                    .map(|o| o.class_name.clone())
                    .unwrap_or_default();
                Ok(string_to_logicvec(&class_name))
            }
            "print" => {
                let data = self.uvm_object_data.get(&obj_id)
                    .ok_or_else(|| "uvm_object not initialized".to_string())?;
                let class_name = self.state.get_object(obj_id)
                    .map(|o| o.class_name.clone())
                    .unwrap_or_default();
                println!("UVM_INFO @ {}: {} [{}]", self.current_time, data.name, class_name);
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => Err(format!("uvm_object::{} not implemented", method)),
        }
    }

    fn execute_uvm_report_object_method(&mut self, obj_id: ObjId, method: &str, args: &[LogicVec]) -> Result<LogicVec, String> {
        match method {
            "uvm_report_info" => {
                let id = args.get(0).map(|a| logicvec_to_string(a)).unwrap_or_default();
                let msg = args.get(1).map(|a| logicvec_to_string(a)).unwrap_or_default();
                eprintln!("UVM_INFO @ {}: {} [{}]", self.current_time, msg, id);
                Ok(LogicVec::from_u64(1, 1))
            }
            "uvm_report_warning" => {
                let id = args.get(0).map(|a| logicvec_to_string(a)).unwrap_or_default();
                let msg = args.get(1).map(|a| logicvec_to_string(a)).unwrap_or_default();
                eprintln!("UVM_WARNING @ {}: {} [{}]", self.current_time, msg, id);
                Ok(LogicVec::from_u64(1, 1))
            }
            "uvm_report_error" => {
                let id = args.get(0).map(|a| logicvec_to_string(a)).unwrap_or_default();
                let msg = args.get(1).map(|a| logicvec_to_string(a)).unwrap_or_default();
                eprintln!("UVM_ERROR @ {}: {} [{}]", self.current_time, msg, id);
                Ok(LogicVec::from_u64(1, 1))
            }
            "uvm_report_fatal" => {
                let id = args.get(0).map(|a| logicvec_to_string(a)).unwrap_or_default();
                let msg = args.get(1).map(|a| logicvec_to_string(a)).unwrap_or_default();
                eprintln!("UVM_FATAL @ {}: {} [{}]", self.current_time, msg, id);
                self.running = false;
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_object_method(obj_id, method, args),
        }
    }

    fn execute_uvm_component_method(&mut self, obj_id: ObjId, method: &str, args: &[LogicVec]) -> Result<LogicVec, String> {
        match method {
            "new" => {
                let name = if !args.is_empty() { logicvec_to_string(&args[0]) } else { String::new() };
                let parent_obj = args.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                self.uvm_object_data.insert(obj_id, UvmObjectData { name: name.clone() });
                let mut cd = UvmComponentData { parent: None, children: Vec::new(), report_verbosity: 2 };
                if parent_obj != 0 {
                    cd.parent = Some(parent_obj);
                    if let Some(pd) = self.uvm_component_data.get_mut(&parent_obj) {
                        pd.children.push(obj_id);
                    }
                }
                self.uvm_component_data.insert(obj_id, cd);
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_full_name" => {
                let mut names = Vec::new();
                let mut current = Some(obj_id);
                while let Some(id) = current {
                    let n = self.uvm_object_data.get(&id).map(|d| d.name.clone()).unwrap_or_default();
                    names.push(n);
                    current = self.uvm_component_data.get(&id).and_then(|d| d.parent);
                }
                names.reverse();
                let full = names.join(".");
                Ok(string_to_logicvec(&full))
            }
            "get_parent" => {
                let pid = self.uvm_component_data.get(&obj_id).and_then(|d| d.parent).unwrap_or(0);
                Ok(LogicVec::from_u64(pid as u64, 64))
            }
            "get_num_children" => {
                let n = self.uvm_component_data.get(&obj_id).map(|d| d.children.len() as u64).unwrap_or(0);
                Ok(LogicVec::from_u64(n, 32))
            }
            "get_child" => {
                let idx = args.first().map(|a| a.to_u64() as usize).unwrap_or(0);
                let cid = self.uvm_component_data.get(&obj_id)
                    .and_then(|d| d.children.get(idx).copied())
                    .unwrap_or(0);
                Ok(LogicVec::from_u64(cid as u64, 64))
            }
            "has_child" => {
                let name = args.first().map(|a| logicvec_to_string(a)).unwrap_or_default();
                let found = self.uvm_component_data.get(&obj_id)
                    .map(|d| d.children.iter().any(|cid| {
                        self.uvm_object_data.get(cid).map(|od| od.name == name).unwrap_or(false)
                    }))
                    .unwrap_or(false);
                Ok(LogicVec::from_u64(if found { 1 } else { 0 }, 1))
            }
            "set_report_verbosity" => {
                let level = args.first().map(|a| a.to_u64() as u32).unwrap_or(2);
                if let Some(d) = self.uvm_component_data.get_mut(&obj_id) {
                    d.report_verbosity = level;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_report_verbosity" => {
                let level = self.uvm_component_data.get(&obj_id).map(|d| d.report_verbosity).unwrap_or(2);
                Ok(LogicVec::from_u64(level as u64, 32))
            }
            "build_phase" | "connect_phase" | "run_phase" => {
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_report_object_method(obj_id, method, args),
        }
    }

    fn execute_uvm_sequence_item_method(&mut self, obj_id: ObjId, method: &str, args: &[LogicVec]) -> Result<LogicVec, String> {
        match method {
            "new" => {
                let name = if !args.is_empty() { logicvec_to_string(&args[0]) } else { String::new() };
                self.uvm_object_data.entry(obj_id).or_insert_with(|| UvmObjectData { name });
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_type_name" => {
                let class_name = self.state.get_object(obj_id)
                    .map(|o| o.class_name.clone())
                    .unwrap_or_default();
                Ok(string_to_logicvec(&class_name))
            }
            _ => self.execute_uvm_object_method(obj_id, method, args),
        }
    }

    fn execute_uvm_sequence_method(&mut self, obj_id: ObjId, method: &str, args: &[LogicVec]) -> Result<LogicVec, String> {
        match method {
            "new" => {
                let name = if !args.is_empty() { logicvec_to_string(&args[0]) } else { String::new() };
                self.uvm_object_data.entry(obj_id).or_insert_with(|| UvmObjectData { name });
                Ok(LogicVec::from_u64(1, 1))
            }
            "start" => {
                // args[0] = sequencer obj_id
                let seqr_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                // Store sequencer obj_id on the sequence object's fields
                if let Some(obj) = self.state.get_object_mut(obj_id) {
                    obj.fields.insert("__sequencer".to_string(), LogicVec::from_u64(seqr_id as u64, 64));
                }
                // Call body()
                if self.find_method_in_hierarchy(&{
                    self.state.get_object(obj_id).map(|o| o.class_name.clone()).unwrap_or_default()
                }, "body").is_ok() {
                    self.execute_method(obj_id, "body", &[])?;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "body" => Ok(LogicVec::from_u64(1, 1)),
            "start_item" => {
                let item_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                // Get sequencer from stored field
                let seqr_id = self.state.get_object(obj_id)
                    .and_then(|o| o.fields.get("__sequencer"))
                    .map(|v| v.to_u64() as ObjId)
                    .unwrap_or(0);
                if seqr_id != 0 {
                    self.uvm_sequencer_data.entry(seqr_id)
                        .or_insert_with(|| UvmSequencerData { item_queue: Vec::new(), current_item: None })
                        .item_queue.push(item_id);
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "finish_item" => {
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_sequencer" => {
                let seqr_id = self.state.get_object(obj_id)
                    .and_then(|o| o.fields.get("__sequencer"))
                    .cloned()
                    .unwrap_or(LogicVec::from_u64(0, 64));
                Ok(seqr_id)
            }
            "create" => {
                let name = args.first().map(|a| logicvec_to_string(a)).unwrap_or_default();
                // Create a new object of the sequence's type
                let class_name = self.state.get_object(obj_id)
                    .map(|o| o.class_name.clone())
                    .unwrap_or_default();
                let child = self.state.alloc_object(&class_name);
                // Set name on the new object
                self.uvm_object_data.entry(child).or_insert_with(|| UvmObjectData { name });
                // Initialize fields from class def
                if let Some(cls) = self.design.classes.get(&class_name) {
                    if let Some(obj) = self.state.get_object_mut(child) {
                        for field in &cls.fields {
                            obj.fields.entry(field.name.clone()).or_insert_with(|| LogicVec::from_u64(0, field.width));
                        }
                    }
                }
                Ok(LogicVec::from_u64(child as u64, 64))
            }
            _ => self.execute_uvm_sequence_item_method(obj_id, method, args),
        }
    }

    fn execute_uvm_sequencer_method(&mut self, obj_id: ObjId, method: &str, args: &[LogicVec]) -> Result<LogicVec, String> {
        match method {
            "new" => {
                let name = if !args.is_empty() { logicvec_to_string(&args[0]) } else { String::new() };
                let parent_obj = args.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                self.uvm_object_data.insert(obj_id, UvmObjectData { name: name.clone() });
                let mut cd = UvmComponentData { parent: None, children: Vec::new(), report_verbosity: 2 };
                if parent_obj != 0 {
                    cd.parent = Some(parent_obj);
                    if let Some(pd) = self.uvm_component_data.get_mut(&parent_obj) {
                        pd.children.push(obj_id);
                    }
                }
                self.uvm_component_data.insert(obj_id, cd);
                self.uvm_sequencer_data.insert(obj_id, UvmSequencerData { item_queue: Vec::new(), current_item: None });
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_next_item" => {
                let data = self.uvm_sequencer_data.get_mut(&obj_id)
                    .ok_or_else(|| "sequencer not initialized".to_string())?;
                let item = data.item_queue.first().copied().unwrap_or(0);
                data.current_item = data.item_queue.first().copied();
                Ok(LogicVec::from_u64(item as u64, 64))
            }
            "item_done" => {
                if let Some(data) = self.uvm_sequencer_data.get_mut(&obj_id) {
                    if data.current_item.is_some() {
                        data.item_queue.remove(0);
                        data.current_item = None;
                    }
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_component_method(obj_id, method, args),
        }
    }

    fn execute_uvm_driver_method(&mut self, obj_id: ObjId, method: &str, args: &[LogicVec]) -> Result<LogicVec, String> {
        match method {
            "new" => {
                let name = if !args.is_empty() { logicvec_to_string(&args[0]) } else { String::new() };
                let parent_obj = args.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                self.uvm_object_data.insert(obj_id, UvmObjectData { name: name.clone() });
                let mut cd = UvmComponentData { parent: None, children: Vec::new(), report_verbosity: 2 };
                if parent_obj != 0 {
                    cd.parent = Some(parent_obj);
                    if let Some(pd) = self.uvm_component_data.get_mut(&parent_obj) {
                        pd.children.push(obj_id);
                    }
                }
                self.uvm_component_data.insert(obj_id, cd);
                self.uvm_driver_data.insert(obj_id, UvmDriverData { sequencer_id: None, current_item: None });
                Ok(LogicVec::from_u64(1, 1))
            }
            "set_sequencer" => {
                let seqr_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                if let Some(data) = self.uvm_driver_data.get_mut(&obj_id) {
                    data.sequencer_id = Some(seqr_id);
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "get_next_item" => {
                let data = self.uvm_driver_data.get(&obj_id)
                    .ok_or_else(|| "driver not initialized".to_string())?;
                let seqr_id = data.sequencer_id.unwrap_or(0);
                if seqr_id != 0 {
                    self.execute_uvm_sequencer_method(seqr_id, "get_next_item", args)
                } else {
                    Ok(LogicVec::from_u64(0, 64))
                }
            }
            "item_done" => {
                let data = self.uvm_driver_data.get(&obj_id)
                    .ok_or_else(|| "driver not initialized".to_string())?;
                let seqr_id = data.sequencer_id.unwrap_or(0);
                if seqr_id != 0 {
                    self.execute_uvm_sequencer_method(seqr_id, "item_done", args)
                } else {
                    Ok(LogicVec::from_u64(1, 1))
                }
            }
            _ => self.execute_uvm_component_method(obj_id, method, args),
        }
    }

    fn execute_uvm_monitor_method(&mut self, obj_id: ObjId, method: &str, args: &[LogicVec]) -> Result<LogicVec, String> {
        match method {
            "new" => {
                let name = if !args.is_empty() { logicvec_to_string(&args[0]) } else { String::new() };
                let parent_obj = args.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                self.uvm_object_data.insert(obj_id, UvmObjectData { name: name.clone() });
                let mut cd = UvmComponentData { parent: None, children: Vec::new(), report_verbosity: 2 };
                if parent_obj != 0 {
                    cd.parent = Some(parent_obj);
                    if let Some(pd) = self.uvm_component_data.get_mut(&parent_obj) {
                        pd.children.push(obj_id);
                    }
                }
                self.uvm_component_data.insert(obj_id, cd);
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_component_method(obj_id, method, args),
        }
    }

    fn execute_uvm_analysis_port_method(&mut self, obj_id: ObjId, method: &str, args: &[LogicVec]) -> Result<LogicVec, String> {
        match method {
            "new" => {
                let name = if !args.is_empty() { logicvec_to_string(&args[0]) } else { String::new() };
                self.uvm_analysis_port_data.insert(obj_id, UvmAnalysisPortData { connections: Vec::new(), name: name.clone() });
                self.uvm_object_data.entry(obj_id).or_insert_with(|| UvmObjectData { name });
                Ok(LogicVec::from_u64(1, 1))
            }
            "connect" => {
                let imp_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                if let Some(data) = self.uvm_analysis_port_data.get_mut(&obj_id) {
                    data.connections.push(imp_id);
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            "write" => {
                let item_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let connections = self.uvm_analysis_port_data.get(&obj_id)
                    .map(|d| d.connections.clone())
                    .unwrap_or_default();
                for imp_id in &connections {
                    let imp_args = vec![LogicVec::from_u64(item_id as u64, 64)];
                    self.execute_uvm_analysis_imp_method(*imp_id, "write", &imp_args)?;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_object_method(obj_id, method, args),
        }
    }

    fn execute_uvm_analysis_imp_method(&mut self, obj_id: ObjId, method: &str, args: &[LogicVec]) -> Result<LogicVec, String> {
        match method {
            "new" => {
                let name = if !args.is_empty() { logicvec_to_string(&args[0]) } else { String::new() };
                let parent_obj = args.get(1).map(|a| a.to_u64() as ObjId).unwrap_or(0);
                self.uvm_analysis_imp_data.insert(obj_id, UvmAnalysisImpData { parent: Some(parent_obj), name: name.clone() });
                self.uvm_object_data.entry(obj_id).or_insert_with(|| UvmObjectData { name });
                Ok(LogicVec::from_u64(1, 1))
            }
            "write" => {
                let item_id = args.first().map(|a| a.to_u64() as ObjId).unwrap_or(0);
                let parent = self.uvm_analysis_imp_data.get(&obj_id)
                    .and_then(|d| d.parent)
                    .unwrap_or(0);
                let parent_name = if parent != 0 {
                    self.state.get_object(parent)
                        .map(|o| o.class_name.clone())
                        .unwrap_or_default()
                } else { String::new() };
                if parent != 0 && !parent_name.is_empty()
                    && self.find_method_in_hierarchy(&parent_name, "write").is_ok()
                {
                    let write_args = vec![LogicVec::from_u64(item_id as u64, 64)];
                    self.execute_method(parent, "write", &write_args)?;
                }
                Ok(LogicVec::from_u64(1, 1))
            }
            _ => self.execute_uvm_object_method(obj_id, method, args),
        }
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
        // Check hierarchy from most specific to least
        if parent == "__uvm_driver" || self.is_uvm_driver_hierarchy(&parent) {
            return self.execute_uvm_driver_method(obj_id, method, args);
        }
        if parent == "__uvm_monitor" || self.is_uvm_monitor_hierarchy(&parent) {
            return self.execute_uvm_monitor_method(obj_id, method, args);
        }
        if parent == "__uvm_sequencer" || self.is_uvm_sequencer_hierarchy(&parent) {
            return self.execute_uvm_sequencer_method(obj_id, method, args);
        }
        if parent == "__uvm_sequence" || self.is_uvm_sequence_hierarchy(&parent) {
            return self.execute_uvm_sequence_method(obj_id, method, args);
        }
        if parent == "__uvm_sequence_item" || self.is_uvm_sequence_item_hierarchy(&parent) {
            return self.execute_uvm_sequence_item_method(obj_id, method, args);
        }
        if parent == "__uvm_analysis_port" || self.is_uvm_analysis_port_hierarchy(&parent) {
            return self.execute_uvm_analysis_port_method(obj_id, method, args);
        }
        if parent == "__uvm_analysis_imp" || self.is_uvm_analysis_imp_hierarchy(&parent) {
            return self.execute_uvm_analysis_imp_method(obj_id, method, args);
        }
        // Check if parent is uvm_component hierarchy
        if parent == "__uvm_component" || self.is_uvm_component_hierarchy(&parent) {
            return self.execute_uvm_component_method(obj_id, method, args);
        }
        // Check if parent is uvm_report_object hierarchy
        if parent == "__uvm_report_object" || self.is_uvm_report_object_hierarchy(&parent) {
            return self.execute_uvm_report_object_method(obj_id, method, args);
        }
        // Check if parent is uvm_object hierarchy
        if parent == "__uvm_object" || self.is_uvm_object_hierarchy(&parent) {
            return self.execute_uvm_object_method(obj_id, method, args);
        }
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

        let depth = self.method_locals.len();
        self.method_locals.push(local_signals);

        let old_method = self.current_method.clone();
        self.current_method = Some(method.to_string());

        if !method_def.stmts.is_empty() {
            let body = Stmt::Block { stmts: method_def.stmts.clone() };
            self.evaluate_ast_stmt(&body)?;
        }

        let return_val = if method_def.is_task {
            LogicVec::new(0)  // tasks return void
        } else {
            self.get_local(method)
                .unwrap_or_else(|| LogicVec::new(1))
        };

        self.current_method = old_method;
        self.method_locals.truncate(depth);
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

fn format_display(state: &SimulationState, signals: &[SignalInfo], hier_map: &HashMap<String, SignalId>, assoc_data: &HashMap<SignalId, HashMap<LogicVec, LogicVec>>, ir_args: &[IrExpr]) -> String {
    let (fmt_str, start_idx) = if let Some(IrExpr::String(s)) = ir_args.first() {
        (s.clone(), 1)
    } else {
        let mut parts = Vec::new();
        for arg in ir_args {
            if let Ok(val) = eval_display_arg(state, signals, hier_map, assoc_data, arg) {
                parts.push(format!("{}", val));
            }
        }
        return parts.join(" ");
    };

    let value_args: Vec<LogicVec> = ir_args[start_idx..].iter()
        .filter_map(|a| eval_display_arg(state, signals, hier_map, assoc_data, a).ok())
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
                Some('f') => {
                    if let Some(val) = value_args.get(value_idx) {
                        result.push_str(&format!("{}", f64::from_bits(val.to_u64())));
                    }
                    value_idx += 1;
                }
                Some('s') => {
                    if let Some(val) = value_args.get(value_idx) {
                        result.push_str(&logicvec_to_string(val));
                    }
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

fn eval_display_arg(state: &SimulationState, signals: &[SignalInfo], hier_map: &HashMap<String, SignalId>, assoc_data: &HashMap<SignalId, HashMap<LogicVec, LogicVec>>, arg: &IrExpr) -> Result<LogicVec, String> {
    match arg {
        IrExpr::HierRef(name) => {
            if let Some(pos) = signals.iter().position(|s| s.name == *name) {
                Ok(state.read_signal(pos).clone())
            } else if let Some(&pos) = hier_map.get(name) {
                Ok(state.read_signal(pos).clone())
            } else {
                Err(format!("hierarchical signal '{}' not found", name))
            }
        }
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
                    let val = eval_display_arg(state, signals, hier_map, assoc_data, inner)?;
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
                    let val = eval_display_arg(state, signals, hier_map, assoc_data, inner)?;
                    let bit = val.bits.get(*idx).copied().unwrap_or(LogicVal::X);
                    Ok(LogicVec { bits: vec![bit], width: 1 })
                }
                IrExpr::ExprPartSelect(inner, base_expr, width_expr) => {
                    let val = eval_display_arg(state, signals, hier_map, assoc_data, inner)?;
                    let base = eval_display_arg(state, signals, hier_map, assoc_data, base_expr).ok().map(|v| v.to_u64() as usize).unwrap_or(0);
                    let width = eval_display_arg(state, signals, hier_map, assoc_data, width_expr).ok().map(|v| v.to_u64() as usize).unwrap_or(0);
                    if width == 0 || base >= val.width {
                        Ok(LogicVec::new(1))
                    } else {
                        let end = (base + width - 1).min(val.width - 1);
                        let mut bits = val.bits[base..=end].to_vec();
                        bits.reverse();
                        Ok(LogicVec { width: bits.len(), bits })
                    }
                }
                IrExpr::Signed(inner) => eval_display_arg(state, signals, hier_map, assoc_data, inner),
                IrExpr::ArrayIndex { sig_id, index, elem_width } => {
                    let key_val = eval_display_arg(state, signals, hier_map, assoc_data, index).ok().unwrap_or(LogicVec::new(1));
                    let sig_info = signals.get(*sig_id);
                    if sig_info.map(|s| s.is_associative).unwrap_or(false) {
                        if let Some(assoc_map) = assoc_data.get(sig_id) {
                            if let Some(val) = assoc_map.get(&key_val) {
                                return Ok(val.clone());
                            }
                        }
                        return Ok(LogicVec::new(*elem_width));
                    }
                    let array_val = state.read_signal(*sig_id);
                    let idx = key_val.to_u64() as usize;
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

fn signal_is_2state(signals: &[SignalInfo], id: SignalId) -> bool {
    signals.get(id).map(|s| s.is_2state).unwrap_or(false)
}

fn sanitize_for_2state(signals: &[SignalInfo], id: SignalId, val: &mut LogicVec) {
    if !signal_is_2state(signals, id) { return; }
    for bit in val.bits.iter_mut() {
        if *bit == LogicVal::X || *bit == LogicVal::Z {
            *bit = LogicVal::Zero;
        }
    }
}


fn resolve_net_values(net_type: NetType, current: &LogicVec, incoming: &LogicVec) -> LogicVec {
    let width = current.width.max(incoming.width);
    let mut bits = Vec::with_capacity(width);
    for i in 0..width {
        let cur = current.bits.get(i).copied().unwrap_or(LogicVal::Z);
        let inc = incoming.bits.get(i).copied().unwrap_or(LogicVal::Z);
        bits.push(net_type.resolve_bit(cur, inc));
    }
    LogicVec { bits, width }
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

pub fn string_to_logicvec(s: &str) -> LogicVec {
    let mut bits = Vec::with_capacity(s.len() * 8);
    for c in s.chars() {
        let byte = c as u8;
        for b in 0..8 {
            bits.push(if (byte >> b) & 1 == 1 { LogicVal::One } else { LogicVal::Zero });
        }
    }
    LogicVec { width: bits.len(), bits }
}

pub fn logicvec_to_string(lv: &LogicVec) -> String {
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

impl SimulationEngine {

fn check_with_clause(&mut self, with_clause: Option<&IrExpr>, elem: &LogicVec) -> Result<bool, String> {
    if let Some(wc) = with_clause {
        let depth = self.method_locals.len();
        let mut scope = std::collections::HashMap::new();
        scope.insert("item".to_string(), elem.clone());
        self.method_locals.push(scope);
        let result = self.evaluate_expr(wc)?.to_bool().unwrap_or(false);
        self.method_locals.truncate(depth);
        Ok(result)
    } else {
        Ok(true)
    }
}

    fn evaluate_array_method(&mut self, sig_id: SignalId, sig: &SignalInfo, method: &str, args: &[IrExpr], with_clause: Option<&IrExpr>) -> Result<LogicVec, String> {
        // Check if this is an associative array method
        if sig.is_associative {
            // Evaluate args first to avoid borrow conflicts with assoc_data access
            let args_eval: Vec<LogicVec> = args.iter()
                .map(|a| self.evaluate_expr(a))
                .collect::<Result<Vec<_>, String>>()?;
            let assoc_map = self.assoc_data.entry(sig_id).or_insert_with(HashMap::new);
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
                let count = if sig.elem_width > 0 { lv.width / sig.elem_width } else { 0 };
                Ok(LogicVec::from_u64(count as u64, 32))
            }
            "delete" => {
                if let Some(index_expr) = args.first() {
                    let idx_val = self.evaluate_expr(index_expr)?;
                    let idx = idx_val.to_u64() as usize;
                    let lv = self.state.read_signal(sig_id);
                    let elem_width = sig.elem_width;
                    let count = if elem_width > 0 { lv.width / elem_width } else { 0 };
                    if idx >= count {
                        return Err(format!("delete index {} out of range (size {})", idx, count));
                    }
                    let before = lv.bits[..idx * elem_width].to_vec();
                    let after = lv.bits[(idx + 1) * elem_width..].to_vec();
                    let mut remaining = Vec::with_capacity(before.len() + after.len());
                    remaining.extend(before);
                    remaining.extend(after);
                    let new_lv = LogicVec { width: remaining.len(), bits: remaining };
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
                    return Err("pop_front on empty queue".to_string());
                }
                let mut bits = Vec::with_capacity(elem_width);
                for i in 0..elem_width {
                    bits.push(lv.bits.get(i).copied().unwrap_or(LogicVal::X));
                }
                let result = LogicVec { width: elem_width, bits };
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
                    return Err("pop_back on empty queue".to_string());
                }
                let start = lv.width - elem_width;
                let mut bits = Vec::with_capacity(elem_width);
                for i in start..lv.width {
                    bits.push(lv.bits.get(i).copied().unwrap_or(LogicVal::X));
                }
                let result = LogicVec { width: elem_width, bits };
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
                    return Err("push_front expects 1 argument".to_string());
                };
                let elem_width = sig.elem_width;
                let padded = if arg_val.width >= elem_width {
                    let bits = arg_val.bits[..elem_width].to_vec();
                    LogicVec { width: elem_width, bits }
                } else {
                    let mut bits = arg_val.bits.clone();
                    bits.resize(elem_width, LogicVal::X);
                    LogicVec { width: elem_width, bits }
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
                let index_expr = args.first().ok_or_else(|| "exists expects 1 argument".to_string())?;
                let idx_val = self.evaluate_expr(index_expr)?;
                let idx = idx_val.to_u64() as usize;
                let lv = self.state.read_signal(sig_id);
                let elem_width = sig.elem_width;
                let count = if elem_width > 0 { lv.width / elem_width } else { 0 };
                Ok(LogicVec::from_u64(if idx < count { 1 } else { 0 }, 1))
            }
            "push_back" => {
                let arg_val = if let Some(a) = args.first() {
                    self.evaluate_expr(a)?
                } else {
                    return Err("push_back expects 1 argument".to_string());
                };
                let elem_width = sig.elem_width;
                let padded = if arg_val.width >= elem_width {
                    let bits = arg_val.bits[..elem_width].to_vec();
                    LogicVec { width: elem_width, bits }
                } else {
                    let mut bits = arg_val.bits.clone();
                    bits.resize(elem_width, LogicVal::X);
                    LogicVec { width: elem_width, bits }
                };
                let mut existing = self.state.read_signal(sig_id).clone();
                existing.bits.extend(padded.bits.iter().copied());
                existing.width += elem_width;
                self.state.write_signal(sig_id, existing);
                Ok(LogicVec::new(0))
            }
            "insert" => {
                if args.len() < 2 {
                    return Err("insert expects 2 arguments (index, value)".to_string());
                }
                let idx_val = self.evaluate_expr(&args[0])?;
                let idx = idx_val.to_u64() as usize;
                let arg_val = self.evaluate_expr(&args[1])?;
                let elem_width = sig.elem_width;
                let padded = if arg_val.width >= elem_width {
                    let bits = arg_val.bits[..elem_width].to_vec();
                    LogicVec { width: elem_width, bits }
                } else {
                    let mut bits = arg_val.bits.clone();
                    bits.resize(elem_width, LogicVal::X);
                    LogicVec { width: elem_width, bits }
                };
                let mut existing = self.state.read_signal(sig_id).clone();
                let count = if elem_width > 0 { existing.width / elem_width } else { 0 };
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
                    let count = lv.width / elem_width;
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
                    let count = lv.width / elem_width;
                    let mut elems: Vec<LogicVec> = (0..count).map(|i| {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        LogicVec { width: elem_width, bits }
                    }).collect();
                    elems.sort_by(|a, b| a.to_u64().cmp(&b.to_u64()));
                    let mut new_bits = Vec::with_capacity(lv.width);
                    for e in &elems {
                        new_bits.extend(e.bits.iter().copied());
                    }
                    let sorted = LogicVec { width: lv.width, bits: new_bits };
                    self.state.write_signal(sig_id, sorted);
                }
                Ok(LogicVec::new(0))
            }
            "rsort" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width / elem_width;
                    let mut elems: Vec<LogicVec> = (0..count).map(|i| {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        LogicVec { width: elem_width, bits }
                    }).collect();
                    elems.sort_by(|a, b| b.to_u64().cmp(&a.to_u64()));
                    let mut new_bits = Vec::with_capacity(lv.width);
                    for e in &elems {
                        new_bits.extend(e.bits.iter().copied());
                    }
                    let sorted = LogicVec { width: lv.width, bits: new_bits };
                    self.state.write_signal(sig_id, sorted);
                }
                Ok(LogicVec::new(0))
            }
            "shuffle" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width / elem_width;
                    let mut elems: Vec<LogicVec> = (0..count).map(|i| {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        LogicVec { width: elem_width, bits }
                    }).collect();
                    use rand::seq::SliceRandom;
                    elems.shuffle(&mut rand::thread_rng());
                    let mut new_bits = Vec::with_capacity(lv.width);
                    for e in &elems {
                        new_bits.extend(e.bits.iter().copied());
                    }
                    let shuffled = LogicVec { width: lv.width, bits: new_bits };
                    self.state.write_signal(sig_id, shuffled);
                }
                Ok(LogicVec::new(0))
            }
            // --- Reduction methods ---
            "sum" => {
                let lv = self.state.read_signal(sig_id).clone();
                let elem_width = sig.elem_width;
                if elem_width > 0 {
                    let count = lv.width / elem_width;
                    let mut result: u64 = 0;
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        let elem = LogicVec { width: elem_width, bits };
                        if !self.check_with_clause(with_clause, &elem)? { continue; }
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
                    let count = lv.width / elem_width;
                    let mut result: u64 = 1;
                    for i in 0..count {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[i * elem_width + j]);
                        }
                        let elem = LogicVec { width: elem_width, bits };
                        if !self.check_with_clause(with_clause, &elem)? { continue; }
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
                        let elem = LogicVec { width: elem_width, bits: bits.clone() };
                        if !self.check_with_clause(with_clause, &elem)? { continue; }
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
                        let elem = LogicVec { width: elem_width, bits: bits.clone() };
                        if !self.check_with_clause(with_clause, &elem)? { continue; }
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
                        let elem = LogicVec { width: elem_width, bits: bits.clone() };
                        if !self.check_with_clause(with_clause, &elem)? { continue; }
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
                        let elem = LogicVec { width: elem_width, bits };
                        if !self.check_with_clause(with_clause, &elem)? { continue; }
                        let v = elem.to_u64();
                        if v < min_val { min_val = v; }
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
                        let elem = LogicVec { width: elem_width, bits };
                        if !self.check_with_clause(with_clause, &elem)? { continue; }
                        let v = elem.to_u64();
                        if v > max_val { max_val = v; }
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
                        let elem = LogicVec { width: elem_width, bits };
                        if !self.check_with_clause(with_clause, &elem)? { continue; }
                        if seen.insert(elem.to_u64()) {
                            for j in 0..elem_width {
                                let idx = i * elem_width + j;
                                new_bits.push(lv.bits.get(idx).copied().unwrap_or(LogicVal::X));
                            }
                        }
                    }
                    let result = LogicVec { width: new_bits.len(), bits: new_bits };
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
                                let elem = LogicVec { width: elem_width, bits };
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
                                let elem = LogicVec { width: elem_width, bits };
                                if self.check_with_clause(with_clause, &elem)? {
                                    result_elems.push(elem);
                                    if method == "find_first" { break; }
                                }
                            }
                        }
                        let total_width = result_elems.len() * elem_width;
                        let mut all_bits = Vec::with_capacity(total_width);
                        for e in &result_elems {
                            all_bits.extend(e.bits.iter());
                        }
                        return Ok(LogicVec { width: total_width, bits: all_bits });
                    }
                    if method == "find_first" && count > 0 {
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[j]);
                        }
                        return Ok(LogicVec { width: elem_width, bits });
                    }
                    if method == "find_last" && count > 0 {
                        let start = (count - 1) * elem_width;
                        let mut bits = Vec::with_capacity(elem_width);
                        for j in 0..elem_width {
                            bits.push(lv.bits[start + j]);
                        }
                        return Ok(LogicVec { width: elem_width, bits });
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
                                let elem = LogicVec { width: elem_width, bits };
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
                                let elem = LogicVec { width: elem_width, bits };
                                if self.check_with_clause(with_clause, &elem)? {
                                    indices.push(i as u64);
                                    if method == "find_first_index" { break; }
                                }
                            }
                        }
                        let mut bits = Vec::new();
                        for idx in &indices {
                            let idx_vec = LogicVec::from_u64(*idx, 32);
                            bits.extend(idx_vec.bits.iter());
                        }
                        return Ok(LogicVec { width: bits.len(), bits });
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
                    return Ok(LogicVec { width: bits.len(), bits });
                }
                Ok(LogicVec::new(0))
            }
            _ => Err(format!("unknown array/queue method: {}", method)),
        }
    }
}




