//! Database context — semua persistent storage (khusus MICD).
//!
//! Compiler TIDAK pernah membuka file database secara langsung; ia meminta
//! melalui DatabaseContext. Database tidak boleh mengetahui Parser.

mod database;
mod diagnostics_db;
mod graph_db;
mod metadata_db;
mod micd;
mod symbol_db;

pub use database::DatabaseContext;
pub use diagnostics_db::{diag_count, total_diag_count};
pub use graph_db::{def_of, deps_of, file_dep_count};
pub use metadata_db::{file_count, file_meta, recompiled_count};
pub use micd::{database_root_for, default_database_root, open_database, project_id_for};
pub use symbol_db::{locate_symbol, symbol_count, symbol_names};
