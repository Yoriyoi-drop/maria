//! Scratch debug helper: mimic main.rs batch flow — preprocess every file of a
//! filelist (fresh Preprocessor each, search paths from PP_PATHS and the file's
//! own directory), prepend `` `line 1 "path" `` and parse the combined stream.
//! Debug aid, not production code.
use maria_parser::lexer::Lexer;
use maria_parser::preprocessor::Preprocessor;
use maria_parser::Parser;

fn main() {
    let filelist = std::env::var("PP_FILELIST").unwrap();
    let mut pp_paths: Vec<String> = std::env::var("PP_PATHS")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let src_list = std::fs::read_to_string(&filelist).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {}", filelist, e);
        std::process::exit(1);
    });
    let files: Vec<String> = src_list
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| format!("{}/{}", std::env::current_dir().unwrap().display(), l))
        .collect();

    let mut combined = String::new();
    let mut failures: Vec<String> = Vec::new();
    for (i, path) in files.iter().enumerate() {
        let mut pp = Preprocessor::new();
        for sp in &pp_paths {
            pp.add_search_path(sp);
        }
        match pp.preprocess_file(path) {
            Ok(processed) => {
                combined.push_str(&format!("`line 1 \"{}\"\n{}\n", path, processed));
            }
            Err(e) => failures.push(format!("pp FAIL {}: {}", path, e)),
        }
        if i % 500 == 0 {
            eprintln!("preprocessed {} / {}", i, files.len());
        }
    }
    for f in &failures {
        eprintln!("{}", f);
    }
    eprintln!("combined bytes: {}", combined.len());

    let mut lexer = Lexer::new(&combined);
    let mut tokens = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == maria_parser::lexer::Token::Eof {
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
        .with_source_lines(&combined)
        .with_file_line_map(file_line_map);
    match parser.parse_design() {
        Ok(_) => println!("PARSE OK ({} files)", files.len()),
        Err(e) => {
            println!("{}", e);
            for d in &parser.errors {
                println!("-- {}", d);
            }
        }
    }
}