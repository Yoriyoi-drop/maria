use crate::ir::*;
use crate::Symbol;
use std::collections::HashMap;
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
    Postponed = 13,
}

pub(super) const IEEE_REGIONS: [EventRegion; 13] = [
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
    EventRegion::Postponed,
];

#[derive(Debug, Clone)]
pub struct RegionEvent {
    pub region: EventRegion,
    pub event: EventKind,
}

/// Blocking event control `@(sig)` / `@(posedge sig)` yang sedang menunggu
/// perubahan/edge pada suatu signal. Continuation di-resume saat signal berubah
/// (level) atau edge yang sesuai terdeteksi (via snapshot delta).
/// Blocking event control `@(sig)` / `@(posedge sig)` yang sedang menunggu
/// perubahan/edge pada satu atau lebih signal. SATU entry mewakili SATU `@(...)`:
/// fire sekali saat sinyal mana pun berubah, cegah double-fire / event stale.
#[derive(Debug, Clone)]
pub struct PendingEventControl {
    /// Semua (signal, edge) dari satu `@(a or b)` — None edge = level.
    pub sigs: Vec<(SignalId, Option<ClockEdge>)>,
    /// Nilai tiap sinyal saat event di-arm (paralel dgn `sigs`). Event hanya fire
    /// jika nilainya BERUBAH setelah arm — mencegah re-fire pada time step yang
    /// sama untuk perubahan yang sama (semantik `@(sig)` yang benar).
    pub armed_vals: Vec<LogicVec>,
    /// Statement yang dilanjutkan setelah event terpenuhi (body + sisa + loop_cont).
    pub continuation: Vec<IrStmt>,
}

/// Blocking event control `@(...)` di jalur AST (class method/task UVM).
/// Menyimpan konteks method (this/locals/method) agar continuation bisa
/// di-resume dengan benar (resume AST task kehilangan konteks tanpa ini).
#[derive(Debug, Clone)]
pub struct PendingAstEventControl {
    /// Semua (signal, edge) dari satu `@(a or b)` — SATU entry, cegah double-fire.
    pub sigs: Vec<(SignalId, Option<ClockEdge>)>,
    /// Nilai tiap sinyal saat arm — fire hanya jika nilai berubah setelah arm.
    pub armed_vals: Vec<LogicVec>,
    pub continuation: Vec<crate::ast::Stmt>,
    pub this: Option<ObjId>,
    pub method: Option<Symbol>,
    /// Snapshot lengkap method_locals saat suspensi.
    pub locals: Vec<HashMap<Symbol, LogicVec>>,
    /// Jumlah frame locals saat suspensi — dipakai truncate saat continuation selesai.
    pub base_len: usize,
}

