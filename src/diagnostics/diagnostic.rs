//! Diagnostic — structured error/warning reporting ala Cargo/Rust.
//!
//! Setiap diagnostic memiliki: level, code, message, spans, notes, hints,
//! explanation, suggestion, runtime context, dan source snippet.
//! Thread-safe via MPSC channel (DiagSink).
//!
//! Format keluaran:
//!
//! ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//!
//! error[RT0001]: Null handle access
//!
//! ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//!
//!    ┌─ uart.sv:52:14
//!    │
//! 52 │ fifo.push(data);
//!    │      ^^^^ Handle "fifo" is null
//!    │
//!    = module : uart_top
//!    = process: write_task
//!    = time   : 125 ns
//!    = thread : worker-2
//!
//! Explanation
//!
//! The FIFO handle was never initialized before simulation started.
//!
//! Help
//!
//! Initialize the interface before using it.

use std::borrow::Cow;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::intern::Symbol;

// ─── Severity ───

/// Severity level untuk diagnostic HDL — fatal, error, warning, note, dll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagLevel {
    /// Fatal — simulasi harus dihentikan segera.
    Fatal,
    /// Internal compiler error — bug di compiler itu sendiri.
    Bug,
    /// Pasti salah — harus diperbaiki user.
    Error,
    /// Mencurigakan tapi valid — user harus perhatikan.
    Warning,
    /// Informasi tambahan.
    Note,
    /// Informasi umum.
    Info,
    /// Saran perbaikan.
    Help,
    /// Informasi trace untuk debugging.
    Trace,
    /// Informasi debug yang sangat detail.
    Debug,
}

impl DiagLevel {
    /// Kembalikan severity label pendek.
    pub fn as_str(&self) -> &'static str {
        match self {
            DiagLevel::Fatal => "fatal",
            DiagLevel::Bug => "bug",
            DiagLevel::Error => "error",
            DiagLevel::Warning => "warning",
            DiagLevel::Note => "note",
            DiagLevel::Info => "info",
            DiagLevel::Help => "help",
            DiagLevel::Trace => "trace",
            DiagLevel::Debug => "debug",
        }
    }

    /// Apakah level ini menghentikan eksekusi?
    pub fn is_fatal(&self) -> bool {
        matches!(self, DiagLevel::Fatal | DiagLevel::Bug)
    }

    /// Apakah level ini dianggap error?
    pub fn is_error(&self) -> bool {
        matches!(self, DiagLevel::Fatal | DiagLevel::Bug | DiagLevel::Error)
    }
}

impl fmt::Display for DiagLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ─── Diagnostic Code ───

