//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan elaborator.rs (SRP Refactoring).
//! Tanggung jawab: Covergroup, DPI imports, multi-driver signal detection.
//!
//! Fungsi:
//!   - elaborate_covergroups()         — elaborate covergroup definitions
//!   - elaborate_dpi_imports()         — elaborate DPI import declarations
//!   - detect_multi_driver_signals()   — deteksi signal multi-driver
//!   - collect_driven_signals()        — kumpulkan signal yang di-driven (static)
//!
//! ──────────────────────────────────────────────────────────────────────────────

use std::collections::{HashMap, HashSet};

use super::Elaborator;
use maria_ast::ModuleItem;
use maria_core::error::SimError;
use maria_core::intern::Symbol;
use maria_ir::*;

impl Elaborator {
    /// Elaborate covergroup definitions from the top module.
    pub(crate) fn elaborate_covergroups(
        &self,
        top_name: &str,
        signal_map: &HashMap<Symbol, SignalId>,
        signals: &[SignalInfo],
    ) -> Result<Vec<IrCovergroup>, SimError> {
        let mut covergroups = Vec::new();
        let top_module = if let Some(m) = self.design.modules.iter().find(|m| m.name == top_name) {
            m
        } else {
            return Ok(covergroups);
        };
        for item in &top_module.items {
            if let ModuleItem::Covergroup(cg) = item {
                let mut ir_cps = Vec::new();
                for cp in &cg.coverpoints {
                    let ir_expr = self.elaborate_expr(&cp.expr, signal_map, signals)?;
                    ir_cps.push(IrCoverpoint {
                        name: cp.name,
                        expr: ir_expr,
                    });
                }
                let ir_crosses = cg
                    .crosses
                    .iter()
                    .map(|c| IrCross {
                        name: c.name,
                        coverpoints: c.coverpoints.clone(),
                    })
                    .collect();
                covergroups.push(IrCovergroup {
                    name: cg.name,
                    coverpoints: ir_cps,
                    crosses: ir_crosses,
                });
            }
        }
        Ok(covergroups)
    }

    /// Elaborate DPI import declarations from all modules.
    pub(crate) fn elaborate_dpi_imports(&self) -> Result<Vec<IrDpiImport>, SimError> {
        let mut dpi_imports = Vec::new();
        for module in &self.design.modules {
            for item in &module.items {
                if let ModuleItem::DpiImport(dpi) = item {
                    let return_width = dpi.return_type.as_ref().map(|dt| dt.width()).unwrap_or(1);
                    let arg_widths: Vec<usize> = dpi.args.iter().map(|a| a.dtype.width()).collect();
                    dpi_imports.push(IrDpiImport {
                        name: dpi.name,
                        return_width,
                        arg_widths,
                        is_task: dpi.is_task,
                    });
                }
            }
        }
        Ok(dpi_imports)
    }

    /// Detect signals driven by multiple processes (multi-driver).
    pub(crate) fn detect_multi_driver_signals(&self, top: &mut IrModule) -> Result<(), SimError> {
        let mut driver_count: Vec<usize> = vec![0; top.signals.len()];
        for process in &top.processes {
            match process {
                Process::Combinational { body, .. }
                | Process::CombReactive { body, .. }
                | Process::Sequential { body, .. } => {
                    let mut driven = HashSet::new();
                    Self::collect_driven_signals(body, &mut driven);
                    for id in driven {
                        if id < driver_count.len() {
                            driver_count[id] += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        for (id, count) in driver_count.iter().enumerate() {
            if *count > 1 {
                if let Some(sig) = top.signals.get_mut(id) {
                    if sig.kind == SignalKind::Wire
                        || sig.kind == SignalKind::Reg
                        || sig.kind == SignalKind::Inout
                    {
                        sig.multi_driver = true;
                    }
                }
            }
        }
        Ok(())
    }

    /// Static method: collect signal IDs that are driven (assigned) in IR statements.
    fn collect_driven_signals(stmts: &[IrStmt], driven: &mut HashSet<usize>) {
        for stmt in stmts {
            match stmt {
                IrStmt::BlockingAssign { lhs, .. } | IrStmt::NonBlockingAssign { lhs, .. } => {
                    if let IrLValue::Signal(id, _) = lhs {
                        driven.insert(*id);
                    }
                }
                IrStmt::Block { stmts: body } | IrStmt::NamedBlock { stmts: body, .. } => {
                    Self::collect_driven_signals(body, driven);
                }
                IrStmt::If {
                    true_branch,
                    false_branch,
                    ..
                } => {
                    Self::collect_driven_signals(true_branch, driven);
                    Self::collect_driven_signals(false_branch, driven);
                }
                IrStmt::Case { items, default, .. } => {
                    for item in items {
                        Self::collect_driven_signals(&item.body, driven);
                    }
                    Self::collect_driven_signals(default, driven);
                }
                IrStmt::LoopFor { init, body, .. } => {
                    if let Some(init) = init {
                        Self::collect_driven_signals(&[init.as_ref().clone()], driven);
                    }
                    Self::collect_driven_signals(body, driven);
                }
                IrStmt::LoopWhile { body, .. }
                | IrStmt::LoopDoWhile { body, .. }
                | IrStmt::Repeat { body, .. } => {
                    Self::collect_driven_signals(body, driven);
                }
                IrStmt::Delay { body, .. } | IrStmt::Wait { body, .. } => {
                    Self::collect_driven_signals(body, driven);
                }
                _ => {}
            }
        }
    }
}
