use std::io::Read;
use maria_parser::preprocessor::Preprocessor;

fn main() {
    let mut src = String::new();
    std::io::stdin().read_to_string(&mut src).unwrap();
    let mut pp = Preprocessor::new();
    match pp.preprocess(&src, None) {
        Ok(expanded) => println!("{}", expanded),
        Err(e) => eprintln!("PREPROCESS ERROR: {}", e),
    }
}