/// Error code untuk diagnostic.
///
/// Kategori kode:
/// - E1xxx: Parse errors
/// - E2xxx: Semantic errors
/// - E3xxx: Elaboration errors
/// - RT0xxx: Memory runtime errors
/// - RT1xxx: Signal runtime errors
/// - RT2xxx: Scheduler runtime errors
/// - RT3xxx: Event runtime errors
/// - RT4xxx: Module runtime errors
/// - RT5xxx: Interface runtime errors
/// - RT6xxx: Clock runtime errors
/// - RT7xxx: Assertion runtime errors
/// - RT8xxx: DPI runtime errors
/// - RT9xxx: Internal runtime errors
/// - WRxxxx: Warnings
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagCode {
    // ── Parse errors: E1xxx ──
    UnexpectedToken,
    ExpectedToken,
    ExpectedSemi,
    UnclosedBlock,
    InvalidSyntax,

    // ── Semantic errors: E2xxx ──
    UndefinedSignal,
    TypeMismatch,
    WidthMismatch,
    UndefinedVariable,

    // ── Elaboration errors: E3xxx ──
    ModuleNotFound,
    CircularDependency,
    ParamMismatch,
    InstanceNotFound,

    // ── Runtime Memory: RT0xxx ──
    /// Null handle access (RT0001)
    NullHandle,
    /// Invalid object reference (RT0002)
    InvalidReference,
    /// Null interface dereference (RT0003)
    NullInterface,
    /// Out of bounds memory access (RT0004)
    MemoryOutOfBounds,
    /// Mailbox access error (RT0005)
    MailboxError,

    // ── Runtime Signal: RT1xxx ──
    /// Signal entered unknown state (X) (RT1001)
    SignalUnknown,
    /// Floating signal detected (RT1002)
    SignalFloating,
    /// Driver contention (RT1003)
    SignalContention,
    /// Signal width mismatch at runtime (RT1004)
    SignalWidthMismatch,
    /// Uninitialized signal read (RT1005)
    SignalUninitialized,

    // ── Runtime Scheduler: RT2xxx ──
    /// Infinite delta cycle (RT2001)
    InfiniteDelta,
    /// Scheduler deadlock (RT2002)
    SchedulerDeadlock,
    /// Simulation timeout (RT2003)
    SimulationTimeout,
    /// Process fork error (RT2004)
    ForkError,
    /// Max delta cycles exceeded (RT2005)
    MaxDeltaExceeded,

    // ── Runtime Event: RT3xxx ──
    /// Event wait timeout (RT3001)
    EventTimeout,
    /// Event order violation (RT3002)
    EventOrderViolation,
    /// Event creation failed (RT3003)
    EventCreateFailed,

    // ── Runtime Module: RT4xxx ──
    /// Module instantiation error (RT4001)
    ModuleInstantiation,
    /// Module binding error (RT4002)
    ModuleBindError,

    // ── Runtime Interface: RT5xxx ──
    /// Null interface handle (RT5001)
    InterfaceNull,
    /// Interface connection error (RT5002)
    InterfaceConnect,

    // ── Runtime Clock: RT6xxx ──
    /// Clock period violation (RT6001)
    ClockPeriodViolation,
    /// Clock generation error (RT6002)
    ClockGenError,

    // ── Runtime Assertion: RT7xxx ──
    /// Concurrent assertion failed (RT7001)
    AssertionFailed,
    /// Immediate assertion failed (RT7002)
    AssertionImmediateFailed,
    /// Cover property hit count (RT7003)
    CoverProperty,
    /// Assertion disable error (RT7004)
    AssertionDisableError,

    // ── Runtime DPI: RT8xxx ──
    /// DPI function error (RT8001)
    DpiError,
    /// DPI import not found (RT8002)
    DpiImportNotFound,
    /// DPI scope error (RT8003)
    DpiScopeError,

    // ── Runtime Internal: RT9xxx ──
    /// Internal simulator error (RT9001)
    InternalError,
    /// Unreachable code reached (RT9002)
    Unreachable,
    /// Not yet implemented (RT9003)
    NotImplemented,

    // ── Legacy Runtime (E9xxx) ──
    /// General simulation error (E9001)
    SimulationError,
    /// Out of bounds (E9002)
    OutOfBounds,
    /// Type mismatch at runtime (E9003)
    RuntimeTypeMismatch,

    // ── Other Codes ──
    /// Waveform error (W0001)
    WaveformError,

    // ── Warnings: WRxxxx ──
    /// Uninitialized register (WR0014)
    UninitializedRegister,
    /// Width mismatch warning (WR0102)
    WidthMismatchWarning,
    /// Unused signal (WR0104)
    UnusedSignal,
    /// Clock never toggles (WR0202)
    ClockNeverToggles,
    /// Reset permanently asserted (WR0203)
    ResetPermanentlyAsserted,
    /// Possible combinational loop (WR0301)
    CombinationalLoop,
    /// Slow simulation region (WR0402)
    SlowSimulation,
}

