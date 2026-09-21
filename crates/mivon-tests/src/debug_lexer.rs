#[cfg(test)]
mod debug_lexer_test {
    use mivon_compiler::frontend::FastLexer;
    use mivon_parser::lexer::{Lexer, Token};

    #[test]
    fn debug_lexer_diff() {
        let input = "module counter(input clk, input rst, output reg [3:0] count);
    always @(posedge clk) begin
        if (rst) count <= 0;
        else count <= count + 1;
    end
endmodule";

        // Legacy
        let mut legacy = Lexer::new(input);
        let mut lt = Vec::new();
        loop {
            let (tok, _, _) = legacy.next_token();
            if tok == Token::Eof {
                break;
            }
            lt.push(tok);
        }

        // Fast
        let mut fast = FastLexer::new(input, "");
        let mut ft = Vec::new();
        loop {
            let (tok, _, _) = fast.next_token();
            if tok == Token::Eof {
                break;
            }
            ft.push(tok);
        }

        assert_eq!(lt.len(), ft.len(), "legacy={}, fast={}", lt.len(), ft.len());

        for (i, (l, f)) in lt.iter().zip(ft.iter()).enumerate() {
            assert_eq!(
                std::mem::discriminant(l),
                std::mem::discriminant(f),
                "Pos {}: legacy={:?} vs fast={:?}",
                i,
                l,
                f
            );
        }
    }
}

#[cfg(test)]
mod debug_syscall_lex {
    use mivon_compiler::frontend::FastLexer;
    use mivon_parser::lexer::{Lexer, Token};

    #[test]
    fn syscall_token_legacy() {
        let input = "module m; initial if ($time) begin end endmodule";
        let mut lexer = Lexer::new(input);
        let mut out = Vec::new();
        loop {
            let (tok, line, col) = lexer.next_token();
            if tok == Token::Eof {
                break;
            }
            out.push(format!("{}:{} {:?}", line, col, tok));
        }
        eprintln!("LEGACY TOKENS: {}", out.join(" | "));
        // Sanity: must contain Dollar
        let has_dollar = out.iter().any(|t| t.contains("Dollar"));
        assert!(
            has_dollar,
            "legacy lexer should produce Dollar: {}",
            out.join(" | ")
        );
    }

    #[test]
    fn syscall_token_fast() {
        let input = "module m; initial if ($time) begin end endmodule";
        let mut lexer = FastLexer::new(input, "");
        let mut out = Vec::new();
        loop {
            let (tok, line, col) = lexer.next_token();
            if tok == Token::Eof {
                break;
            }
            out.push(format!("{}:{} {:?}", line, col, tok));
        }
        eprintln!("FAST TOKENS: {}", out.join(" | "));
        // Sanity: must contain Dollar too
        let has_dollar = out.iter().any(|t| t.contains("Dollar"));
        assert!(
            has_dollar,
            "fast lexer should produce Dollar: {}",
            out.join(" | ")
        );
    }

    // Gap parser: deklarasi tanpa ';' (`logic c = 1` lalu `logic d;` di baris
    // berikut) dulu ditelan diam-diam oleh skip_semi → kode rusak dianggap
    // valid. Kini harus memunculkan warning di lokasi token penyebab (bukan
    // EOF / baris salah).
    #[test]
    fn parse_missing_semicolon_diag_location() {
        use mivon_parser::Parser;
        let source = "module top;\n  logic a;\n  logic b;\n  logic c = 1\n  logic d;\nendmodule";
        // Header-aligned persis seperti compile_str: source_lines[0] =
        // directive `` `line 1 "file" ``, konten baris N di [N].
        let header_line = "`line 1 \"<test>\"";
        let source_with_header = format!("{}\n{}", header_line, source);
        let mut lexer = Lexer::new(&source_with_header);
        let mut tokens = Vec::new();
        loop {
            let (tok, line, col) = lexer.next_token();
            if tok == Token::Eof {
                break;
            }
            tokens.push((tok, line, col));
        }
        let file_line_map = lexer.file_line_map.clone();
        let mut parser = Parser::new(tokens, "<test>")
            .with_source_lines(&source_with_header)
            .with_file_line_map(file_line_map);
        let design = parser
            .parse_design()
            .expect("parse harus sukses (recovery)");
        assert_eq!(design.modules.len(), 1);
        let warns: Vec<_> = parser.errors.iter().filter(|d| !d.is_error()).collect();
        assert!(
            !warns.is_empty(),
            "missing ';' harus memunculkan warning (dulu diam-diam)"
        );
        // Lokasi harus baris 5 (`logic d;` — token tempat ';' hilang), bukan
        // EOF/lokasi lain.
        let any_line5 = warns
            .iter()
            .any(|d| d.source_snippet.as_ref().is_some_and(|s| s.line == 5));
        assert!(
            any_line5,
            "warning harus berlokasi di baris 5, dapat {:?}",
            warns
                .iter()
                .map(|d| d.source_snippet.as_ref().map(|s| (s.line, s.col)))
                .collect::<Vec<_>>()
        );
    }

    // Gap parser: header modul tak diakhiri ';' — `module top {` dulu dianggap
    // valid (token `{` jatuh ke fallback Ok(None) tanpa diag). Kini warning di
    // lokasi `{`.
    #[test]
    fn parse_module_header_bad_token_warns() {
        use mivon_parser::Parser;
        let source = "module top {\n  logic a;\nendmodule";
        let header_line = "`line 1 \"<test>\"";
        let source_with_header = format!("{}\n{}", header_line, source);
        let mut lexer = Lexer::new(&source_with_header);
        let mut tokens = Vec::new();
        loop {
            let (tok, line, col) = lexer.next_token();
            if tok == Token::Eof {
                break;
            }
            tokens.push((tok, line, col));
        }
        let file_line_map = lexer.file_line_map.clone();
        let mut parser = Parser::new(tokens, "<test>")
            .with_source_lines(&source_with_header)
            .with_file_line_map(file_line_map);
        let _ = parser
            .parse_design()
            .expect("parse harus sukses (recovery)");
        let warns: Vec<_> = parser.errors.iter().filter(|d| !d.is_error()).collect();
        assert!(
            !warns.is_empty(),
            "header modul invalid harus memunculkan warning (dulu diam-diam)"
        );
        let any_line1 = warns
            .iter()
            .any(|d| d.source_snippet.as_ref().is_some_and(|s| s.line == 1));
        assert!(
            any_line1,
            "warning harus berlokasi di baris 1 (token `LBrace`), dapat {:?}",
            warns
                .iter()
                .map(|d| d.source_snippet.as_ref().map(|s| (s.line, s.col)))
                .collect::<Vec<_>>()
        );
    }
}
