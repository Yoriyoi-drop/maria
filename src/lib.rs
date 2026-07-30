// Allow large Result Err variant for SimError (intentional — Diagnostic contains spans/files)
#![allow(clippy::result_large_err)]

// ── VPI (Verilog Procedural Interface) ──
pub mod vpi;

// ── LSP (Language Server Protocol) ──
pub mod lsp;

// ── Formal Verification Engine ──
pub mod formal;

// ── Core Infrastructure (Fase 0) ──
pub mod arena;
pub mod intern;

// ── Legacy Modules ──
pub mod ast;
pub mod debugger;
pub mod elaboration;
pub mod error;
pub mod ir;
pub mod parser;
pub mod simulator;
pub mod waveform;

// ── New Module Structure (under construction) ──
pub mod backend;
pub mod cache;
pub mod diagnostics;
pub mod frontend;
pub mod hir;
pub mod mir;
pub mod plugin;
pub mod profiling;
pub mod scheduler;

pub use arena::{BumpArena, TypedArena};
pub use error::SimError;
pub use diagnostics::{DiagCode, DiagLevel, Diagnostic, DiagSink, RuntimeContext, SourceSnippet};
pub use frontend::compile_session::{CompileSession, SessionConfig};
pub use frontend::discovery::FileDiscovery;
pub use intern::{init_string_table, Span, Symbol};

use parser::lexer::Lexer;
use parser::preprocessor::Preprocessor;
use parser::Parser;
use std::fs;
use std::path::Path;

/// Compare two ASTs for regression testing. Returns list of structural differences.
pub fn compare_asts(design_a: &ir::IrDesign, design_b: &ir::IrDesign) -> Vec<String> {
    let mut diffs = Vec::new();

    // Compare module count
    if design_a.modules.len() != design_b.modules.len() {
        diffs.push(format!(
            "module count: {} vs {}",
            design_a.modules.len(),
            design_b.modules.len()
        ));
    }

    // Compare signal count
    if design_a.top.signals.len() != design_b.top.signals.len() {
        diffs.push(format!(
            "top signal count: {} vs {}",
            design_a.top.signals.len(),
            design_b.top.signals.len()
        ));
    }

    // Compare process count
    if design_a.top.processes.len() != design_b.top.processes.len() {
        diffs.push(format!(
            "process count: {} vs {}",
            design_a.top.processes.len(),
            design_b.top.processes.len()
        ));
    }

    // Compare each signal info
    for (i, (sa, sb)) in design_a.top.signals.iter().zip(design_b.top.signals.iter()).enumerate() {
        if sa.width != sb.width {
            diffs.push(format!("signal[{}] '{}' width: {} vs {}", i, sa.name, sa.width, sb.width));
        }
        if sa.is_signed != sb.is_signed {
            diffs.push(format!("signal[{}] '{}' signed: {} vs {}", i, sa.name, sa.is_signed, sb.is_signed));
        }
    }

    // Compare class definitions
    if design_a.classes.len() != design_b.classes.len() {
        diffs.push(format!(
            "class count: {} vs {}",
            design_a.classes.len(),
            design_b.classes.len()
        ));
    }

    // Compare covergroups
    if design_a.covergroups.len() != design_b.covergroups.len() {
        diffs.push(format!(
            "covergroup count: {} vs {}",
            design_a.covergroups.len(),
            design_b.covergroups.len()
        ));
    }

    diffs
}

/// Read a .maria project file and return list of .sv file paths
/// Paths in .maria are resolved relative to the .maria file's directory
pub fn read_project_file(path: &str) -> Result<Vec<String>, SimError> {
    let content = fs::read_to_string(path)
        .map_err(|e| SimError::new(None, format!("cannot read '{}': {}", path, e)))?;
    let base = Path::new(path).parent().unwrap_or(Path::new("."));
    let files: Vec<String> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let p = base.join(l);
            p.to_string_lossy().to_string()
        })
        .collect();
    if files.is_empty() {
        return Err(SimError::new(
            None,
            format!("no .sv files listed in '{}'", path),
        ));
    }
    Ok(files)
}

/// Compile multiple .sv files into IR design
pub fn compile_files(paths: &[String]) -> Result<ir::IrDesign, SimError> {
    let mut combined = String::new();
    let mut last_timescale = None;
    for path in paths {
        let mut pp = Preprocessor::new();
        let processed = pp.preprocess_file(path)?;
        if pp.timescale.is_some() {
            last_timescale = pp.timescale.clone();
        }
        combined.push_str(&format!("`line 1 \"{}\"\n", path));
        combined.push_str(&processed);
        combined.push('\n');
    }
    let mut result = compile_str(&combined)?;
    if last_timescale.is_some() && result.timescale.is_none() {
        result.timescale = last_timescale;
    }
    Ok(result)
}

/// Compile a SystemVerilog source file and run simulation
pub fn simulate_file(path: &str, max_time: u64) -> Result<(), SimError> {
    let source = fs::read_to_string(path)
        .map_err(|e| SimError::new(None, format!("cannot read '{}': {}", path, e)))?;
    simulate_str(&source, max_time)
}

