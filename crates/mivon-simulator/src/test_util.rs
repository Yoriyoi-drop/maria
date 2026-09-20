//! Test utilities untuk mivon-simulator.
//!
//! `compile_str` mereplikasi pipeline root `mivon::compile_str`
//! (preprocessor → lexer → parser → elaborator) agar test di crate ini
//! bisa membangun `IrDesign` dari source string tanpa bergantung pada
//! crate root (yang akan menjadi binary-only pada akhir migrasi).

use mivon_core::diagnostics::DiagCode;
use mivon_core::error::SimError;
use mivon_elaboration::ElaborateMode;
use mivon_parser::lexer::Lexer;
use mivon_parser::preprocessor::Preprocessor;
use mivon_parser::Parser;

/// Compile source string → IrDesign (pipeline lengkap, mirror `mivon::compile_str`).
pub fn compile_str(source: &str) -> Result<mivon_ir::IrDesign, SimError> {
    let mut pp = Preprocessor::new();
    let preprocessed = pp
        .preprocess(source, None)
        .map_err(|e| SimError::with_diag(DiagCode::InvalidSyntax, format!("preprocessor: {e}")))?;
    let timescale = pp.timescale.clone();
    let mut lexer = Lexer::new(&preprocessed);
    let mut tokens = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == mivon_parser::lexer::Token::Eof {
            break;
        }
        tokens.push((tok, line, col));
    }

    let file_line_map = lexer.file_line_map.clone();
    let first_source = if file_line_map.is_empty() {
        "<string>".to_string()
    } else {
        file_line_map[0].2.clone()
    };
    let mut parser = Parser::new(tokens, &first_source)
        .with_source_lines(&preprocessed)
        .with_file_line_map(file_line_map);
    let mut design = match parser.parse_design() {
        Ok(d) => d,
        Err(e) => {
            if !parser.errors.is_empty() {
                let mut emitter =
                    mivon_core::diagnostics::TerminalEmitter::new().with_simple_mode(true);
                for diag in &parser.errors {
                    let _ = emitter.emit(diag);
                }
            }
            return Err(e);
        }
    };
    if !parser.errors.is_empty() {
        let has_real_errors = parser.errors.iter().any(|d| d.is_error());
        let mut emitter = mivon_core::diagnostics::TerminalEmitter::new().with_simple_mode(true);
        for diag in &parser.errors {
            let _ = emitter.emit(diag);
        }
        if has_real_errors {
            return Err(SimError::from_parse_diagnostic(parser.errors[0].clone()));
        }
    }
    design.timescale = timescale;

    let source_lines: Vec<String> = preprocessed.lines().map(|s| s.to_string()).collect();
    let mut elaborator =
        mivon_elaboration::Elaborator::with_source(design, source_lines, first_source);
    let ir_design = elaborator.elaborate(None, ElaborateMode::StrictSimulation)?;

    // Paritas dengan `mivon::compile_str`: bawa exclusion ranges dari
    // `` `coverage_off ``/`` `coverage_on `` (koordinat output preprocessed)
    // ke design untuk engine line coverage.
    let mut ir_design = ir_design;
    ir_design.coverage_exclusions = pp.coverage_exclusions.clone();

    let elab_diags = elaborator.flush_diagnostics();
    if !elab_diags.is_empty() {
        let mut emitter = mivon_core::diagnostics::TerminalEmitter::new().with_simple_mode(true);
        for diag in &elab_diags {
            let _ = emitter.emit(diag);
        }
    }

    Ok(ir_design)
}