impl DiagCode {
    /// Kembalikan string kode error (misal "RT0001", "WR0102").
    pub fn as_str(&self) -> &'static str {
        match self {
            // Parse
            DiagCode::UnexpectedToken => "E1001",
            DiagCode::ExpectedToken => "E1002",
            DiagCode::ExpectedSemi => "E1003",
            DiagCode::UnclosedBlock => "E1004",
            DiagCode::InvalidSyntax => "E1005",
            // Semantic
            DiagCode::UndefinedSignal => "E2001",
            DiagCode::TypeMismatch => "E2002",
            DiagCode::WidthMismatch => "E2003",
            DiagCode::UndefinedVariable => "E2004",
            // Elaboration
            DiagCode::ModuleNotFound => "E3001",
            DiagCode::CircularDependency => "E3002",
            DiagCode::ParamMismatch => "E3003",
            DiagCode::InstanceNotFound => "E3004",
            // Runtime Memory
            DiagCode::NullHandle => "RT0001",
            DiagCode::InvalidReference => "RT0002",
            DiagCode::NullInterface => "RT0003",
            DiagCode::MemoryOutOfBounds => "RT0004",
            DiagCode::MailboxError => "RT0005",
            // Runtime Signal
            DiagCode::SignalUnknown => "RT1001",
            DiagCode::SignalFloating => "RT1002",
            DiagCode::SignalContention => "RT1003",
            DiagCode::SignalWidthMismatch => "RT1004",
            DiagCode::SignalUninitialized => "RT1005",
            // Runtime Scheduler
            DiagCode::InfiniteDelta => "RT2001",
            DiagCode::SchedulerDeadlock => "RT2002",
            DiagCode::SimulationTimeout => "RT2003",
            DiagCode::ForkError => "RT2004",
            DiagCode::MaxDeltaExceeded => "RT2005",
            // Runtime Event
            DiagCode::EventTimeout => "RT3001",
            DiagCode::EventOrderViolation => "RT3002",
            DiagCode::EventCreateFailed => "RT3003",
            // Runtime Module
            DiagCode::ModuleInstantiation => "RT4001",
            DiagCode::ModuleBindError => "RT4002",
            // Runtime Interface
            DiagCode::InterfaceNull => "RT5001",
            DiagCode::InterfaceConnect => "RT5002",
            // Runtime Clock
            DiagCode::ClockPeriodViolation => "RT6001",
            DiagCode::ClockGenError => "RT6002",
            // Runtime Assertion
            DiagCode::AssertionFailed => "RT7001",
            DiagCode::AssertionImmediateFailed => "RT7002",
            DiagCode::CoverProperty => "RT7003",
            DiagCode::AssertionDisableError => "RT7004",
            // Runtime DPI
            DiagCode::DpiError => "RT8001",
            DiagCode::DpiImportNotFound => "RT8002",
            DiagCode::DpiScopeError => "RT8003",
            // Runtime Internal
            DiagCode::InternalError => "RT9001",
            DiagCode::Unreachable => "RT9002",
            DiagCode::NotImplemented => "RT9003",
            // Other
            DiagCode::WaveformError => "W0001",
            // Legacy Runtime
            DiagCode::SimulationError => "E9001",
            DiagCode::OutOfBounds => "E9002",
            DiagCode::RuntimeTypeMismatch => "E9003",
            // Warnings
            DiagCode::UninitializedRegister => "WR0014",
            DiagCode::WidthMismatchWarning => "WR0102",
            DiagCode::UnusedSignal => "WR0104",
            DiagCode::ClockNeverToggles => "WR0202",
            DiagCode::ResetPermanentlyAsserted => "WR0203",
            DiagCode::CombinationalLoop => "WR0301",
            DiagCode::SlowSimulation => "WR0402",
        }
    }

    /// Kembalikan deskripsi singkat kode error.
    pub fn description(&self) -> &'static str {
        match self {
            // Parse
            DiagCode::UnexpectedToken => "unexpected token",
            DiagCode::ExpectedToken => "expected token",
            DiagCode::ExpectedSemi => "expected ';'",
            DiagCode::UnclosedBlock => "unclosed block",
            DiagCode::InvalidSyntax => "invalid syntax",
            // Semantic
            DiagCode::UndefinedSignal => "undefined signal",
            DiagCode::TypeMismatch => "type mismatch",
            DiagCode::WidthMismatch => "width mismatch",
            DiagCode::UndefinedVariable => "undefined variable",
            // Elaboration
            DiagCode::ModuleNotFound => "module not found",
            DiagCode::CircularDependency => "circular dependency",
            DiagCode::ParamMismatch => "parameter mismatch",
            DiagCode::InstanceNotFound => "instance not found",
            // Runtime Memory
            DiagCode::NullHandle => "null handle access",
            DiagCode::InvalidReference => "invalid object reference",
            DiagCode::NullInterface => "null interface dereference",
            DiagCode::MemoryOutOfBounds => "memory out of bounds",
            DiagCode::MailboxError => "mailbox access error",
            // Runtime Signal
            DiagCode::SignalUnknown => "signal entered unknown state (X)",
            DiagCode::SignalFloating => "floating signal detected",
            DiagCode::SignalContention => "driver contention",
            DiagCode::SignalWidthMismatch => "signal width mismatch",
            DiagCode::SignalUninitialized => "uninitialized signal read",
            // Runtime Scheduler
            DiagCode::InfiniteDelta => "infinite delta cycle",
            DiagCode::SchedulerDeadlock => "scheduler deadlock",
            DiagCode::SimulationTimeout => "simulation timeout",
            DiagCode::ForkError => "process fork error",
            DiagCode::MaxDeltaExceeded => "max delta cycles exceeded",
            // Runtime Event
            DiagCode::EventTimeout => "event wait timeout",
            DiagCode::EventOrderViolation => "event order violation",
            DiagCode::EventCreateFailed => "event creation failed",
            // Runtime Module
            DiagCode::ModuleInstantiation => "module instantiation error",
            DiagCode::ModuleBindError => "module binding error",
            // Runtime Interface
            DiagCode::InterfaceNull => "null interface handle",
            DiagCode::InterfaceConnect => "interface connection error",
            // Runtime Clock
            DiagCode::ClockPeriodViolation => "clock period violation",
            DiagCode::ClockGenError => "clock generation error",
            // Runtime Assertion
            DiagCode::AssertionFailed => "assertion failed",
            DiagCode::AssertionImmediateFailed => "immediate assertion failed",
            DiagCode::CoverProperty => "cover property",
            DiagCode::AssertionDisableError => "assertion disable error",
            // Runtime DPI
            DiagCode::DpiError => "DPI function error",
            DiagCode::DpiImportNotFound => "DPI import not found",
            DiagCode::DpiScopeError => "DPI scope error",
            // Runtime Internal
            DiagCode::InternalError => "internal simulator error",
            DiagCode::Unreachable => "unreachable code reached",
            DiagCode::NotImplemented => "not yet implemented",
            // Other
            DiagCode::WaveformError => "waveform error",
            // Legacy Runtime
            DiagCode::SimulationError => "simulation error",
            DiagCode::OutOfBounds => "out of bounds",
            DiagCode::RuntimeTypeMismatch => "runtime type mismatch",
            // Warnings
            DiagCode::UninitializedRegister => "uninitialized register",
            DiagCode::WidthMismatchWarning => "width mismatch",
            DiagCode::UnusedSignal => "unused signal",
            DiagCode::ClockNeverToggles => "clock never toggles",
            DiagCode::ResetPermanentlyAsserted => "reset permanently asserted",
            DiagCode::CombinationalLoop => "possible combinational loop",
            DiagCode::SlowSimulation => "slow simulation region",
        }
    }

    /// Dapatkan penjelasan panjang tentang penyebab error.
    pub fn explanation(&self) -> &'static str {
        match self {
            DiagCode::UndefinedSignal =>
                "A signal name was referenced but has not been declared in the current scope.",
            DiagCode::ModuleNotFound =>
                "A module, interface, or package name could not be found in the design hierarchy.",
            DiagCode::ParamMismatch =>
                "A parameter value does not match its declaration type or range constraints.",
            DiagCode::TypeMismatch =>
                "An expression's type does not match the expected type for this context.",
            DiagCode::WidthMismatch =>
                "The bit width of an expression does not match the expected width.",
            DiagCode::NullHandle =>
                "An object handle was used (method call or member access) but the handle is null.",
            DiagCode::NullInterface =>
                "A virtual interface was used before being connected to a physical interface.",
            DiagCode::MemoryOutOfBounds =>
                "A memory access attempted to read or write beyond the declared array bounds.",
            DiagCode::NotImplemented =>
                "A SystemVerilog feature used in the design is not yet implemented.",
            _ => self.description(),
        }
    }

    /// Dapatkan saran perbaikan untuk error ini.
    pub fn help(&self) -> &'static str {
        match self {
            DiagCode::UndefinedSignal =>
                "Declare the signal before using it, or check for typos in the signal name.",
            DiagCode::ModuleNotFound =>
                "Check that the module/package name is spelled correctly and that all source files are included.",
            DiagCode::ParamMismatch =>
                "Verify parameter values match the declared types and ranges in the module definition.",
            DiagCode::TypeMismatch =>
                "Use a type conversion or change the expression type to match the expected type.",
            DiagCode::NullHandle =>
                "Initialize the object handle with 'new()' before accessing its members.",
            DiagCode::NullInterface =>
                "Connect the virtual interface to a physical interface instance before simulation.",
            DiagCode::NotImplemented =>
                "Consider using an alternative coding style or wait for the feature to be implemented.",
            _ => "Review the code at the indicated location and fix the reported issue.",
        }
    }

    /// Dapatkan kategori kode error.
    pub fn category(&self) -> &'static str {
        match self {
            DiagCode::UnexpectedToken
            | DiagCode::ExpectedToken
            | DiagCode::ExpectedSemi
            | DiagCode::UnclosedBlock
            | DiagCode::InvalidSyntax => "Parse",
            DiagCode::UndefinedSignal
            | DiagCode::TypeMismatch
            | DiagCode::WidthMismatch
            | DiagCode::UndefinedVariable => "Semantic",
            DiagCode::ModuleNotFound
            | DiagCode::CircularDependency
            | DiagCode::ParamMismatch
            | DiagCode::InstanceNotFound => "Elaboration",
            DiagCode::NullHandle
            | DiagCode::InvalidReference
            | DiagCode::NullInterface
            | DiagCode::MemoryOutOfBounds
            | DiagCode::MailboxError => "Memory",
            DiagCode::SignalUnknown
            | DiagCode::SignalFloating
            | DiagCode::SignalContention
            | DiagCode::SignalWidthMismatch
            | DiagCode::SignalUninitialized => "Signal",
            DiagCode::InfiniteDelta
            | DiagCode::SchedulerDeadlock
            | DiagCode::SimulationTimeout
            | DiagCode::ForkError
            | DiagCode::MaxDeltaExceeded => "Scheduler",
            DiagCode::EventTimeout
            | DiagCode::EventOrderViolation
            | DiagCode::EventCreateFailed => "Event",
            DiagCode::ModuleInstantiation | DiagCode::ModuleBindError => "Module",
            DiagCode::InterfaceNull | DiagCode::InterfaceConnect => "Interface",
            DiagCode::ClockPeriodViolation | DiagCode::ClockGenError => "Clock",
            DiagCode::AssertionFailed
            | DiagCode::AssertionImmediateFailed
            | DiagCode::CoverProperty
            | DiagCode::AssertionDisableError => "Assertion",
            DiagCode::DpiError
            | DiagCode::DpiImportNotFound
            | DiagCode::DpiScopeError => "DPI",
            DiagCode::InternalError
            | DiagCode::Unreachable
            | DiagCode::NotImplemented => "Internal",
            DiagCode::WaveformError => "Waveform",
            DiagCode::SimulationError
            | DiagCode::OutOfBounds
            | DiagCode::RuntimeTypeMismatch => "Runtime",
            DiagCode::UninitializedRegister
            | DiagCode::WidthMismatchWarning
            | DiagCode::UnusedSignal
            | DiagCode::ClockNeverToggles
            | DiagCode::ResetPermanentlyAsserted
            | DiagCode::CombinationalLoop
            | DiagCode::SlowSimulation => "Warning",
        }
    }
}

