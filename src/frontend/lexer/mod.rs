//! SIMD-accelerated byte-level lexer for SystemVerilog.
//!
//! Three layers:
//!   1. `simd.rs` — SIMD primitives (whitespace skip, identifier scan)
//!   2. `lexer.rs` — byte-level SimdLexer producing same tokens as legacy
//!   3. `mod.rs` — re-exports

pub mod simd;
pub mod lexer;

pub use lexer::{SimdLexer, tokenize_simd, tokenize_legacy};
pub use simd::{SimdLevel, detect_simd_level};
