//! Debug test: apakah localparam body (`localparam int VccPokStrNum = 4;`)
//! tersimpan di module AST hasil CompileSession.parse? — untuk F37/F38 fix.
use maria_ast::ModuleItem;

#[test]
fn debug_nonansi_port_width() {
    // Cek apakah range decl non-ANSI (`input [7:0] a;`) ter-attach ke Port.
    let src = "module alu_opt(a, b, y);\n  input [7:0] a;\n  input [7:0] b;\n  output [7:0] y;\n  assign y = a | b;\nendmodule\n";
    // Jalur legacy (sama dengan maria_api::compile_str yang dipakai synth test).
    let mut pp = maria_parser::preprocessor::Preprocessor::new();
    let pre = pp.preprocess(src, None).unwrap();
    let mut lexer = maria_parser::lexer::Lexer::new(&pre);
    let mut tokens = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == maria_parser::lexer::Token::Eof {
            break;
        }
        tokens.push((tok, line, col));
    }
    let mut parser = maria_parser::Parser::new(tokens, "<string>")
        .with_source_lines(&pre)
        .with_file_line_map(lexer.file_line_map.clone());
    let design = parser.parse_design().unwrap();
    for m in &design.modules {
        if m.name.as_str() != "alu_opt" {
            continue;
        }
        eprintln!("MODULE alu_opt ports={}", m.ports.len());
        for p in &m.ports {
            eprintln!(
                "  PORT name={} dir={:?} range={:?} expr_range={:?}",
                p.name.as_str(),
                p.direction,
                p.range,
                p.expr_range
            );
        }
        eprintln!("  decls={}", m.decls.len());
        for d in &m.decls {
            for v in &d.names {
                eprintln!(
                    "  DECL name={} kind={:?} range={:?} expr_range={:?}",
                    v.name.as_str(),
                    d.kind,
                    v.range,
                    v.expr_range
                );
            }
        }
    }
}

#[test]
fn debug_parse_body_localparam() {
    let path = "/home/whale-d/maria/opentitan/hw/top_englishbreakfast/ip/ast/rtl/rglts_pdm_3p3v.sv";
    let raw = std::fs::read_to_string(path).unwrap();
    let mut pp = maria_parser::preprocessor::Preprocessor::new();
    let pre = pp.preprocess(&raw, None).unwrap();
    let combined = format!("`line 1 \"{}\"\n{}", path, pre);

    // Tes lexing langsung pada baris $value$plusargs
    {
        let line =
            "  if ( !$value$plusargs(\"accelerate_regulators_power_up_time=%d\", dv_hook) ) begin";
        for (label, toks) in [
            ("F-$", {
                let mut lex = maria_compiler::frontend::FastLexer::new(line, "x");
                let mut t = Vec::new();
                loop {
                    let (tok, l, c) = lex.next_token();
                    if tok == maria_parser::lexer::Token::Eof {
                        break;
                    }
                    t.push(format!("{:?}@{}:{}", tok, l, c));
                }
                t
            }),
            ("L-$", {
                let mut lex = maria_parser::lexer::Lexer::new(line);
                let mut t = Vec::new();
                loop {
                    let (tok, l, c) = lex.next_token();
                    if tok == maria_parser::lexer::Token::Eof {
                        break;
                    }
                    t.push(format!("{:?}@{}:{}", tok, l, c));
                }
                t
            }),
        ] {
            eprintln!("{}: {}", label, toks.join(" "));
        }
    }
    // Dump combined source lines 55-75 untuk konteks divergensi
    {
        let lines: Vec<&str> = combined.lines().collect();
        for i in 55..75.min(lines.len()) {
            eprintln!("C[{}]: {}", i + 1, lines[i]);
        }
    }
    // Bandingkan token streams: cari divergensi pertama
    {
        let mut lex_f = maria_compiler::frontend::FastLexer::new(&combined, path);
        let mut lex_l = maria_parser::lexer::Lexer::new(&combined);
        let mut n = 0;
        loop {
            let (tf, lf, cf) = lex_f.next_token();
            let (tl, ll, cl) = lex_l.next_token();
            let same = std::mem::discriminant(&tf) == std::mem::discriminant(&tl);
            if !same {
                eprintln!(
                    "DIVERGE at tok#{}: FAST={:?}({}:{}) LEGACY={:?}({}:{})",
                    n, tf, lf, cf, tl, ll, cl
                );
                break;
            }
            if tf == maria_parser::lexer::Token::Eof {
                eprintln!("TOKEN MATCH: {} tokens identical", n);
                break;
            }
            n += 1;
            if n > 200000 {
                eprintln!("TOKEN MATCH: reached 200k, identical so far");
                break;
            }
        }
    }
    // Tes dengan FastLexer (jalur CompileSession) vs LegacyLexer
    for (label, toks) in [
        ("FAST", {
            let mut lexer = maria_compiler::frontend::FastLexer::new(&combined, path);
            let mut t = Vec::new();
            loop {
                let (tok, line, col) = lexer.next_token();
                if tok == maria_parser::lexer::Token::Eof {
                    break;
                }
                t.push((tok, line, col));
            }
            t
        }),
        ("LEGACY", {
            let mut lexer = maria_parser::lexer::Lexer::new(&combined);
            let mut t = Vec::new();
            loop {
                let (tok, line, col) = lexer.next_token();
                if tok == maria_parser::lexer::Token::Eof {
                    break;
                }
                t.push((tok, line, col));
            }
            t
        }),
    ] {
        let mut parser = maria_parser::Parser::new(toks, path);
        match parser.parse_design() {
            Ok(design) => {
                for m in &design.modules {
                    let n_param_items = m
                        .items
                        .iter()
                        .filter(|it| matches!(it, ModuleItem::Param(_)))
                        .count();
                    eprintln!(
                        "{} MODULE: {} params={} items={} decls={} param_items={}",
                        label,
                        m.name.as_str(),
                        m.params.len(),
                        m.items.len(),
                        m.decls.len(),
                        n_param_items
                    );
                }
                eprintln!("{} ERRORS: {}", label, parser.errors.len());
            }
            Err(e) => eprintln!("{} FAIL: {:?}", label, e),
        }
    }
}