impl fmt::Display for DiagCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ─── Diagnostic Span ───

/// Source location untuk diagnostic.
#[derive(Debug, Clone)]
pub struct DiagSpan {
    pub file: Symbol,
    pub start: u32,
    pub end: u32,
    pub label: Option<Cow<'static, str>>,
}

impl DiagSpan {
    pub fn new(file: Symbol, start: u32, end: u32) -> Self {
        DiagSpan {
            file,
            start,
            end,
            label: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<Cow<'static, str>>) -> Self {
        self.label = Some(label.into());
        self
    }
}

// ─── Diagnostic Note ───

/// Additional note yang dilampirkan ke diagnostic.
#[derive(Debug, Clone)]
pub struct DiagNote {
    pub message: Cow<'static, str>,
    pub span: Option<DiagSpan>,
}

impl DiagNote {
    pub fn new(message: impl Into<Cow<'static, str>>) -> Self {
        DiagNote {
            message: message.into(),
            span: None,
        }
    }

    pub fn with_span(mut self, span: DiagSpan) -> Self {
        self.span = Some(span);
        self
    }
}

// ─── Runtime Context ───

/// Konteks simulasi saat runtime error terjadi.
///
/// Ini adalah informasi yang tidak dimiliki compiler biasa — spesifik untuk HDL simulation.
/// Memberi insinyur gambaran lengkap: kapan, di mana, dan dalam konteks apa error terjadi.
#[derive(Debug, Clone, Default)]
pub struct RuntimeContext {
    /// Simulation time (misal 125 ns)
    pub time: Option<String>,
    /// Delta cycle
    pub delta: Option<u64>,
    /// Worker thread
    pub thread: Option<String>,
    /// Module name
    pub module: Option<String>,
    /// Hierarchical instance path
    pub instance: Option<String>,
    /// RNG seed
    pub seed: Option<u64>,
    /// Process type (always_ff, always_comb, initial, dll)
    pub process: Option<String>,
}

impl RuntimeContext {
    pub fn new() -> Self {
        RuntimeContext::default()
    }

