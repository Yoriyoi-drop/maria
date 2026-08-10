//! Compiler context — pipeline compiler (preprocess → lexer → parser →
//! AST → HIR → elaborasi → optimizer).
//!
//! Compiler TIDAK tahu database/GUI/logger — semua lewat context.

mod ast;
mod compiler;
mod elaboration;
mod hir;
mod lexer;
mod optimize;
mod parser;
mod preprocess;

pub use ast::{merge_all, merge_designs};
pub use compiler::CompilerContext;
pub use elaboration::elaborate;
pub use hir::HirHandle;
pub use lexer::{lex, lex_fast};
pub use optimize::{OptimizeLevel, apply_optimizations};
pub use parser::{parse, parse_strict};
pub use preprocess::{build_preprocessor, preprocess_file, preprocess_str};
