//! Scratch debug helper: EXACT replica of CompileSession per-file parse
//! pipeline (FastLexer + global discovery + per-file Parser with line_base).
//! Debug aid, not production code.
use maria_compiler::frontend::lexer::FastLexer;
use maria_core::intern::Symbol;
use maria_parser::preprocessor::Preprocessor;
use maria_parser::Parser;
use maria_parser::lexer::Token;
use std::collections::HashSet;
use std::path::Path;

fn discover(src: &str, classes: &mut HashSet<Symbol>, typedefs: &mut HashSet<Symbol>) {
    let mut lexer = FastLexer::new(src, "");
    let mut in_typedef = false;
    let mut last_ident: Option<Symbol> = None;
    let mut brace_depth = 0usize;
    loop {
        let (tok, _, _) = lexer.next_token();
        match tok {
            Token::Eof => break,
            Token::Class => {
                loop {
                    let (t, _, _) = lexer.next_token();
                    match t {
                        Token::Eof => break,
                        Token::Ident(n) => {
                            classes.insert(n);
                            break;
                        }
                        _ => break,
                    }
                }
            }
            Token::Typedef => {
                in_typedef = true;
                last_ident = None;
                brace_depth = 0;
            }
            Token::LBrace => {
                if in_typedef {
                    brace_depth += 1;
                }
            }
            Token::RBrace => {
                if in_typedef && brace_depth > 0 {
                    brace_depth -= 1;
                }
            }
            Token::Ident(n) => {
                if in_typedef && brace_depth == 0 {
                    last_ident = Some(n);
                }
            }
            Token::Semi => {
                if in_typedef && brace_depth == 0 {
                    if let Some(n) = last_ident {
                        typedefs.insert(n);
                    }
                    in_typedef = false;
                    last_ident = None;
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let file = std::env::var("PP_FILE").unwrap();
    let filelist = std::env::var("PP_FILELIST").unwrap_or_default();
    let mut pp_paths: Vec<String> = std::env::var("PP_PATHS")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let abs_file = if Path::new(&file).is_absolute() {
        file.clone()
    } else {
        format!("{}/{}", std::env::current_dir().unwrap().display(), file)
    };

    // Preprocess target + (optionally) all other files for discovery
    let mut target_combined = String::new();
    let mut discovery_sources: Vec<String> = Vec::new();
    if filelist.is_empty() {
        let mut pp = Preprocessor::new();
        for sp in &pp_paths {
            pp.add_search_path(sp);
        }
        match pp.preprocess_file(&abs_file) {
            Ok(p) => {
                target_combined = format!("`line 1 \"{}\"\n{}\n", abs_file, p);
                discovery_sources.push(target_combined.clone());
            }
            Err(e) => {
                eprintln!("pp ERR: {}", e);
                return;
            }
        }
    } else {
        let src_list = std::fs::read_to_string(&filelist).unwrap();
        let cwd = std::env::current_dir().unwrap().display().to_string();
        let files: Vec<String> = src_list
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| format!("{}/{}", cwd, l))
            .collect();
        for (i, path) in files.iter().enumerate() {
            let mut pp = Preprocessor::new();
            for sp in &pp_paths {
                pp.add_search_path(sp);
            }
            match pp.preprocess_file(path) {
                Ok(p) => {
                    let combined = format!("`line 1 \"{}\"\n{}\n", path, p);
                    if *path == abs_file {
                        target_combined = combined.clone();
                    }
                    discovery_sources.push(combined);
                }
                Err(e) => eprintln!("pp FAIL {}: {}", path, e),
            }
            if i % 500 == 0 {
                eprintln!("pp {} / {}", i, files.len());
            }
        }
    }

    // Discovery over ALL source
    let mut classes = HashSet::new();
    let mut typedefs = HashSet::new();
    for src in &discovery_sources {
        discover(src, &mut classes, &mut typedefs);
    }
    eprintln!("discovery: classes={} typedefs={}", classes.len(), typedefs.len());

    // Per-file FastLexer parse, exactly like CompileSession
    let mut lexer = FastLexer::new(&target_combined, &abs_file);
    let mut tokens = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == Token::Eof {
            break;
        }
        tokens.push((tok, line + 0 + 1, col));
    }
    let mut parser = Parser::new(tokens, &abs_file)
        .with_global_type_names(&classes, &typedefs)
        .with_source_lines(&target_combined)
        .with_line_base(1);
    match parser.parse_design() {
        Ok(_) => println!("PARSE OK"),
        Err(e) => {
            println!("{}", e);
            for d in &parser.errors {
                println!("-- {}", d);
            }
            eprintln!("---- combined ----");
            for (idx, l) in target_combined.lines().enumerate() {
                if idx < 4000 {
                    println!("{:>6} | {}", idx + 1, l);
                }
            }
        }
    }
}