    pub fn with_time(mut self, time: impl Into<String>) -> Self {
        self.time = Some(time.into());
        self
    }

    pub fn with_delta(mut self, delta: u64) -> Self {
        self.delta = Some(delta);
        self
    }

    pub fn with_thread(mut self, thread: impl Into<String>) -> Self {
        self.thread = Some(thread.into());
        self
    }

    pub fn with_module(mut self, module: impl Into<String>) -> Self {
        self.module = Some(module.into());
        self
    }

    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    pub fn with_process(mut self, process: impl Into<String>) -> Self {
        self.process = Some(process.into());
        self
    }

    /// Format konteks simulasi sebagai string multi-baris.
    pub fn format(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(module) = &self.module {
            parts.push(format!("module : {}", module));
        }
        if let Some(instance) = &self.instance {
            parts.push(format!("instance: {}", instance));
        }
        if let Some(process) = &self.process {
            parts.push(format!("process: {}", process));
        }
        if let Some(time) = &self.time {
            parts.push(format!("time   : {}", time));
        }
        if let Some(delta) = self.delta {
            parts.push(format!("delta  : {}", delta));
        }
        if let Some(thread) = &self.thread {
            parts.push(format!("thread : {}", thread));
        }
        if let Some(seed) = self.seed {
            parts.push(format!("seed   : {}", seed));
        }
        if parts.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        out.push_str("Simulation Context\n\n");
        for part in &parts {
            out.push_str(&format!("  • {}\n", part));
        }
        out
    }
}

// ─── Source Snippet ───

/// Source code snippet yang menunjukkan lokasi error.
#[derive(Debug, Clone)]
pub struct SourceSnippet {
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub source_line: String,
    pub pointer_label: Option<String>,
}

impl SourceSnippet {
    pub fn new(
        file: impl Into<String>,
        line: usize,
        col: usize,
        source_line: impl Into<String>,
    ) -> Self {
        SourceSnippet {
            file: file.into(),
            line,
            col,
            source_line: source_line.into(),
            pointer_label: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.pointer_label = Some(label.into());
        self
    }

    /// Format source snippet dalam gaya Rust.
    pub fn format(&self, use_color: bool) -> String {
        let cyan = if use_color { "\x1b[36m" } else { "" };
        let reset = if use_color { "\x1b[0m" } else { "" };

        let mut out = String::new();
        // Garis atas
        out.push_str(&format!(
            "   {0}┌─{1} {2}:{3}:{4}\n",
            cyan, reset, self.file, self.line, self.col
        ));
        out.push_str(&format!("   {0}│{1}\n", cyan, reset));

        // Baris source
        out.push_str(&format!("{:>4} │ {}\n", self.line, self.source_line));

        // Pointer
        out.push_str("     │ ");
        for _ in 1..self.col.saturating_sub(1) {
            out.push(' ');
        }
        out.push('^');

        // Label pointer
        if let Some(label) = &self.pointer_label {
            out.push_str(&format!(" {0}── {1}{2}", "─", label, reset));
        }
        out.push('\n');

        out
    }
}

// ─── Diagnostic ───

/// Structured diagnostic ala Rust/Cargo (error/warning/note/help).
///
/// Setiap diagnostic memiliki:
/// - Severity (fatal/error/warning/note/info/help/trace/debug)
/// - Error code (RT0001, WR0102, dll)
/// - Title/message utama
/// - Source location + snippet
/// - Runtime context (khusus simulasi HDL)
/// - Explanation
/// - Help/suggestion
/// - Notes tambahan
/// - Example code
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub level: DiagLevel,
    pub code: DiagCode,
    pub message: Cow<'static, str>,
    pub spans: Vec<DiagSpan>,
    pub notes: Vec<DiagNote>,
    pub hints: Vec<Cow<'static, str>>,
    /// Penjelasan panjang tentang penyebab error.
    pub explanation: Option<Cow<'static, str>>,
    /// Saran perbaikan spesifik.
    pub suggestion: Option<Cow<'static, str>>,
    /// Konteks simulasi (waktu, delta, thread, dll).
    pub runtime_context: Option<RuntimeContext>,
    /// Source snippet dengan pointer.
    pub source_snippet: Option<SourceSnippet>,
    /// Contoh kode perbaikan (ditampilkan setelah Help).
    pub example: Option<Cow<'static, str>>,
}

impl Diagnostic {
    pub fn new(level: DiagLevel, code: DiagCode, message: impl Into<Cow<'static, str>>) -> Self {
        Diagnostic {
            level,
            code,
            message: message.into(),
            spans: Vec::new(),
            notes: Vec::new(),
            hints: Vec::new(),
            explanation: None,
            suggestion: None,
            runtime_context: None,
            source_snippet: None,
            example: None,
        }
    }

