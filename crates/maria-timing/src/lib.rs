//! Maria Timing & Area (SYNTHESIS.md §15/§16 — phase 5).
//!
//! Static timing analysis (STA) pada netlist hasil synthesis:
//!
//! ```text
//! netlist (tech/generic) ──analyze──► TimingReport (arrival/required/slack)
//!        │                                 WNS / TNS / critical path
//!        └──constraint .mcs──► Constraint  (clock period, IO delay, ...)
//! ```
//!
//! - `constraint` — parser `.mcs` (Maria Constraint Specification):
//!   `clock clk { period = 10ns; }`, `input_delay 2ns;`, `output_delay 2ns;`,
//!   `max_fanout 32;`, `false_path { from = rst; }`,
//!   `multicycle_path 2 { from = reg_a; to = reg_b; }`.
//! - `timing` — propagation arrival dari startpoint (input port / FF-Q) ke
//!   endpoint (FF-D / output port), slack = required − arrival. WNS = slack
//!   terkecil, TNS = jumlah slack negatif, critical path dilaporkan.
//! - `area` — estimasi area dari resource (LUT/FF/CARRY4/...) dalam satuan
//!   unit area (bukan gate count ASIC — itu `maria-tech`/Liberty fase 6-7).

pub mod area;
pub mod constraint;
pub mod timing;

pub use area::{AreaReport, estimate_area, render_area_report};
pub use constraint::{Constraint, ClockSpec, load_constraints, parse_constraints};
pub use timing::{
    TimingOptions, TimingReport, analyze, render_timing_report,
};

/// Versi library.
pub const VERSION: &str = "0.1.0";
