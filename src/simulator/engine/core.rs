use super::SimulationEngine;
use crate::simulator::util::*;
use crate::ast::*;
use crate::error::SimError;
use crate::ir::*;
use crate::Symbol;
use crate::simulator::parallel;
use crate::simulator::state::SimulationState;
use crate::simulator::types::*;
use crate::simulator::value::*;
use crate::waveform::FstWaveWriter;
use crate::waveform::VcdWriter;
use rand::Rng;
use rand::SeedableRng;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;

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
            fst: None,
            current_this: None,
            method_locals: Vec::new(),
            current_method: None,
            rng: rand::rngs::StdRng::seed_from_u64(42),
            file_handles: HashMap::new(),
            file_ungetc_buf: HashMap::new(),
            file_read_pos: HashMap::new(),
            next_file_handle: 1,
            monitor_args: None,
            monitor_last_values: None,
            disable_pending: None,
            control_flow: None,
            forced_signals: HashSet::new(),
            signal_snapshot: None,
            pending_waits: Vec::new(),
            pending_await_target: None,
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
            sdf_timing_checks: Vec::new(),
            uvm_resource_db_data: HashMap::new(),
            factory_type_overrides: HashMap::new(),
            root_test_obj_id: None,
            process_map: HashMap::new(),
            _next_process_id: 1,
            current_process_id: None,
            cover_hits: HashMap::new(),
            cover_total: HashMap::new(),
            cover_bins: HashMap::new(),
            plusargs: HashMap::new(),
            debug_mode: DebugMode::Normal,
            breakpoints: Vec::new(),
            watchpoints: Vec::new(),
            signal_history: HashMap::new(),
            signal_last_change: HashMap::new(),
            udp_prev_args: HashMap::new(),
            parallel_config: ParallelConfig::default(),
            sysfunc_prev: HashMap::new(),
            sysfunc_history: HashMap::new(),
            snapshots: Vec::new(),
            paused: false,
            step_mode: StepMode::Running,
            event_log: Vec::new(),
            snapshot_interval: 1000,
            assert_off_all: false,
            assert_kill_all: false,
            assert_modules_off: HashSet::new(),
            coverage_options: HashMap::new(),
            coverage_enabled: true,
            coverage_model_handles: HashMap::new(),
            next_coverage_model_handle: 1,
            sequence_attempts: Vec::new(),
            recursion_depth: HashMap::new(),
            max_recursion_depth: 256,
            objection_count: 0,
            objection_triggered: false,
        }
    }

    pub fn set_vcd(&mut self, vcd: VcdWriter) {
        self.vcd = Some(vcd);
    }

    pub fn set_fst(&mut self, fst: FstWaveWriter) {
        self.fst = Some(fst);
    }

    pub fn set_parallel_config(&mut self, config: ParallelConfig) {
        self.parallel_config = config;
    }

    pub fn run(&mut self) -> Result<(), SimError> {
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
            self.dump_fst_time()?;

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
                                        } else {
                                            true
                                        }
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
                                        } else {
                                            true
                                        }
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
                                    let events: Vec<RegionEvent> = self.events[t]
                                        .drain(..)
                                        .filter(|re| re.region == region)
                                        .collect();
                                    if events.is_empty() {
                                        break;
                                    }
                                    activity = true;
                                    for re in events {
                                        self.process_event(re.event, t)?;
                                    }
                                    // Inactive re-drains; Active drains once (outer loop
                                    // re-circulates if new events appear later)
                                    if region == EventRegion::Active {
                                        break;
                                    }
                                }
                            }
                        }
                        EventRegion::Nba => {
                            // NBA region: commit pending non-blocking assignments
                            self.commit_nba();
                            if t < self.events.len() {
                                let events: Vec<RegionEvent> = self.events[t]
                                    .drain(..)
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
                                let events: Vec<RegionEvent> = self.events[t]
                                    .drain(..)
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
                    return Err(SimError::runtime(
                        "simulation exceeded max delta cycles per time step (10M)",
                    ));
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
                        matches!(
                            re.region,
                            EventRegion::PreActive
                                | EventRegion::Active
                                | EventRegion::Inactive
                                | EventRegion::PreNba
                                | EventRegion::Nba
                                | EventRegion::PostNba
                                | EventRegion::PreObserved
                                | EventRegion::Observed
                                | EventRegion::PostObserved
                                | EventRegion::Reactive
                                | EventRegion::PostReactive
                        )
                    })
                    || !self.nba_pending.is_empty();

                if has_remaining {
                    activity = true;
                }

                if !activity {
                    break;
                }
            }

            // ── Postponed region: $strobe, $monitor, VCD, timing checks ──
            self.process_strobe()?;
            self.dump_vcd_state()?;
            self.dump_fst_state()?;
            self.check_monitor()?;
            self.check_timing_constraints()?;

            // ── Debug check at start of cycle ──
            if self.debug_mode != DebugMode::Normal {
                self.debug_check()?;
                if self.paused {
                    break;
                }
                if self.step_mode == StepMode::StepCycle {
                    self.paused = true;
                    break;
                }
            }

            // Advance and evaluate sequence attempts
            self.evaluate_sequence_attempts()?;
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

    fn initialize_time_zero(&mut self) -> Result<(), SimError> {
        let t = 0usize;
        let processes = self.design.top.processes.clone();

        // IEEE 1800: initial blocks execute FIRST, then always_comb evaluates.
        // Schedule initial blocks and always-with-delay first,
        // then combinational/reactive processes AFTER.
        // All in Active region, processed in FIFO order by the event loop.

        // Pass 1: Initial blocks (execute first at time 0)
        for (pid, process) in processes.iter().enumerate() {
            if matches!(process, Process::Initial { .. }) {
                self.events[t].push(RegionEvent {
                    region: EventRegion::Active,
                    event: EventKind::EvalProcess(pid),
                });
            }
        }

        // Pass 2: Combinational/Reactive processes (evaluate after initial)
        for (pid, process) in processes.iter().enumerate() {
            if matches!(
                process,
                Process::Combinational { .. } | Process::CombReactive { .. }
            ) {
                self.events[t].push(RegionEvent {
                    region: EventRegion::Active,
                    event: EventKind::EvalProcess(pid),
                });
            }
        }

        // Pass 3: AlwaysWithDelay (time-0 processes that schedule future events)
        for (pid, process) in processes.iter().enumerate() {
            if matches!(process, Process::AlwaysWithDelay { .. }) {
                self.events[t].push(RegionEvent {
                    region: EventRegion::Active,
                    event: EventKind::EvalProcess(pid),
                });
            }
        }

        // Sequential processes wait for edge events, not scheduled at time 0
        // Final processes execute only at $finish

        // Initialize coverage tracking
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

    pub fn annotate_sdf(&mut self, sdf: &SdfData) -> Result<(), SimError> {
        // Apply cell delays to signals (simplified annotation)
        for (_, cell_delay) in &sdf.cell_delays {
            // Try to apply delays to the first matching signal
            if let Some(rise) = cell_delay.rise {
                if let Some(sig) = self.design.top.signals.first_mut() {
                    sig.delay_rise = Some(rise as u64);
                }
            }
            if let Some(fall) = cell_delay.fall {
                if let Some(sig) = self.design.top.signals.first_mut() {
                    sig.delay_fall = Some(fall as u64);
                }
            }
        }

        // Apply net delays to signals
        for (net_name, net_delay) in &sdf.net_delays {
            if let Some(rise) = net_delay.rise {
                for sig in &mut self.design.top.signals {
                    if sig.name == *net_name || sig.name.ends_with(&format!(".{}", net_name)) {
                        sig.delay_rise = Some(rise as u64);
                    }
                }
            }
            if let Some(fall) = net_delay.fall {
                for sig in &mut self.design.top.signals {
                    if sig.name == *net_name || sig.name.ends_with(&format!(".{}", net_name)) {
                        sig.delay_fall = Some(fall as u64);
                    }
                }
            }
        }

        // Store timing checks for later use
        self.sdf_timing_checks = sdf.timing_checks.clone();

        Ok(())
    }

    fn execute_final_blocks(&mut self) -> Result<(), SimError> {
        let bodies: Vec<Vec<IrStmt>> = self
            .design
            .top
            .processes
            .iter()
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

}
