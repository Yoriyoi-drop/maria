pub mod coverage;
pub mod debug;
pub mod engine_utils;
pub mod waveform;
pub mod core;
pub mod scheduler;
pub mod eval;
pub mod uvm;
pub mod sequence;

use crate::diagnostics::diagnostic::{DiagCode, DiagLevel, Diagnostic, RuntimeContext, SourceSnippet};
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
use crate::waveform::FstWaveWriter;
use crate::waveform::VcdWriter;
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
    pub current_this: Option<ObjId>,
    pub method_locals: Vec<HashMap<Symbol, LogicVec>>,
    pub current_method: Option<Symbol>,
    pub rng: StdRng,
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
    pub signal_history: HashMap<Symbol, std::collections::VecDeque<(u64, LogicVec)>>,
    pub signal_history_max: usize,
    pub signal_last_change: HashMap<usize, u64>,
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
    /// Diagnostic collector for structured runtime diagnostics
    pub diag_sink: DiagSink,
    /// Current delta cycle count (for runtime context)
    pub current_delta: u64,
    /// Current process name (for runtime context)
    pub current_process_name: Option<String>,
    /// Current instance path (for runtime context), e.g. "soc.cpu0.fetch"
    pub current_instance_path: Option<String>,
}

// ============================================================================
// CATATAN: Fungsi-fungsi helper standalone (evaluate_string_method,
// sym_char_matches, edge_matches_abbrev) sudah dipindahkan ke
// src/simulator/engine/engine_utils.rs untuk memisahkan tanggung jawab.
// ============================================================================