#[derive(Debug, Clone)]
pub struct ForkGroup {
    pub(super) remaining: usize,
    pub(super) continuation: Vec<IrStmt>,
    /// Continuation sudah dijadwalkan/dieksekusi — cegah evaluasi ganda.
    pub(super) fired: bool,
    /// Group masih dipakai (belum di-retire ke `fork_free`).
    pub(super) active: bool,
    /// true = Join/JoinNone (slot aman di-reuse saat semua branch selesai).
    /// false = JoinAny (branch yang dibuang masih mereferensi fid → jangan reuse).
    pub(super) reclaimable: bool,
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

/// UVM callback data: registered callback objects for a callback type.
#[derive(Debug, Clone)]
pub struct UvmCallbackData {
    /// Callback type name (e.g., "my_callbacks")
    pub cb_type_name: String,
    /// Registered callback objects (ObjId → callback instance name)
    pub callbacks: Vec<(ObjId, String)>,
    /// Whether the callback is enabled
    pub enabled: bool,
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

/// UVM register field data: bit position within a register, width, access policy.
#[derive(Debug, Clone)]
pub struct UvmRegFieldData {
    /// Parent register object ID
    pub parent_reg: Option<ObjId>,
    /// Bit position (LSB) within the register
    pub bit_pos: usize,
    /// Width in bits
    pub width: usize,
    /// Current value (mirrored)
    pub value: LogicVec,
    /// Desired value (next value to be written)
    pub desired: LogicVec,
    /// Access policy: "RO", "WO", "RW", "RC", "RS", "WRC", "WRS", "WC", "WS", "W1C", "W1S", etc.
    pub access: String,
    /// Whether the field has been modified since last update
    pub modified: bool,
    /// Whether the field is volatile
    pub volatile: bool,
}

impl UvmRegFieldData {
    pub fn new() -> Self {
        UvmRegFieldData {
            parent_reg: None,
            bit_pos: 0,
            width: 1,
            value: LogicVec::new(1),
            desired: LogicVec::new(1),
            access: "RW".to_string(),
            modified: false,
            volatile: false,
        }
    }
}

/// UVM register data: a named register with address, width, and fields.
#[derive(Debug, Clone)]
pub struct UvmRegData {
    /// Register address in the address map
    pub address: u64,
    /// Total width in bits (sum of field widths)
    pub width: usize,
    /// Current mirrored value
    pub value: LogicVec,
    /// Desired value (pending write)
    pub desired: LogicVec,
    /// List of field object IDs belonging to this register
    pub fields: Vec<ObjId>,
    /// Whether register has been modified since last update
    pub modified: bool,
    /// Parent block data
    pub parent_block: Option<ObjId>,
}

impl UvmRegData {
    pub fn new() -> Self {
        UvmRegData {
            address: 0,
            width: 32,
            value: LogicVec::new(32),
            desired: LogicVec::new(32),
            fields: Vec::new(),
            modified: false,
            parent_block: None,
        }
    }
}

/// UVM register block data: a block containing registers with an address map.
#[derive(Debug, Clone)]
pub struct UvmRegBlockData {
    /// Address map: offset -> register object ID
    pub regs_by_offset: std::collections::HashMap<u64, ObjId>,
    /// Default register map object ID (uvm_reg_map instance)
    pub default_map: Option<ObjId>,
    /// Base address of the block
    pub base_address: u64,
}

impl UvmRegBlockData {
    pub fn new() -> Self {
        UvmRegBlockData {
            regs_by_offset: std::collections::HashMap::new(),
            default_map: None,
            base_address: 0,
        }
    }
}

/// UVM register map data: address decoding for a set of registers.
#[derive(Debug, Clone)]
pub struct UvmRegMapData {
    /// Address map: offset -> register object ID
    pub regs_by_offset: std::collections::HashMap<u64, ObjId>,
    /// Base address
    pub base_address: u64,
    /// Address width in bits
    pub n_bits: usize,
}

impl UvmRegMapData {
    pub fn new() -> Self {
        UvmRegMapData {
            regs_by_offset: std::collections::HashMap::new(),
            base_address: 0,
            n_bits: 32,
        }
    }
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

/// Coverage types that can be selectively enabled
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoverageType {
    Line,
    Toggle,
    Branch,
    Fsm,
    Covergroup,
}

/// X-propagation mode for controlling how X (unknown) values propagate
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum XPropagationMode {
    /// X is masked by deterministic values: 0 & X = 0, 1 | X = 1, X == 5 = X
    Optimistic,
    /// Any X input produces X output: 0 & X = X, 1 | X = X
    Pessimistic,
    /// X treated as potentially any value — strictest checking
    XAnywhere,
}

impl XPropagationMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            XPropagationMode::Optimistic => "optimistic",
            XPropagationMode::Pessimistic => "pessimistic",
            XPropagationMode::XAnywhere => "x-anywhere",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "optimistic" | "opt" => Some(XPropagationMode::Optimistic),
            "pessimistic" | "pess" => Some(XPropagationMode::Pessimistic),
            "x-anywhere" | "anywhere" | "xany" => Some(XPropagationMode::XAnywhere),
            _ => None,
        }
    }
}

/// Konfigurasi format waktu untuk `%t` (set oleh `$timeformat`).
/// units = eksponen unit (mis. -9 = ns, -12 = ps), sesuai IEEE 1800.
#[derive(Debug, Clone)]
pub struct TimeFormat {
    pub units: i64,
    pub precision: i64,
    pub suffix: String,
    pub min_field_width: usize,
    /// Eksponen unit basis sim-time (mis. -9 = ns). Diderivasi dari
    /// `` `timescale `` desain saat engine init; dipakai untuk skala %t.
    pub base_units: i64,
}

impl TimeFormat {
    /// Parse eksponen unit dari string seperti "1ns", "10ps", "100us", "1ms".
    /// Mengembalikan None jika tidak dikenal (caller fallback ke -9 = ns).
    pub fn unit_exponent(unit: &str) -> Option<i64> {
        let unit = unit.trim();
        if unit.ends_with("fs") {
            Some(-15)
        } else if unit.ends_with("ps") {
            Some(-12)
        } else if unit.ends_with("ns") {
            Some(-9)
        } else if unit.ends_with("us") {
            Some(-6)
        } else if unit.ends_with("ms") {
            Some(-3)
        } else if unit.ends_with('s') {
            Some(0)
        } else {
            None
        }
    }
}

impl Default for TimeFormat {
    fn default() -> Self {
        TimeFormat {
            units: -9,
            precision: 0,
            suffix: String::new(),
            min_field_width: 0,
            base_units: -9,
        }
    }
}
