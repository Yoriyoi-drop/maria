//! Pass Manager synthesis (SYNTHESIS.md §4).
//!
//! Optimasi/mapping BUKAN pipeline hardcode — setiap tahap adalah pass yang
//! didaftarkan (`SynthPass`), dikomposisi lewat `SynthPipeline`, dan dipilih
//! via preset (`generic` / `fpga` / `asic` / `custom`).
//!
//! ```rust
//! use maria_synth::{ArithmeticSimplify, ConstFold, Cse, Dce, MuxSimplify, SynthPipeline};
//! use maria_core::intern::Symbol;
//! use maria_sir::SirModule;
//!
//! let module = SirModule::new(Symbol::intern("top"));
//! let mut p = SynthPipeline::new();
//! p.add(ConstFold)
//!     .add(ArithmeticSimplify)
//!     .add(MuxSimplify)
//!     .add(Cse)
//!     .add(Dce);
//! let (module, results) = p.run(module).expect("optimasi gagal");
//! assert_eq!(results.len(), 5);
//! # let _ = results;
//! # let _ = module;
//! ```

use maria_core::error::SimError;
use maria_sir::SirModule;

use crate::opt::{ArithmeticSimplify, ConstFold, Cse, Dce, MuxSimplify};

/// Hasil satu pass (untuk report/statistik).
#[derive(Debug, Clone, PartialEq)]
pub struct PassResult {
    pub name: &'static str,
    pub nodes_before: usize,
    pub nodes_after: usize,
    /// Jumlah rewrite (fold/alias/simplify/eliminasi).
    pub changed: usize,
}

/// Konteks pass — saat ini hanya modul SIR (mapping/tech menyusul).
#[derive(Debug, Clone)]
pub struct SynthContext {
    pub module: SirModule,
}

/// Trait pass synthesis. Mengembalikan jumlah rewrite (`changed`) — statistik
/// node dihitung pipeline (pass tidak perlu tahu jumlah node).
pub trait SynthPass {
    fn name(&self) -> &'static str;
    fn run(&mut self, ctx: &mut SynthContext) -> Result<usize, SimError>;
}

/// Pipeline pass terkomposisi.
pub struct SynthPipeline {
    passes: Vec<Box<dyn SynthPass>>,
}

impl SynthPipeline {
    pub fn new() -> Self {
        SynthPipeline { passes: Vec::new() }
    }

    /// Daftarkan pass (builder-style: `.add(A).add(B)`).
    pub fn add<P: SynthPass + 'static>(&mut self, pass: P) -> &mut Self {
        self.passes.push(Box::new(pass));
        self
    }

    /// Jalankan semua pass berurutan. Modul dipindah (ownership) — tanpa clone.
    pub fn run(&mut self, module: SirModule) -> Result<(SirModule, Vec<PassResult>), SimError> {
        let mut ctx = SynthContext { module };
        let mut results = Vec::new();
        for pass in self.passes.iter_mut() {
            let name = pass.name();
            let nodes_before = ctx.module.nodes.len();
            let changed = pass.run(&mut ctx)?;
            results.push(PassResult {
                name,
                nodes_before,
                nodes_after: ctx.module.nodes.len(),
                changed,
            });
        }
        Ok((ctx.module, results))
    }

    /// Pipeline bawaan per preset. Phase 2: semua preset memakai pipeline
    /// optimasi yang sama (technology mapping menyusul di phase 4/7/8).
    pub fn with_preset(name: &str) -> Result<Self, SimError> {
        match name {
            "generic" | "fpga" | "asic" | "custom" => {
                let mut p = SynthPipeline::new();
                p.add(ConstFold);
                p.add(ArithmeticSimplify);
                p.add(MuxSimplify);
                p.add(Cse);
                p.add(Dce);
                // Fixed-point: satu putaran tambahan agar optimasi yang
                // membuka peluang fold baru (mis. Mux(Not(c),t,f) → swap
                // → sel konstanta) ikut tersederhanakan.
                p.add(ConstFold);
                p.add(Dce);
                Ok(p)
            }
            other => Err(SimError::with_diag(
                maria_core::diagnostics::DiagCode::InvalidSyntax,
                format!(
                    "preset '{}' tidak dikenal — pakai 'generic' | 'fpga' | 'asic' | 'custom'",
                    other
                ),
            )),
        }
    }
}

impl Default for SynthPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_core::intern::Symbol;
    use maria_core::LogicVec;
    use maria_sir::{SirNodeKind, SirValue, ValueId};

    #[test]
    fn pipeline_reports_per_pass_results_and_removes_dead() {
        let mut m = SirModule::new(Symbol::intern("top"));
        // And(0, 1) → 0 (const fold) → node mati → DCE buang.
        m.add_value(SirValue::Const(LogicVec::from_u64(0, 8))); // vid 0
        m.add_value(SirValue::Const(LogicVec::from_u64(1, 8))); // vid 1
        let nid = m.add_node(SirNodeKind::And, vec![0, 1], 8);
        let out = m.nodes[nid].output;
        m.add_value(SirValue::Node(nid));
        assert_eq!(out, 2, "output = index value berikutnya");

        let mut p = SynthPipeline::with_preset("fpga").unwrap();
        let (m2, results) = p.run(m).unwrap();
        assert_eq!(
            results.len(),
            7,
            "const_fold, arith, mux, cse, dce + fixed-point fold/dce"
        );
        // And(0,1) → const 0 → node mati → DCE menghapusnya. Dengan
        // fixed-point, DCE pertama yang membersihkan; DCE kedua (trailing)
        // sah-sah saja no-op — cek ADA pass DCE yang mengurangi node.
        assert_eq!(m2.nodes.len(), 0, "node And harus dihapus DCE");
        let removed = results
            .iter()
            .any(|r| r.name == "dce" && r.nodes_after < r.nodes_before);
        assert!(
            removed,
            "harus ada pass DCE yang menghapus node mati: {results:?}"
        );
    }

    #[test]
    fn unknown_preset_is_error() {
        assert!(SynthPipeline::with_preset("nope").is_err());
    }

    #[test]
    fn output_field_is_consistent() {
        let mut m = SirModule::new(Symbol::intern("top"));
        let _ = m.add_value(SirValue::Const(LogicVec::from_u64(0, 1)));
        let nid = m.add_node(SirNodeKind::Not, vec![0], 1);
        let vid: ValueId = m.add_value(SirValue::Node(nid));
        assert_eq!(m.nodes[nid].output, vid);
    }
}