    /// Buat fatal diagnostic — simulasi harus berhenti.
    pub fn fatal(code: DiagCode, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(DiagLevel::Fatal, code, message)
    }

    pub fn error(code: DiagCode, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(DiagLevel::Error, code, message)
    }

    pub fn warning(code: DiagCode, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(DiagLevel::Warning, code, message)
    }

    pub fn note(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(DiagLevel::Note, DiagCode::SimulationError, message)
    }

    pub fn info(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(DiagLevel::Info, DiagCode::SimulationError, message)
    }

    pub fn bug(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(DiagLevel::Bug, DiagCode::InternalError, message)
    }

    pub fn help(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(DiagLevel::Help, DiagCode::SimulationError, message)
    }

    pub fn with_span(mut self, span: DiagSpan) -> Self {
        self.spans.push(span);
        self
    }

    pub fn with_note(mut self, note: impl Into<Cow<'static, str>>) -> Self {
        self.notes.push(DiagNote::new(note));
        self
    }

    pub fn with_hint(mut self, hint: impl Into<Cow<'static, str>>) -> Self {
        self.hints.push(hint.into());
        self
    }

    pub fn with_explanation(mut self, explanation: impl Into<Cow<'static, str>>) -> Self {
        self.explanation = Some(explanation.into());
        self
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<Cow<'static, str>>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    pub fn with_runtime_context(mut self, ctx: RuntimeContext) -> Self {
        self.runtime_context = Some(ctx);
        self
    }

    pub fn with_source_snippet(mut self, snippet: SourceSnippet) -> Self {
        self.source_snippet = Some(snippet);
        self
    }

    /// Buat diagnostic dengan explanation + help dari DiagCode yang sesuai.
    pub fn with_code_context(mut self) -> Self {
        if self.explanation.is_none() {
            let expl = self.code.explanation();
            if !expl.is_empty() {
                self.explanation = Some(Cow::Borrowed(expl));
            }
        }
        if self.suggestion.is_none() {
            let hlp = self.code.help();
            if !hlp.is_empty() {
                self.suggestion = Some(Cow::Borrowed(hlp));
            }
        }
        self
    }

