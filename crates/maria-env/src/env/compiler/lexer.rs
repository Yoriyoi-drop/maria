use maria_compiler::frontend::lexer::FastLexer;
use maria_parser::lexer::{Lexer, Token};

/// Lex source gabungan → daftar token (legacy lexer, posisi kumulatif).
pub fn lex(combined: &str) -> Vec<(Token, usize, usize)> {
    let mut lexer = Lexer::new(combined);
    let mut toks = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == Token::Eof {
            break;
        }
        toks.push((tok, line, col));
    }
    toks
}

/// Lex source gabungan dengan FastLexer byte-level (posisi relative-file).
pub fn lex_fast(combined: &str, source_name: &str) -> Vec<(Token, usize, usize)> {
    let mut lexer = FastLexer::new(combined, source_name);
    let mut toks = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == Token::Eof {
            break;
        }
        toks.push((tok, line, col));
    }
    toks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lex_simple() {
        let toks = lex("module m; endmodule");
        assert!(!toks.is_empty());
        assert!(toks.iter().any(|(t, _, _)| matches!(t, Token::Endmodule)));
    }

    #[test]
    fn test_lex_fast_simple() {
        let toks = lex_fast("module m; endmodule", "t.sv");
        assert!(!toks.is_empty());
    }
}
