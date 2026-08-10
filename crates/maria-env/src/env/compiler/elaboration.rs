use maria_ast::Design;
use maria_core::diagnostics::Diagnostic;
use maria_elaboration::elaborator::{ElaborateMode, Elaborator};
use maria_core::error::SimError;
use maria_ir::IrDesign;

/// Elaborasi AST → IR. Mengembalikan IR + diagnostics yang ter-flush.
pub fn elaborate(
    design: Design,
    source_lines: Vec<String>,
    source_name: String,
    top: Option<&str>,
    mode: ElaborateMode,
) -> Result<(IrDesign, Vec<Diagnostic>), SimError> {
    let mut elaborator = if source_lines.is_empty() {
        Elaborator::new(design)
    } else {
        Elaborator::with_source(design, source_lines, source_name)
    };
    let ir = elaborator.elaborate(top, mode)?;
    let diags = elaborator.flush_diagnostics();
    Ok((ir, diags))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::compiler::{lex, parse_strict};

    #[test]
    fn test_elaborate_simple() {
        let design = parse_strict(
            lex("module top(input a, output y); assign y = a; endmodule"),
            "t.sv",
        )
        .unwrap();
        let (ir, diags) = elaborate(
            design,
            vec!["module top(input a, output y); assign y = a; endmodule".to_string()],
            "t.sv".to_string(),
            Some("top"),
            ElaborateMode::StrictSimulation,
        )
        .expect("elaborasi harus sukses");
        assert_eq!(ir.top.name.as_str(), "top");
        // Diagnostics hanya warning; tidak boleh ada error.
        assert!(!diags.iter().any(|d| d.is_error()));
    }
}
