//! Maria Synthesis (SYNTHESIS.md) — RTL → netlist gate-level.
//!
//! Fase S1 (fondasi):
//! - `subset::check` — analisis sintesizability (SYN-1..9)
//! - `infer::infer_netlist` — inferensi primitif RTL (FF) + netlist pra-map
//! - `emit::emit_mvnet` — emisi format `.mvnet`
//! - `report` — report utilisasi & sintesizability
//!
//! Pipeline lengkap `IrDesign → Netlist` diekspos via `synthesize()`.

pub mod emit;
pub mod infer;
pub mod netlist;
pub mod opt;
pub mod pass;
pub mod report;
pub mod subset;
pub mod techmap;

use maria_core::intern::Symbol;
use maria_ir::IrDesign;

pub use infer::{infer_netlist, SynthOpts};
pub use netlist::{CellKind, DeviceKind, Instance, Net, Netlist, PinRef, Port, PortDir};
pub use opt::{ArithmeticSimplify, ConstFold, Cse, Dce, MuxSimplify};
pub use pass::{PassResult, SynthContext, SynthPass, SynthPipeline};
pub use report::DeviceCapacity;
pub use subset::{check as synth_check, SynCheck, SynIssue, SynSeverity};
pub use techmap::{tech_map, TechMapResult};

/// Hasil pipeline synthesis S1.
#[derive(Debug, Clone)]
pub struct SynthOutput {
    pub netlist: Netlist,
    pub check: SynCheck,
}

/// Pipeline synthesis lengkap: SYN check + inferensi netlist.
pub fn synthesize(ir: &IrDesign, opts: &SynthOpts) -> SynthOutput {
    let check = subset::check(ir);
    let netlist = infer_netlist(ir, opts);
    SynthOutput { netlist, check }
}

/// Versi library (untuk header emit).
pub const VERSION: &str = "0.1.0";

/// Konvensi penamaan: nama net listrik dari signal RTL (S1: identik).
pub fn net_name(sig: Symbol) -> Symbol {
    sig
}

/// Re-export untuk maria-tools (agar tool tidak perlu import dua crate).
pub mod prelude {
    pub use crate::emit::{emit_mvnet, emit_summary};
    pub use crate::report::{render_syn_report, render_util_report};
    pub use crate::{synth_check, synthesize};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesize_empty_design_yields_clean() {
        let ir = IrDesign {
            top: maria_ir::IrModule {
                name: Symbol::intern("top"),
                ..Default::default()
            },
            modules: Default::default(),
            classes: Default::default(),
            covergroups: Vec::new(),
            dpi_imports: Vec::new(),
            hier_signal_map: Default::default(),
            udp_defs: Vec::new(),
            specify_items: Vec::new(),
            timescale: None,
            module_functions: Default::default(),
            source_lines: None,
            source_file: None,
            pkg_scoped_consts: Default::default(),
            coverage_exclusions: Vec::new(),
            stmt_lines: std::collections::HashMap::new(),
            net_aliases: std::collections::HashMap::new(),
        };
        let out = synthesize(&ir, &SynthOpts::default());
        assert_eq!(out.check.error_count(), 0);
        assert_eq!(out.netlist.nets.len(), 0);
    }
}
