//! Maria Netlist — netlist gate-level generik hasil mapping SIR
//! (SYNTHESIS.md §11/§13, phase 3).
//!
//! ```text
//! SirModule ──lower.rs──► Netlist ──emit.rs──► netlist.v / .mvnet
//!                              │
//!                              ├──graph.rs──► DAG check + level + stats
//!                              └──json.rs───► netlist.json (GUI/CI)
//! ```
//!
//! Aturan netlist Maria:
//! 1. **1 driver, N loads** — acyclic DAG.
//! 2. Sel = primitif technology-agnostik (teknologi nyata di `maria-tech`,
//!    phase 4/7). Konstanta = net dengan `const_value`, tanpa driver sel.
//! 3. Deterministik — lowering dari `SirModule` yang sama selalu identik
//!    (bisa di-commit/di-diff, incremental synthesis friendly).

pub mod cell;
pub mod emit;
pub mod graph;
pub mod json;
pub mod lower;
pub mod net;

pub use cell::{CellId, CellInstance, CellKind, PinConn, PinRef};
pub use emit::{emit_mvnet, emit_summary, emit_verilog};
pub use graph::{combinational_levels, stats, verify_dag, DagCheck, NetlistStats};
pub use json::emit_json;
pub use lower::lower_module;
pub use net::{Net, NetId, Netlist, Port, PortDir};

/// Versi library (untuk header).
pub const VERSION: &str = "0.1.0";
