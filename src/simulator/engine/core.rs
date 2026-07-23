use super::SimulationEngine;
use crate::error::SimError;
use crate::ir::*;
use crate::scheduler::clock_domain::ClockDomain;
use crate::simulator::parallel::ParallelConfig;
use crate::simulator::sdf::SdfData;
use crate::simulator::state::SimulationState;
use crate::simulator::types::*;
use crate::Symbol;
use crate::waveform::FstWaveWriter;
use crate::waveform::VcdWriter;
use rand::Rng;
use rand::SeedableRng;
use std::collections::{HashMap, HashSet};

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
            disable_pending: None,
            rng: rand::rngs::StdRng::seed_from_u64(42),
            file_handles: HashMap::new(),
            file_ungetc_buf: HashMap::new(),
            file_read_pos: HashMap::new(),
            next_file_handle: 1,
            monitor_args: None,
            monitor_last_values: None,
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
            jit_evaluator: Some(crate::simulator::JITEvaluator::new()),
            use_packed_eval: false,
            sim_arena: crate::simulator::arena::SimulationArena::with_bump_size(4 * 1024 * 1024), // 4MB initial
            sim_dag: None,
            use_dag_parallel: false,
            clock_analysis: None,
            use_cycle_fusion: false,
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

    pub fn set_use_packed_eval(&mut self, enabled: bool) {
        self.use_packed_eval = enabled;
    }

    pub fn set_use_dag_parallel(&mut self, enabled: bool) {
        self.use_dag_parallel = enabled;
    }

    pub fn set_use_cycle_fusion(&mut self, enabled: bool) {
        self.use_cycle_fusion = enabled;
    }

    pub fn run(&mut self) -> Result<(), SimError> {
        self.initialize_time_zero()?;
        self.execute_phases()?;

        // ── Register thread-local arena untuk zero-deallocation ──
        // Semua LogicVec::new(), fill(), from_u64() otomatis alokasi dari arena
        // selama event loop berjalan. Tidak perlu ubah evaluate_expr() call sites.
        crate::simulator::arena::set_thread_arena(Some(&mut self.sim_arena));

        // ── Build DAG untuk parallel process evaluation ──
        // Hanya untuk Combinational/CombReactive processes yang aman di-paralelkan.
        if self.use_dag_parallel && self.sim_dag.is_none() {
            let dag = crate::scheduler::SimulationDag::build(&self.design);
            if dag.num_processes() > 0 {
                let n_layers = dag.num_layers();
                let avg_par = dag.avg_parallelism();
                eprintln!(
                    "DAG: {} processes, {} layers, avg {:.1} parallelism",
                    dag.num_processes(),
                    n_layers,
                    avg_par
                );
                self.sim_dag = Some(dag);
            }
        }

        // ── Build clock domains untuk cycle-based simulation fusion ──
        if self.use_cycle_fusion && self.clock_analysis.is_none() {
            let analysis = crate::scheduler::ClockDomainAnalysis::analyze(&self.design);
            if analysis.num_domains() > 0 {
                eprintln!(
                    "Cycle fusion: {} clock domains, {} processes fused",
                    analysis.num_domains(),
                    analysis.num_fused_processes(),
                );
                self.clock_analysis = Some(analysis);
            }
        }

        while self.running && self.state.time <= self.max_time {
            let t = self.state.time as usize;

            // ── Zero-deallocation: reset cycle arena (O(1) — bump pointer reset) ──
            // Pool Vec<LogicVal> tetap hidup (backing storage tidak di-free),
            // sehingga alokasi siklus berikutnya langsung reuse backing storage.
            self.sim_arena.reset_cycle();

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

                                    // ── DAG-Parallel: batch EvalProcess events ──
                                    // Hanya Combinational/CombReactive/Initial yang aman
                                    // di-paralelkan. Sequential, AlwaysWithDelay, dan Final
                                    // butuh event loop semantics (clock edges, delay).
                                    if self.use_dag_parallel && self.sim_dag.is_some() {
                                        let mut eval_pids: Vec<usize> = Vec::new();
                                        let mut other_events: Vec<RegionEvent> = Vec::new();
                                        for re in events {
                                            if let EventKind::EvalProcess(pid) = re.event {
                                                if pid < self.design.top.processes.len()
                                                    && crate::scheduler::is_process_parallelizable(
                                                        &self.design.top.processes[pid],
                                                    )
                                                {
                                                    eval_pids.push(pid);
                                                } else {
                                                    other_events.push(re);
                                                }
                                            } else {
                                                other_events.push(re);
                                            }
                                        }

                                        // Process non-EvalProcess events sequentially
                                        for re in other_events {
                                            self.process_event(re.event, t)?;
                                        }

                                        // Process EvalProcess events via DAG parallel
                                        if !eval_pids.is_empty() {
                                            self.evaluate_eval_processes_parallel(
                                                &eval_pids,
                                            )?;
                                        }
                                    } else {
                                        // Sequential: process all events one by one
                                        for re in events {
                                            self.process_event(re.event, t)?;
                                        }
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

        // ── Cleanup: deregister thread-local arena untuk cegah dangling pointer ──
        crate::simulator::arena::set_thread_arena(None);

        Ok(())
    }

    /// Evaluate EvalProcess events in parallel using DAG layers.
    ///
    /// Processes in the same DAG layer are independent (no signal conflicts)
    /// and are evaluated via rayon work-stealing. Writes are collected and
    /// applied after all processes in the layer complete.
    ///
    /// # Lock-Free Design
    ///
    /// Setiap process bekerja pada snapshot sinyal sendiri (clone).
    /// Tidak ada shared mutable state antar process dalam satu layer.
    fn evaluate_eval_processes_parallel(
        &mut self,
        pids: &[usize],
    ) -> Result<(), SimError> {
        // Clone DAG layers + signal snapshot + process bodies upfront
        // untuk menghindari borrow conflicts dengan &mut self.
        let dag_layers: Vec<Vec<usize>> = match &self.sim_dag {
            Some(dag) => dag.layers().to_vec(),
            None => {
                for &pid in pids {
                    self.process_event(EventKind::EvalProcess(pid), self.current_time as usize)?;
                }
                return Ok(());
            }
        };
        let signal_snapshot = self.state.signals.clone();

        // Clone only the process bodies we need (parallelizable ones)
        let mut body_map: HashMap<usize, Vec<IrStmt>> = HashMap::new();
        for &pid in pids {
            if pid < self.design.top.processes.len() {
                let process = &self.design.top.processes[pid];
                if let Some(body) = match process {
                    Process::Combinational { body, .. }
                    | Process::CombReactive { body, .. }
                    | Process::Initial { body, .. } => Some(body.clone()),
                    _ => None,
                } {
                    body_map.insert(pid, body);
                }
            }
        }

        // Evaluate each layer sequentially (processes WITHIN a layer are parallel)
        for layer in &dag_layers {
            let layer_pids: Vec<&usize> = layer.iter().filter(|pid| pids.contains(pid)).collect();
            if layer_pids.is_empty() {
                continue;
            }

            // Evaluate all processes in this layer in parallel via rayon
            // Each worker gets its own signal clone + body reference
            let writes = evaluate_bodies_parallel(
                &layer_pids,
                &body_map,
                &signal_snapshot,
            )?;

            // Apply writes back to state (no borrow conflicts, all data cloned)
            for (sig_id, val) in writes {
                if sig_id < self.state.signals.len() {
                    self.state.write_signal(sig_id, val);
                }
            }
        }

        Ok(())
    }

    pub(crate) fn evaluate_clock_domain(&mut self, domain: &ClockDomain) -> Result<(), SimError> {
        // Clone process bodies upfront untuk hindari borrow conflicts
        let num_sigs = self.state.signals.len();

        // Collect sequential process bodies
        let seq_bodies: Vec<(usize, Vec<IrStmt>)> = domain
            .sequential_processes
            .iter()
            .filter_map(|&pid| {
                if pid < self.design.top.processes.len() {
                    if let Process::Sequential { body, .. } = &self.design.top.processes[pid] {
                        return Some((pid, body.clone()));
                    }
                }
                None
            })
            .collect();

        // Collect follower combinational process bodies
        let follower_bodies: Vec<(usize, Vec<IrStmt>)> = domain
            .follower_processes
            .iter()
            .filter_map(|&pid| {
                if pid < self.design.top.processes.len() {
                    match &self.design.top.processes[pid] {
                        Process::Combinational { body, .. }
                        | Process::CombReactive { body, .. } => {
                            Some((pid, body.clone()))
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .collect();

        // Fixed-point iteration: evaluate sequential + follower until stable
        let max_iter = 10; // Safety limit
        for _iter in 0..max_iter {
            let prev_snapshot: Vec<LogicVec> = (0..num_sigs)
                .map(|i| self.state.read_signal(i).clone())
                .collect();

            // Step 1: Evaluate all sequential processes in the domain
            for (_, body) in &seq_bodies {
                self.evaluate_stmt_block(body)?;
            }

            // Commit NBA from sequential process evaluations
            self.commit_nba();

            // Step 2: Evaluate all follower combinational processes
            for (_, body) in &follower_bodies {
                self.evaluate_stmt_block(body)?;
            }

            // Check if stable: no signal changes
            let mut changed = false;
            for i in 0..num_sigs.min(prev_snapshot.len()) {
                let cur = self.state.read_signal(i);
                if cur != &prev_snapshot[i] {
                    changed = true;
                    break;
                }
            }

            if !changed {
                break; // Converged
            }
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
                self.cover_total.insert(Symbol::intern(&key), 0);
                self.cover_hits.insert(Symbol::intern(&key), 0);
                self.cover_bins.insert(Symbol::intern(&key), HashMap::new());
            }
            for cross in &cg.crosses {
                let key = format!("{}.{}", cg.name, cross.name);
                self.cover_total.insert(Symbol::intern(&key), 0);
                self.cover_hits.insert(Symbol::intern(&key), 0);
                self.cover_bins.insert(Symbol::intern(&key), HashMap::new());
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
