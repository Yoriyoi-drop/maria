//! Maria SIR — Synthesis Intermediate Representation (SYNTHESIS.md §3).
//!
//! IR node-based khusus synthesis, DIBANGUN dari `IrDesign` hasil elaborator
//! (bukan dari AST — width/kind sudah final). Jauh dari bentuk Verilog:
//! register = `{clock, reset, enable, d, q}`, logika = graf node
//! (`AND/OR/XOR/ADD/MUX/...`).
//!
//! Pipeline fase 1 (RTL → SIR):
//! ```text
//! IrDesign ──lower.rs──► SirModule ──print.rs──► dump teks (--dump-sir)
//! ```
//!
//! Fase berikutnya (pass manager + optimizer) berjalan DI ATAS `SirModule` —
//! bukan lagi di atas IR statement.

pub mod lower;
pub mod print;
pub mod sir;

pub use lower::{LowerResult, lower};
pub use print::{render_sir, value_label};
pub use sir::{
    NodeId, PortDir, PortId, RegisterId, ResetSpec, SirModule, SirNode, SirNodeKind, SirPort,
    SirRegister, SirValue, SirWire, ValueId, WireId,
};

/// Versi library (untuk header dump/laporan).
pub const VERSION: &str = "0.1.0";
