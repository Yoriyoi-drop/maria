use super::*;

#[test]
fn test_preprocess_then_lex() {
    use maria_parser::lexer::{Lexer, Token};
    use maria_parser::preprocessor::Preprocessor;
    let path = std::path::Path::new("/tmp/pp_test.sv");
    if !path.exists() {
        std::fs::write(path, "module m; initial if ($time) begin end endmodule\n").unwrap();
    }
    let mut pp = Preprocessor::new();
    let path_str = path.to_str().unwrap();
    let processed = pp.preprocess_file(path_str).unwrap();
    let combined = format!("`line 1 \"{}\"\n{}\n", path.display(), processed);
    eprintln!("COMBINED: {:?}", combined);
    let mut lexer = Lexer::new(&combined);
    let mut out = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == Token::Eof { break; }
        out.push(format!("{}:{} {:?}", line, col, tok));
    }
    eprintln!("PPLEX TOKENS: {}", out.join(" | "));
    let has_dollar = out.iter().any(|t| t.contains("Dollar"));
    assert!(has_dollar, "expected Dollar token: {}", out.join(" | "));
}

#[test]
fn test_lex_dollar_with_line_directive() {
    use maria_parser::lexer::{Lexer, Token};
    // combined persis seperti di main.rs run(): prefix `line directive
    let input = "`line 1 \"/tmp/mini.sv\"\nmodule m; initial if ($time) begin end endmodule\n";
    let mut lexer = Lexer::new(input);
    let mut out = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == Token::Eof { break; }
        out.push(format!("{}:{} {:?}", line, col, tok));
    }
    eprintln!("LINEDIR TOKENS: {}", out.join(" | "));
    let has_dollar = out.iter().any(|t| t.contains("Dollar"));
    assert!(has_dollar, "expected Dollar token: {}", out.join(" | "));
}

#[test]
fn test_lex_dollar_time_legacy() {
    use maria_parser::lexer::{Lexer, Token};
    let input = "module m; initial if ($time) begin end endmodule";
    let mut lexer = Lexer::new(input);
    let mut out = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == Token::Eof { break; }
        out.push(format!("{}:{} {:?}", line, col, tok));
    }
    eprintln!("LEGACY TOKENS: {}", out.join(" | "));
    let has_dollar = out.iter().any(|t| t.contains("Dollar"));
    assert!(has_dollar, "legacy lexer should produce Dollar: {}", out.join(" | "));
}

#[test]
fn test_lex_dollar_time_fast() {
    use maria_compiler::frontend::FastLexer;
    use maria_parser::lexer::Token;
    let input = "module m; initial if ($time) begin end endmodule";
    let mut lexer = FastLexer::new(input, "");
    let mut out = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == Token::Eof { break; }
        out.push(format!("{}:{} {:?}", line, col, tok));
    }
    eprintln!("FAST TOKENS: {}", out.join(" | "));
    let has_dollar = out.iter().any(|t| t.contains("Dollar"));
    assert!(has_dollar, "fast lexer should produce Dollar: {}", out.join(" | "));
}