/// Compile SystemVerilog source string and run simulation
pub fn simulate_str(source: &str, max_time: u64) -> Result<(), SimError> {
    let design = compile_str(source)?;
    run_simulation(design, max_time)
}

/// Compile SystemVerilog source string into IR
pub fn compile_str(source: &str) -> Result<ir::IrDesign, SimError> {
    let mut pp = Preprocessor::new();
    let preprocessed = pp
        .preprocess(source, None)
        .map_err(|e| SimError::new(None, format!("preprocessor: {}", e)))?;
    let timescale = pp.timescale.clone();
    let mut lexer = Lexer::new(&preprocessed);
    let mut tokens = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == parser::lexer::Token::Eof {
            break;
        }
        tokens.push((tok, line, col));
    }

    let file_line_map = lexer.file_line_map.clone();
    let first_source = if file_line_map.is_empty() {
        "<string>".to_string()
    } else {
        file_line_map[0].1.clone()
    };
    let mut parser = Parser::new(tokens, &first_source)
        .with_source_lines(&preprocessed)
        .with_file_line_map(file_line_map);
    let mut design = match parser.parse_design() {
        Ok(d) => d,
        Err(e) => {
            // Parse function returned fatal error — emit collected errors too
            if !parser.errors.is_empty() {
                let mut emitter = diagnostics::TerminalEmitter::new().with_simple_mode(true);
                for diag in &parser.errors {
                    let _ = emitter.emit(diag);
                }
            }
            return Err(e);
        }
    };
    // Cek accumulated parser diagnostics (warnings + errors)
    // Hanya abort untuk real errors, warnings seperti "skipping construct" tetap lanjut
    if !parser.errors.is_empty() {
        let has_real_errors = parser.errors.iter().any(|d| d.is_error());
        let mut emitter = diagnostics::TerminalEmitter::new().with_simple_mode(true);
        for diag in &parser.errors {
            let _ = emitter.emit(diag);
        }
        if has_real_errors {
            return Err(SimError::from_parse_diagnostic(parser.errors[0].clone()));
        }
    }
    design.timescale = timescale;

    let source_lines: Vec<String> = preprocessed.lines().map(|s| s.to_string()).collect();
    let mut elaborator = elaboration::Elaborator::with_source(design, source_lines, first_source);
    let ir_design = elaborator.elaborate(None)?;

    // Flush elaboration-time diagnostics (warnings like WR0102)
    let elab_diags = elaborator.flush_diagnostics();
    if !elab_diags.is_empty() {
        let mut emitter = diagnostics::TerminalEmitter::new().with_simple_mode(true);
        for diag in &elab_diags {
            let _ = emitter.emit(diag);
        }
    }

    Ok(ir_design)
}

/// Run simulation on compiled IR
pub fn run_simulation(ir_design: ir::IrDesign, max_time: u64) -> Result<(), SimError> {
    let mut engine = simulator::SimulationEngine::new(ir_design, max_time);

    let design_name = &engine.design.top.name.clone();
    let vcd_path = format!("{}.vcd", design_name);
    let vcd = waveform::VcdWriter::new(&vcd_path, &engine.design)
        .map_err(|e| SimError::new(None, format!("VCD creation failed: {}", e)))?;
    engine.set_vcd(vcd);

    // Also create FST waveform
    let fst_path = format!("{}.fst", design_name);
    match waveform::FstWaveWriter::new(&fst_path, &engine.design) {
        Ok(fst) => engine.set_fst(fst),
        Err(e) => {
            let diag = diagnostics::Diagnostic::warning(
                diagnostics::DiagCode::WaveformError,
                format!("FST: cannot create '{}': {}", fst_path, e),
            );
            let mut emitter = diagnostics::TerminalEmitter::new().with_simple_mode(true);
            let _ = emitter.emit(&diag);
        }
    }

    engine.run()?;

    // Flush any runtime diagnostics
    let diagnostics = engine.flush_diagnostics();
    if !diagnostics.is_empty() {
        let mut emitter = diagnostics::TerminalEmitter::new().with_simple_mode(true);
        for diag in &diagnostics {
            let _ = emitter.emit(diag);
        }
    }

    println!("Simulation completed at time {}", engine.state.time);
    println!("VCD waveform written to '{}'", vcd_path);
    println!("FST waveform written to '{}'", fst_path);

    Ok(())
}

/// Run simulation and return final signal values
pub fn simulate_signals(
    source: &str,
    max_time: u64,
) -> Result<Vec<(String, ir::LogicVec)>, SimError> {
    let design = compile_str(source)?;
    let mut engine = simulator::SimulationEngine::new(design, max_time);
    engine.run()?;
    let sigs: Vec<(String, ir::LogicVec)> = engine
        .design
        .top
        .signals
        .iter()
        .map(|s| {
            (
                s.name.to_string(),
                engine
                    .state
                    .read_signal(
                        engine
                            .design
                            .top
                            .signals
                            .iter()
                            .position(|x| x.name == s.name)
                            .unwrap_or(0),
                    )
                    .clone(),
            )
        })
        .collect();
    Ok(sigs)
}

#[cfg(test)]
#[macro_use]
pub mod test_util;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod edge_tests;
