//! Maria — Test Suite Terpadu.
//!
//! Seluruh test suite (`src/tests/`, `src/edge_tests.rs`, `src/debug_lexer.rs`)
//! pindah ke crate ini pada migrasi monorepo. `crate::simulator`,
//! `crate::compile_str`, `crate::compare_asts`, dll. tetap valid karena
//! crate ini mere-export seluruh API `maria_api` (dan `simulator` milik
//! maria-simulator) ke akar crate.
//!
//! Menjalankan: `cargo test -p maria-tests` (atau `cargo test --workspace`).

pub use maria_api::*;

// Private use mirror dari lib.rs lama — dibawa ke submodule test via glob
// `use super::*` (tests/mod.rs, edge_tests.rs) sehingga `fs::…`, `Lexer`,
// `Parser`, `Preprocessor` tetap valid tanpa edit di 15K LOC test.
// (cfg(test): hanya terpakai saat crate ini di-test, hindari unused-import
// warning pada build lib biasa.)
#[cfg(test)]
use std::fs;
#[cfg(test)]
use maria_parser::lexer::Lexer;
#[cfg(test)]
use maria_parser::preprocessor::Preprocessor;
#[cfg(test)]
use maria_parser::Parser;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod edge_tests;

#[cfg(test)]
mod debug_lexer;
