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
use crate::simulator::sdf::SdfData;
use crate::simulator::state::SimulationState;
use crate::simulator::types::*;
use crate::waveform::FstWaveWriter;
use crate::waveform::VcdWriter;
use rand::Rng;
use rand::SeedableRng;
use std::collections::{HashMap, VecDeque};

pub(crate) const MAX_LOOP_ITER: usize = 10_000_000;

/// Tracks a single attempt of a concurrent assertion sequence evaluation
pub struct SequenceAttempt {
    /// The sequence expression being evaluated
    pub sequence: Box<IrSequence>,
    /// Clock cycles elapsed since this attempt started
    pub cycles: u64,
    /// Pass statements to execute on success
    pub pass_stmt: Vec<IrStmt>,
    /// Fail statements to execute on failure
    pub fail_stmt: Vec<IrStmt>,
    /// Clock event for this assertion
    pub clock_event: crate::ast::types::ClockEvent,
}

/// Main simulation engine — event-driven SystemVerilog simulator.
pub struct SimulationEngine {
    pub design: IrDesign,
    pub state: SimulationState,
    pub vcd: Option<VcdWriter>,
    pub fst: Option<FstWaveWriter>,
    pub parallel_config: Option<ParallelConfig>,
    pub sdf: Option<SdfData>,
    pub event_queue: VecDeque<(usize, EventKind)>,
    pub nba_pending: HashMap<SignalId, LogicVec>,
    pub method_locals: Vec<HashMap<Symbol, LogicVec>>,
    pub current_this: Option<ObjId>,
    pub control_flow: Option<FlowControl>,
    pub rng: Rng,
    pub max_time: u64,
    pub current_time: u64,
    pub debug_mode: DebugMode,
    pub debugger: Option<DebuggerState>,
    pub signal_history: HashMap<String, Vec<(u64, LogicVec)>>,
    pub coverage_data: HashMap<String, CoverageInfo>,
    pub thread_pool: Option<rayon::ThreadPool>,
    pub sequence_attempts: Vec<SequenceAttempt>,
    pub active_sequences: HashMap<usize, Vec<IrLValue>>,
    pub seq_counter: usize,
    pub signal_snapshot: Option<HashMap<SignalId, LogicVec>>,
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