    /// Tambahkan contoh kode perbaikan (ditampilkan sebagai "Example" setelah Help).
    pub fn with_example(mut self, example: impl Into<Cow<'static, str>>) -> Self {
        self.example = Some(example.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.level.is_error()
    }

    pub fn is_fatal(&self) -> bool {
        self.level.is_fatal()
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}: {}", self.level, self.code, self.message)?;

        for span in &self.spans {
            write!(f, "\n  --> {}:{}:{}", span.file, span.start, span.end)?;
            if let Some(label) = &span.label {
                write!(f, " — {}", label)?;
            }
        }

        if let Some(snippet) = &self.source_snippet {
            write!(f, "\n{}", snippet.format(false))?;
        }

        if let Some(ctx) = &self.runtime_context {
            let ctx_str = ctx.format();
            if !ctx_str.is_empty() {
                write!(f, "\n{}", ctx_str)?;
            }
        }

        if let Some(explanation) = &self.explanation {
            write!(f, "\nExplanation\n\n{}", explanation)?;
        }

        for note in &self.notes {
            write!(f, "\n  = note: {}", note.message)?;
        }

        if let Some(suggestion) = &self.suggestion {
            write!(f, "\n  = help: {}", suggestion)?;
        }

        if let Some(example) = &self.example {
            write!(f, "\n\nExample\n\n{}", example)?;
        }

        for hint in &self.hints {
            write!(f, "\n  = help: {}", hint)?;
        }

        Ok(())
    }
}

// ─── Diagnostic Sink (thread-safe) ───

/// Thread-safe diagnostic collection menggunakan crossbeam channel.
pub struct DiagSink {
    /// MPSC channel untuk cross-thread diagnostics
    sender: crossbeam::channel::Sender<Diagnostic>,
    receiver: crossbeam::channel::Receiver<Diagnostic>,
    /// Collected diagnostics (after flush)
    collected: Mutex<Vec<Diagnostic>>,
    /// Total diagnostics pushed (atomic counter)
    pub total_pushed: AtomicUsize,
}

impl DiagSink {
    pub fn new() -> Self {
        let (sender, receiver) = crossbeam::channel::unbounded();
        DiagSink {
            sender,
            receiver,
            collected: Mutex::new(Vec::new()),
            total_pushed: AtomicUsize::new(0),
        }
    }

    /// Push a diagnostic (non-blocking, lock-free fast path).
    pub fn push(&self, diag: Diagnostic) {
        self.total_pushed.fetch_add(1, Ordering::Relaxed);
        let _ = self.sender.try_send(diag);
    }

    /// Flush all pending diagnostics into collected vec.
    pub fn flush(&self) {
        while let Ok(diag) = self.receiver.try_recv() {
            self.collected.lock().unwrap().push(diag);
        }
    }

    /// Get all collected diagnostics (flush first).
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.flush();
        let mut all = self.collected.lock().unwrap().clone();
        // Sort by file, then by position
        all.sort_by(|a, b| {
            let file_a = a.spans.first().map(|s| s.file.index());
            let file_b = b.spans.first().map(|s| s.file.index());
            file_a.cmp(&file_b).then_with(|| {
                let pos_a = a.spans.first().map(|s| s.start).unwrap_or(0);
                let pos_b = b.spans.first().map(|s| s.start).unwrap_or(0);
                pos_a.cmp(&pos_b)
            })
        });
        all
    }

    /// Get count of errors (not warnings/notes).
    pub fn error_count(&self) -> usize {
        self.flush();
        self.collected
            .lock()
            .unwrap()
            .iter()
            .filter(|d| d.is_error())
            .count()
    }

    /// Are there any errors?
    pub fn has_errors(&self) -> bool {
        self.error_count() > 0
    }

    /// Clear all collected diagnostics.
    pub fn clear(&self) {
        self.collected.lock().unwrap().clear();
    }
}

impl Default for DiagSink {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostic_basic() {
        let d = Diagnostic::error(DiagCode::UnexpectedToken, "found 'foo' where ';' expected");
        assert!(d.is_error());
        assert_eq!(d.level, DiagLevel::Error);
        assert_eq!(d.code, DiagCode::UnexpectedToken);
    }

    #[test]
    fn test_diagnostic_fatal() {
        let d = Diagnostic::fatal(DiagCode::NullHandle, "null handle access");
        assert!(d.is_error());
        assert!(d.is_fatal());
        assert_eq!(d.level, DiagLevel::Fatal);
    }

    #[test]
    fn test_diagnostic_with_source_snippet() {
        let snippet = SourceSnippet::new("test.sv", 42, 14, "fifo.push(data);")
            .with_label("Handle \"fifo\" is null");
        let d = Diagnostic::error(DiagCode::NullHandle, "null handle access")
            .with_source_snippet(snippet);
        assert!(d.source_snippet.is_some());
        let formatted = d.source_snippet.as_ref().unwrap().format(false);
        assert!(formatted.contains("test.sv:42:14"));
        assert!(formatted.contains("fifo.push(data);"));
        assert!(formatted.contains("^"));
    }

