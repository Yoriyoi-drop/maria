//! Scratch debug helper: preprocess a file (search paths from PP_PATHS) then
//! parse it and report the first error with preprocessed line context.
use maria_parser::preprocessor::Preprocessor;
use maria_parser::Parser;

fn main() {
    let file = std::env::var("PP_FILE").unwrap_or_default();
    let mut pp = Preprocessor::new();
    for sp in std::env::var("PP_PATHS")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
    {
        pp.add_search_path(sp);
    }
    let expanded = match std::env::var("PP_COMBINED") {
        Ok(c_path) => std::fs::read_to_string(&c_path)
            .unwrap_or_else(|e| panic!("cannot read combined {}: {}", c_path, e)),
        Err(_) => match pp.preprocess_file(&file) {
            Ok(expanded) => expanded,
            Err(e) => {
                eprintln!("PREPROCESS ERROR: {}", e);
                return;
            }
        },
    };
    {
        let mut lexer = maria_parser::lexer::Lexer::new(&expanded);
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
            file_line_map[0].2.clone()
        };
        let mut parser = Parser::new(tokens, &first_source)
            .with_source_lines(&expanded)
            .with_file_line_map(file_line_map);
        match parser.parse_design() {
            Ok(_) => println!("PARSE OK"),
            Err(e) => {
                println!("{}", e);
                eprintln!("---- preprocessed source tail ----");
                for (idx, l) in expanded.lines().enumerate().skip(expanded.lines().count().saturating_sub(200)) {
                    println!("{:>6} | {}", idx + 1, l);
                }
            }
        }
    }
}