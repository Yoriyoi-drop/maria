use maria_ast::Design;
use maria_core::diagnostics::Diagnostic;
use maria_core::error::SimError;
use maria_parser::lexer::Token;
use maria_parser::Parser;

/// Parse daftar token → Design. Diagnostics parser dikembalikan terpisah
/// (caller yang memutuskan fatal/tidak).
pub fn parse(
    tokens: Vec<(Token, usize, usize)>,
    source_name: &str,
) -> Result<(Design, Vec<Diagnostic>), SimError> {
    let mut parser = Parser::new(tokens, source_name);
    let design = parser.parse_design()?;
    Ok((design, parser.errors.clone()))
}

/// Parse daftar token → Design, ERROR bila ada error parser.
pub fn parse_strict(
    tokens: Vec<(Token, usize, usize)>,
    source_name: &str,
) -> Result<Design, SimError> {
    let (design, errors) = parse(tokens, source_name)?;
    if let Some(e) = errors.iter().find(|d| d.is_error()) {
        return Err(SimError::from_parse_diagnostic(e.clone()));
    }
    Ok(design)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::compiler::lex;

    #[test]
    fn test_parse_basic() {
        let toks = lex("module m; endmodule");
        let (design, errs) = parse(toks, "t.sv").unwrap();
        assert!(errs.is_empty());
        assert_eq!(design.modules.len(), 1);
    }

    #[test]
    fn test_parse_strict_ok() {
        let toks = lex("module m; endmodule");
        assert!(parse_strict(toks, "t.sv").is_ok());
    }
}
