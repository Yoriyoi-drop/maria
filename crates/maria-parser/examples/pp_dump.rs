//! Scratch debug helper: dump preprocessed source for a file with include
//! search paths fed from an environment variable (colon separated) or
//! from the opentitan filelist. Debug aid, not production code.
use maria_parser::preprocessor::Preprocessor;

fn main() {
    let file = std::env::var("PP_FILE").expect("PP_FILE=/path/to/file.sv");
    let mut pp = Preprocessor::new();
    for sp in std::env::var("PP_PATHS")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
    {
        pp.add_search_path(sp);
    }
    match pp.preprocess_file(&file) {
        Ok(expanded) => print!("{}", expanded),
        Err(e) => eprintln!("PREPROCESS ERROR: {}", e),
    }
}