//! `melab` — Standalone Elaborator.
//!
//! Hanya melakukan: parameter resolve, generate expansion, hierarchy.
//! Tidak menjalankan simulasi.

use maria_elaboration::elaborator::ElaborateMode;
use maria_core::error::SimError;
use crate::{open_elaborated, section, kv};

/// Opsi melab.
pub struct ElabArgs<'a> {
    pub files: &'a [String],
    pub incdirs: &'a [String],
    pub defines: &'a [String],
    pub top: Option<&'a str>,
    pub tree: bool,
    pub params: bool,
    pub signals: bool,
}

/// Jalankan melab.
pub fn run(args: &ElabArgs) -> Result<(), SimError> {
    // Use AnalysisRecovery mode for analysis tools (Rule 10)
    let (session, design, ir) = open_elaborated(args.files, args.incdirs, args.defines, args.top, ElaborateMode::AnalysisRecovery)?;
    let top_name = ir.top.name.as_str();

    section("Elaboration Result");
    kv("top module", top_name);
    kv("modules", ir.modules.len());
    kv("signals (top)", ir.top.signals.len());
    kv("processes (top)", ir.top.processes.len());
    kv("classes", ir.classes.len());
    kv("timescale", ir.timescale.as_ref().map(|t| format!("{}/{}", t.0, t.1)).unwrap_or_else(|| "-".into()));
    kv("parse time", format!("{} ms", session.timing.parse_ms));
    kv("elab time", format!("{} ms", session.timing.elab_ms));

    if args.tree {
        print_tree(&ir);
    }

    if args.params {
        section("Parameters (per module, default dari source)");
        let mut mods: Vec<(&str, &maria_ast::types::Module)> = design
            .modules
            .iter()
            .map(|m| (m.name.as_str(), m))
            .collect();
        mods.sort_by_key(|(n, _)| *n);
        for (name, m) in mods {
            let mut params: Vec<&maria_ast::types::ParamDecl> = m.params.iter().collect();
            for it in &m.items {
                if let maria_ast::types::ModuleItem::Param(p) = it {
                    params.push(p);
                }
            }
            if params.is_empty() {
                continue;
            }
            println!("  {}", name);
            for p in params {
                let kind = if p.is_localparam { "localparam" } else { "parameter" };
                let default = p
                    .default
                    .as_ref()
                    .map(|e| format!(" = {}", crate::expr_to_string(e)))
                    .unwrap_or_default();
                println!("    {:<10} {:<32}{}", kind, p.name.as_str(), default);
            }
        }
    }

    if args.signals {
        section("Signals (top module)");
        for s in &ir.top.signals {
            let kind = match s.kind {
                maria_ir::SignalKind::Reg => "reg",
                maria_ir::SignalKind::Wire => "wire",
                maria_ir::SignalKind::Logic => "logic",
                maria_ir::SignalKind::Input => "input",
                maria_ir::SignalKind::Output => "output",
                maria_ir::SignalKind::Inout => "inout",
            };
            println!(
                "  {:<8} {:<6} bits={:<4} name={}",
                kind,
                format!("[{}:0]", s.width - 1),
                s.width,
                s.name.as_str()
            );
        }
    }

    Ok(())
}

/// Cetak hierarchy tree dari IR (sub_instances).
fn print_tree(ir: &maria_ir::IrDesign) {
    section("Hierarchy Tree");
    println!("  └── {}", ir.top.name.as_str());
    let mut visited: std::collections::HashSet<maria_core::intern::Symbol> = std::collections::HashSet::new();
    for inst in &ir.top.sub_instances {
        print_inst(inst, ir, &mut visited, 1);
    }
}

fn print_inst(
    inst: &maria_ir::IrInstance,
    ir: &maria_ir::IrDesign,
    visited: &mut std::collections::HashSet<maria_core::intern::Symbol>,
    depth: usize,
) {
    let prefix = "  ".repeat(depth + 1);
    println!("{}{} ({}),", prefix, inst.instance_name.as_str(), inst.module_name.as_str());
    if visited.contains(&inst.module_name) {
        println!("{}^ cycle", prefix);
        return;
    }
    if let Some(sub) = ir.modules.get(&inst.module_name) {
        visited.insert(inst.module_name);
        for c in &sub.sub_instances {
            print_inst(c, ir, visited, depth + 1);
        }
        visited.remove(&inst.module_name);
    }
}
