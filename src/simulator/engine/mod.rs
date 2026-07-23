pub mod coverage;
pub mod debug;
pub mod waveform;
pub mod core;
pub mod scheduler;
pub mod eval;
pub mod uvm;
pub mod sequence;

use crate::error::SimError;
use crate::ir::*;
use crate::simulator::parallel::ParallelConfig;

use crate::simulator::sdf::TimingCheck;
use crate::simulator::state::SimulationState;
use crate::simulator::types::*;
use crate::simulator::util::logicvec_to_string;
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
    pub forced_signals: HashSet<SignalId>,
    pub signal_snapshot: Option<Vec<LogicVec>>,
    pub pending_waits: Vec<(Vec<SignalId>, Vec<IrStmt>)>,
    pub pending_await_target: Option<ObjId>,
    pub pending_wait_orders: Vec<WaitOrderState>,
    pub loop_continuation: Option<Vec<IrStmt>>,
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
    pub uvm_config_db_data: HashMap<(Symbol, Symbol), LogicVec>,
    pub sdf_timing_checks: Vec<TimingCheck>,
    pub uvm_resource_db_data: HashMap<(Symbol, Symbol), LogicVec>,
    pub factory_type_overrides: HashMap<Symbol, Symbol>,
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
    pub signal_history: HashMap<Symbol, Vec<(u64, LogicVec)>>,
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
}

// ─── Standalone helper functions ───

pub(crate) fn evaluate_string_method(s: &str, method: &str, args: &[LogicVec]) -> Result<LogicVec, SimError> {
    match method {
        "len" => Ok(LogicVec::from_u64(s.len() as u64, 32)),
        "substr" => {
            if args.len() != 2 {
                return Err(SimError::runtime(format!(
                    "substr expects 2 arguments, got {}",
                    args.len()
                )));
            }
            let i = args[0].to_u64() as usize;
            let j = args[1].to_u64() as usize;
            if i > j || j >= s.len() {
                return Err(SimError::runtime(format!(
                    "substr({}, {}) out of range for string of len {}",
                    i, j, s.len()
                )));
            }
            let sub = &s[i..=j];
            let mut bits = Vec::with_capacity(sub.len() * 8);
            for c in sub.chars() {
                let byte = c as u8;
                for b in 0..8 {
                    bits.push(if (byte >> b) & 1 == 1 {
                        LogicVal::One
                    } else {
                        LogicVal::Zero
                    });
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
                return Err(SimError::runtime(format!("{} expects 1 argument", method)));
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
        _ => Err(SimError::runtime(format!("unknown string method: {}", method))),
    }
}

pub(crate) fn sym_char_matches(c: char, val: LogicVal) -> bool {
    match c {
        '0' => val == LogicVal::Zero,
        '1' => val == LogicVal::One,
        'x' | 'X' => val == LogicVal::X,
        '?' => true,
        'b' | 'B' => val == LogicVal::Zero || val == LogicVal::One,
        _ => false,
    }
}

pub(crate) fn edge_matches_abbrev(edge: &str, prev: LogicVal, curr: LogicVal) -> bool {
    match edge {
        "r" | "R" => prev == LogicVal::Zero && curr == LogicVal::One,
        "f" | "F" => prev == LogicVal::One && curr == LogicVal::Zero,
        "p" | "P" => {
            (prev == LogicVal::Zero || prev == LogicVal::X || prev == LogicVal::Z)
                && curr == LogicVal::One
        }
        "n" | "N" => {
            (prev == LogicVal::One || prev == LogicVal::X || prev == LogicVal::Z)
                && curr == LogicVal::Zero
        }
        "*" => prev != curr,
        _ => false,
    }
}
