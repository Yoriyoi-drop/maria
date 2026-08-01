pub mod coverage;
pub mod debug;
pub mod engine_utils;
pub mod waveform;
pub mod core;
pub mod scheduler;
pub mod eval;
pub mod uvm;
pub mod sequence;

use crate::diagnostics::DiagSink;
use crate::ir::*;
use crate::scheduler::clock_domain::ClockDomainAnalysis;
use crate::scheduler::SimulationDag;
use crate::simulator::arena::SimulationArena;
use crate::simulator::parallel::ParallelConfig;

use crate::simulator::sdf::TimingCheck;
use crate::simulator::state::SimulationState;
use crate::simulator::types::*;
use crate::Symbol;
use crate::waveform::{CsvWaveWriter, FstWaveWriter, SignalStats, VcdWriter};
use rand::rngs::StdRng;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;


pub(crate) const MAX_LOOP_ITER: usize = 10_000_000;

/// Tracks a single attempt of a concurrent assertion sequence evaluation
pub struct SequenceAttempt {
    pub sequence: Box<IrSequence>,
    pub cycles: u64,
    pub pass_stmt: Vec<IrStmt>,
    pub fail_stmt: Vec<IrStmt>,
    pub clock_event: crate::ast::types::ClockEvent,
}

/// Main simulation engine — event-driven SystemVerilog simulator.
pub struct SimulationEngine {
    pub design: IrDesign,
    pub state: SimulationState,
    pub max_time: u64,
    pub running: bool,
    pub events: Vec<Vec<RegionEvent>>,
    pub nba_pending: Vec<(IrLValue, LogicVec)>,
    pub vcd: Option<VcdWriter>,
    pub fst: Option<FstWaveWriter>,
    /// CSV waveform writer (signal values as comma-separated values)
    pub csv: Option<CsvWaveWriter>,
    /// Signal statistics collector (toggle counts, transitions)
    pub signal_stats: Option<SignalStats>,
    pub current_this: Option<ObjId>,
    pub method_locals: Vec<HashMap<Symbol, LogicVec>>,
    pub current_method: Option<Symbol>,
    pub rng: StdRng,
    /// Jumlah panggilan fungsi random (untuk $get_randcount).
    pub rand_call_count: u64,
    /// Seed RNG terakhir (untuk $get_randstate).
    pub rand_seed: u64,
    /// Scope aktif untuk $scope / $showscopes.
    pub current_scope_name: Option<String>,
    pub file_handles: HashMap<u32, File>,
    pub file_ungetc_buf: HashMap<u32, Vec<u8>>,
    pub file_read_pos: HashMap<u32, u64>,
    pub next_file_handle: u32,
    pub monitor_args: Option<Vec<IrExpr>>,
    pub monitor_last_values: Option<Vec<LogicVec>>,
    pub disable_pending: Option<Symbol>,
    pub control_flow: Option<FlowControl>,
    pub expr_recursion_depth: usize,
    pub forced_signals: HashSet<SignalId>,
    pub signal_snapshot: Option<Vec<LogicVec>>,
    /// Snapshot coverage di awal time step (SEBELUM loop delta). Tidak di-refresh
    /// per delta — dipakai record_coverage_after_commit untuk diff toggle/FSM
    /// (fix SIM-30: signal_snapshot di-refresh tiap delta untuk edge detection,
    /// membuat diff coverage selalu kosong).
    pub coverage_snapshot: Option<Vec<LogicVec>>,
    pub pending_waits: Vec<(Vec<SignalId>, Vec<IrStmt>)>,
    pub pending_await_target: Option<ObjId>,
    pub pending_wait_orders: Vec<WaitOrderState>,
    pub loop_continuation: Option<Vec<IrStmt>>,
    pub post_loop_tail: Vec<IrStmt>,
    pub current_time: u64,
    pub fork_groups: Vec<ForkGroup>,
    pub reactive_events: Vec<EventKind>,
    pub strobe_events: Vec<Vec<IrExpr>>,
    pub fstrobe_events: Vec<(u32, Vec<IrExpr>)>,
    pub fmonitor_map: HashMap<u32, (Vec<IrExpr>, Vec<LogicVec>)>,
    pub mailbox_queues: HashMap<usize, VecDeque<LogicVec>>,
    pub semaphore_counts: HashMap<usize, u32>,
    pub assoc_data: HashMap<usize, HashMap<LogicVec, LogicVec>>,
    pub uvm_object_data: HashMap<ObjId, UvmObjectData>,
    pub uvm_component_data: HashMap<ObjId, UvmComponentData>,
    pub uvm_sequencer_data: HashMap<ObjId, UvmSequencerData>,
    pub uvm_driver_data: HashMap<ObjId, UvmDriverData>,
    pub uvm_analysis_port_data: HashMap<ObjId, UvmAnalysisPortData>,
    pub uvm_analysis_imp_data: HashMap<ObjId, UvmAnalysisImpData>,
    pub uvm_config_db_data: HashMap<(String, String), LogicVec>,
    pub sdf_timing_checks: Vec<TimingCheck>,
    pub uvm_resource_db_data: HashMap<(String, String), LogicVec>,
    /// UVM register layer data: register objects
    pub uvm_reg_data: std::collections::HashMap<ObjId, UvmRegData>,
    /// UVM register layer data: register field objects
    pub uvm_reg_field_data: std::collections::HashMap<ObjId, UvmRegFieldData>,
    /// UVM register layer data: register block objects
    pub uvm_reg_block_data: std::collections::HashMap<ObjId, UvmRegBlockData>,
    /// UVM register layer data: register map objects
    pub uvm_reg_map_data: std::collections::HashMap<ObjId, UvmRegMapData>,
    /// UVM callback queues: (component_type, cb_type) → registered callbacks
    pub callback_queues: HashMap<(String, String), crate::simulator::types::UvmCallbackData>,
    pub factory_type_overrides: HashMap<String, String>,
    pub root_test_obj_id: Option<ObjId>,
    pub process_map: HashMap<ObjId, ProcessInfo>,
    pub _next_process_id: usize,
    pub current_process_id: Option<ObjId>,
    pub cover_hits: HashMap<Symbol, u64>,
    pub cover_total: HashMap<Symbol, u64>,
    pub cover_bins: HashMap<Symbol, HashMap<Symbol, u64>>,
    pub plusargs: HashMap<String, String>,
    pub debug_mode: DebugMode,
    pub breakpoints: Vec<Breakpoint>,
    pub watchpoints: Vec<Watchpoint>,
    pub signal_history: crate::simulator::signal_history::SignalHistoryStore,
    pub signal_last_change: HashMap<usize, u64>,
    /// Arah transisi terakhir tiap signal (untuk timing check edge-aware, SIM-24).
    pub signal_last_dir: HashMap<usize, crate::ast::types::EdgeKind>,
    /// Waktu perubahan SEBELUM signal_last_change (untuk dedupe width/period, SIM-24).
    pub signal_prev_change: HashMap<usize, u64>,
    /// Dedupe pelaporan timing violation per (check_name, signal_id) → last_change
    /// yang sudah dilaporkan, supaya width/period/recovery tidak spam (SIM-24).
    pub timing_reported: HashMap<(String, usize), u64>,
    pub udp_prev_args: HashMap<Symbol, Vec<LogicVec>>,
    pub parallel_config: ParallelConfig,
    pub sysfunc_prev: HashMap<Symbol, LogicVec>,
    pub sysfunc_history: HashMap<Symbol, Vec<LogicVec>>,
    pub snapshots: Vec<StateSnapshot>,
    pub paused: bool,
    pub step_mode: StepMode,
    pub event_log: Vec<DebugEvent>,
    pub snapshot_interval: u64,
    pub assert_off_all: bool,
    pub assert_kill_all: bool,
    pub assert_modules_off: HashSet<Symbol>,
    pub coverage_options: HashMap<String, String>,
    pub coverage_enabled: bool,
    /// Selectively enable specific coverage types (empty = all enabled)
    pub coverage_enabled_types: HashSet<CoverageType>,
    /// Glitch detection: max pulse width (in time units) for A->B->A detection. 0 = disabled.
    pub glitch_window: u64,
    /// Glitch detection: per-signal (time of last change, value before last change)
    pub glitch_prev: std::collections::HashMap<SignalId, (u64, crate::ir::LogicVec)>,
    pub cover_line: HashMap<Symbol, u64>,
    pub cover_toggle: HashMap<usize, HashSet<(LogicVal, LogicVal)>>,
    pub cover_branches: HashMap<Symbol, HashMap<Symbol, u64>>,
    pub cover_fsm: HashMap<usize, HashSet<u64>>,
    pub cover_branch_counter: u64,
    pub coverage_model_handles: HashMap<usize, Symbol>,
    pub next_coverage_model_handle: usize,
    pub sequence_attempts: Vec<SequenceAttempt>,
    pub recursion_depth: HashMap<Symbol, usize>,
    pub max_recursion_depth: usize,
    pub objection_count: usize,
    pub objection_triggered: bool,
    /// JIT evaluator (native code compilation for fast expression eval)
    pub jit_evaluator: Option<crate::simulator::JITEvaluator>,
    /// Use packed 4-state bitmask eval (SIMD-ready) for bitwise operations
    pub use_packed_eval: bool,
    /// Use expression-level JIT compilation (compiles entire IrExpr tree at once)
    pub use_jit_expression: bool,
    /// Zero-deallocation arena for temporary allocations during simulation
    pub sim_arena: SimulationArena,
    /// DAG-parallel process evaluator (built lazily)
    pub sim_dag: Option<SimulationDag>,
    /// Enable DAG-parallel process evaluation
    pub use_dag_parallel: bool,
    /// Clock domain analysis + fused domains for cycle-based simulation
    pub clock_analysis: Option<ClockDomainAnalysis>,
    /// Enable cycle-based simulation fusion
    pub use_cycle_fusion: bool,
    /// MIR JIT compiler for compiled-code simulation path
    pub mir_jit: Option<crate::mir::MirJitCompiler>,
    /// Enable MIR JIT for combinational process evaluation
    pub use_mir_jit: bool,
    /// Diagnostic collector for structured runtime diagnostics
    pub diag_sink: DiagSink,
    /// Current delta cycle count (for runtime context)
    pub current_delta: u64,
    /// Current process name (for runtime context)
    pub current_process_name: Option<String>,
    /// Current instance path (for runtime context), e.g. "soc.cpu0.fetch"
    pub current_instance_path: Option<String>,
    /// Race detection: tracks which process (ObjId) last wrote each signal in current delta
    pub signal_writers: std::collections::HashMap<SignalId, Option<ObjId>>,
    /// Race detection: write count per signal per time step (for oscillation detection)
    pub signal_write_count: std::collections::HashMap<SignalId, u32>,
    /// Max delta cycles per time step before abort (configurable for testing)
    pub delta_limit: u64,

