use crate::ir::*;
use std::fmt;

// ── Debug types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DebugMode {
    Normal,
    Debug,
    DeepDebug,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StepMode {
    Running,
    Paused,
    StepCycle,
}

#[derive(Debug, Clone)]
pub enum Breakpoint {
    Cycle(u64),
    SignalEq(String, LogicVec),
    SignalNeq(String, LogicVec),
    SignalChange(String),
    Module(String),
}

impl fmt::Display for Breakpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Breakpoint::Cycle(c) => write!(f, "break cycle {}", c),
            Breakpoint::SignalEq(n, v) => write!(f, "break signal {} == {}", n, v),
            Breakpoint::SignalNeq(n, v) => write!(f, "break signal {} != {}", n, v),
            Breakpoint::SignalChange(n) => write!(f, "break change {}", n),
            Breakpoint::Module(n) => write!(f, "break module {}", n),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Watchpoint {
    Signal(String),
    MemAddr(u64),
}

impl fmt::Display for Watchpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Watchpoint::Signal(n) => write!(f, "watch {}", n),
            Watchpoint::MemAddr(a) => write!(f, "watch mem[{:#x}]", a),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DebugEvent {
    pub kind: DebugEventKind,
    pub time: u64,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum DebugEventKind {
    BreakpointHit,
    WatchpointHit,
    StepComplete,
    SignalChanged,
}

#[derive(Debug, Clone)]
pub struct StateSnapshot {
    pub time: u64,
    pub signals: Vec<LogicVec>,
    pub next_signals: Vec<LogicVec>,
    pub changed: Vec<bool>,
}

#[derive(Debug, Clone)]
pub enum EventKind {
    EvalProcess(usize),
    ContinueBlock(Continuation),
    ContinueAstBlock(Vec<crate::ast::Stmt>, Option<usize>),
    NbaCommit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventRegion {
    Preponed = 1,
    PreActive = 2,
    Active = 3,
    Inactive = 4,
    PreNba = 5,
    Nba = 6,
    PostNba = 7,
    PreObserved = 8,
    Observed = 9,
    PostObserved = 10,
    Reactive = 11,
    PostReactive = 12,
}

pub(super) const IEEE_REGIONS: [EventRegion; 12] = [
    EventRegion::Preponed,
    EventRegion::PreActive,
    EventRegion::Active,
    EventRegion::Inactive,
    EventRegion::PreNba,
    EventRegion::Nba,
    EventRegion::PostNba,
    EventRegion::PreObserved,
    EventRegion::Observed,
    EventRegion::PostObserved,
    EventRegion::Reactive,
    EventRegion::PostReactive,
];

#[derive(Debug, Clone)]
pub struct RegionEvent {
    pub region: EventRegion,
    pub event: EventKind,
}

#[derive(Debug, Clone)]
pub struct ForkGroup {
    pub(super) remaining: usize,
    pub(super) continuation: Vec<IrStmt>,
}

#[derive(Debug, Clone)]
pub struct Continuation {
    pub stmts_to_exec: Vec<IrStmt>,
    pub stmts_remaining: Vec<IrStmt>,
    pub fork_id: Option<usize>,
    pub process_id: Option<ObjId>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FlowControl {
    Break,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessStatus {
    Finished = 0,
    Running = 1,
    Waiting = 2,
    Suspended = 3,
    Killed = 4,
}

#[derive(Debug, Clone)]
pub struct UvmObjectData {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct UvmComponentData {
    pub parent: Option<ObjId>,
    pub children: Vec<ObjId>,
    pub report_verbosity: u32,
}

#[derive(Debug, Clone)]
pub struct UvmSequencerData {
    pub item_queue: Vec<ObjId>,
    pub current_item: Option<ObjId>,
}

#[derive(Debug, Clone)]
pub struct UvmDriverData {
    pub sequencer_id: Option<ObjId>,
    pub current_item: Option<ObjId>,
}

#[derive(Debug, Clone)]
pub struct UvmAnalysisPortData {
    pub connections: Vec<ObjId>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct UvmAnalysisImpData {
    pub parent: Option<ObjId>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct WaitOrderState {
    pub events: Vec<SignalId>,
    pub expected_idx: usize,
    pub continuation: Vec<IrStmt>,
    pub failure_stmts: Vec<IrStmt>,
}

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub status: ProcessStatus,
    pub await_continuations: Vec<Vec<IrStmt>>,
}
