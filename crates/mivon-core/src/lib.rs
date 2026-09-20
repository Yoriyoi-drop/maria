//! mivon-core — fondasi Mivon: string interning, arena allocator, error,
//! diagnostics, config TOML, animasi pipeline terminal, dan tipe nilai
//! logika 4-state (LogicVal/LogicVec).
//!
//! Crate pertama dalam migrasi monorepo (src/ → crates/). Semua crate lain
//! (ast, ir, parser, simulator, ...) bergantung pada mivon-core.

pub mod animasi;
pub mod arena;
pub mod audit;
pub mod checksum;
pub mod config;
pub mod diagnostics;
pub mod error;
pub mod intern;
pub mod logic;
pub mod template;

pub use intern::{init_string_table, Span, Symbol};
/// Tipe nilai logika inti — dipakai langsung (`mivon_core::LogicVec`) dan
/// di-reexport oleh `mivon-ir` agar `mivon_ir::LogicVec` tetap valid.
pub use logic::{LogicVal, LogicVec};

/// Fast content hashing (xxhash3) — dipakai cache/MICD/elaboration. Dipindah
/// dari src/cache/checksum.rs (crate 5/6 depend pada ini); cache re-export
/// dari sini agar `crate::cache::checksum::*` tetap valid.
pub use checksum::{
    checksum_fold, combine_checksum, compute_checksum, compute_file_checksum, compute_str_checksum,
};
