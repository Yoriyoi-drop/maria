pub mod ast;
pub mod debugger;
pub mod elaboration;
pub mod error;
pub mod ir;
pub mod parser;
pub mod simulator;
pub mod waveform;

use std::fs;
use std::path::Path;
use parser::lexer::Lexer;
use parser::parser::Parser;
use parser::preprocessor::Preprocessor;
use error::SimError;

/// Read a .maria project file and return list of .sv file paths
/// Paths in .maria are resolved relative to the .maria file's directory
pub fn read_project_file(path: &str) -> Result<Vec<String>, SimError> {
    let content = fs::read_to_string(path)
        .map_err(|e| SimError::new(None, format!("cannot read '{}': {}", path, e)))?;
    let base = Path::new(path).parent().unwrap_or(Path::new("."));
    let files: Vec<String> = content.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let p = base.join(l);
            p.to_string_lossy().to_string()
        })
        .collect();
    if files.is_empty() {
        return Err(SimError::new(None, format!("no .sv files listed in '{}'", path)));
    }
    Ok(files)
}

/// Compile multiple .sv files into IR design
pub fn compile_files(paths: &[String]) -> Result<ir::IrDesign, SimError> {
    let mut combined = String::new();
    for path in paths {
        let mut pp = Preprocessor::new();
        let processed = pp.preprocess_file(path)?;
        combined.push_str(&format!("`line 1 \"{}\"\n", path));
        combined.push_str(&processed);
        combined.push('\n');
    }
    compile_str(&combined)
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
    let preprocessed = pp.preprocess(source, None)
        .map_err(|e| SimError::new(None, format!("preprocessor: {}", e)))?;
    let mut lexer = Lexer::new(&preprocessed);
    let mut tokens = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == parser::lexer::Token::Eof {
            break;
        }
        tokens.push((tok, line, col));
    }

    let mut parser = Parser::new(tokens, "<string>");
    let design = parser.parse_design()?;

    let mut elaborator = elaboration::Elaborator::new(design);
    let ir_design = elaborator.elaborate(None)?;

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

    engine.run()?;

    println!("Simulation completed at time {}", engine.state.time);
    println!("VCD waveform written to '{}'", vcd_path);

    Ok(())
}

/// Run simulation and return final signal values
pub fn simulate_signals(source: &str, max_time: u64) -> Result<Vec<(String, ir::LogicVec)>, SimError> {
    let design = compile_str(source)?;
    let mut engine = simulator::SimulationEngine::new(design, max_time);
    engine.run()?;
    let sigs: Vec<(String, ir::LogicVec)> = engine.design.top.signals.iter()
        .map(|s| (s.name.clone(), engine.state.read_signal(
            engine.design.top.signals.iter()
                .position(|x| x.name == s.name).unwrap_or(0)
        ).clone()))
        .collect();
    Ok(sigs)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod edge_tests;