    #[test]
    fn test_diagnostic_with_runtime_context() {
        let ctx = RuntimeContext::new()
            .with_time("125 ns")
            .with_delta(12)
            .with_module("uart_top")
            .with_process("write_task");
        let d = Diagnostic::error(DiagCode::NullHandle, "null handle access")
            .with_runtime_context(ctx);
        assert!(d.runtime_context.is_some());
        let formatted = d.runtime_context.as_ref().unwrap().format();
        assert!(formatted.contains("uart_top"));
        assert!(formatted.contains("125 ns"));
        assert!(formatted.contains("write_task"));
    }

    #[test]
    fn test_diagnostic_with_explanation() {
        let d = Diagnostic::error(DiagCode::NullHandle, "null handle access")
            .with_explanation("The FIFO handle was never initialized before simulation started.");
        assert_eq!(
            d.explanation.as_ref().unwrap().as_ref(),
            "The FIFO handle was never initialized before simulation started."
        );
    }

    #[test]
    fn test_diagnostic_with_suggestion() {
        let d = Diagnostic::error(DiagCode::NullHandle, "null handle access")
            .with_suggestion("Initialize fifo before use: fifo = new();");
        assert_eq!(
            d.suggestion.as_ref().unwrap().as_ref(),
            "Initialize fifo before use: fifo = new();"
        );
    }

    #[test]
    fn test_diagnostic_with_span() {
        let file = Symbol::intern("test.sv");
        let d = Diagnostic::error(DiagCode::UndefinedSignal, "signal 'foo' not found")
            .with_span(DiagSpan::new(file, 10, 13).with_label("here"));
        assert_eq!(d.spans.len(), 1);
    }

    #[test]
    fn test_diag_sink_push_flush() {
        let sink = DiagSink::new();
        sink.push(Diagnostic::error(DiagCode::UnexpectedToken, "bad token"));
        sink.push(Diagnostic::warning(
            DiagCode::WidthMismatch,
            "width differs",
        ));

        let diags = sink.diagnostics();
        assert_eq!(diags.len(), 2);
        assert_eq!(sink.error_count(), 1);
    }

    #[test]
    fn test_diag_code_display() {
        assert_eq!(DiagCode::UnexpectedToken.as_str(), "E1001");
        assert_eq!(DiagCode::ModuleNotFound.as_str(), "E3001");
        assert_eq!(DiagCode::NullHandle.as_str(), "RT0001");
        assert_eq!(DiagCode::SignalUnknown.as_str(), "RT1001");
        assert_eq!(DiagCode::InfiniteDelta.as_str(), "RT2001");
        assert_eq!(DiagCode::AssertionFailed.as_str(), "RT7001");
        assert_eq!(DiagCode::UninitializedRegister.as_str(), "WR0014");
        assert_eq!(DiagCode::WidthMismatchWarning.as_str(), "WR0102");
    }

    #[test]
    fn test_diag_level_display() {
        assert_eq!(DiagLevel::Fatal.as_str(), "fatal");
        assert_eq!(DiagLevel::Error.as_str(), "error");
        assert_eq!(DiagLevel::Warning.as_str(), "warning");
        assert_eq!(DiagLevel::Note.as_str(), "note");
        assert_eq!(DiagLevel::Info.as_str(), "info");
        assert_eq!(DiagLevel::Help.as_str(), "help");
        assert_eq!(DiagLevel::Trace.as_str(), "trace");
        assert_eq!(DiagLevel::Debug.as_str(), "debug");
    }

    #[test]
    fn test_diag_code_category() {
        assert_eq!(DiagCode::NullHandle.category(), "Memory");
        assert_eq!(DiagCode::SignalUnknown.category(), "Signal");
        assert_eq!(DiagCode::InfiniteDelta.category(), "Scheduler");
        assert_eq!(DiagCode::AssertionFailed.category(), "Assertion");
        assert_eq!(DiagCode::UninitializedRegister.category(), "Warning");
    }

    #[test]
    fn test_source_snippet_format() {
        let snippet = SourceSnippet::new("cpu_top.sv", 182, 17, "    axi.read(addr);")
            .with_label("interface is null");
        let formatted = snippet.format(false);
        assert!(formatted.contains("cpu_top.sv:182:17"));
        assert!(formatted.contains("axi.read(addr);"));
        // The pointer should be at the right column
        assert!(formatted.contains("^"));
    }

    #[test]
    fn test_runtime_context_format() {
        let ctx = RuntimeContext::new()
            .with_module("cpu_top")
            .with_process("fetch_stage")
            .with_time("125 ns")
            .with_thread("worker-2")
            .with_delta(12)
            .with_seed(129381293);

        let formatted = ctx.format();
        assert!(formatted.contains("module : cpu_top"));
        assert!(formatted.contains("process: fetch_stage"));
        assert!(formatted.contains("time   : 125 ns"));
        assert!(formatted.contains("thread : worker-2"));
        assert!(formatted.contains("delta  : 12"));
        assert!(formatted.contains("seed   : 129381293"));
    }
}