    /// Co-simulation state (shared with external simulator via TCP)
    pub cosim_state: Option<std::sync::Arc<std::sync::Mutex<crate::simulator::cosim::CosimState>>>,

    /// Co-simulation signal mapping: (signal_id, signal_name, direction)
    pub cosim_signals: Vec<(usize, String, bool)>,

    /// Per-path signal delays from SDF annotation: "cell_name:from->to" → SignalDelay
    pub signal_delays: std::collections::HashMap<String, crate::simulator::state::SignalDelay>,

    /// UPF (Unified Power Format) power intent database for power-aware simulation
    pub power_intent: Option<crate::simulator::upf::PowerIntent>,

    /// Process body cache for DAG parallel evaluation.
    /// Dibangun sekali di run() untuk menghindari clone bodies setiap cycle.
    pub process_body_cache: HashMap<usize, Vec<IrStmt>>,

    /// Hierarchical timing wheel for O(1) event scheduling (replaces Vec<Vec<RegionEvent>>).
    /// When enabled, events are stored in the timing wheel instead of `events: Vec<Vec<RegionEvent>>`.
    pub timing_wheel: Option<crate::simulator::engine::scheduler::timing_wheel::HierarchicalTimingWheel>,

    /// Whether to use the timing wheel for event scheduling.
    pub use_timing_wheel: bool,

    /// Performance monitoring dashboard (SIM-25): metrik simulasi runtime
    /// (delta cycles, events processed, throughput). Dipakai CLI --perf-dashboard.
    pub sim_perf: crate::profiling::PerfDashboard,
}

// ============================================================================
// CATATAN: Fungsi-fungsi helper standalone (evaluate_string_method,
// sym_char_matches, edge_matches_abbrev) sudah dipindahkan ke
// src/simulator/engine/engine_utils.rs untuk memisahkan tanggung jawab.
// ============================================================================
