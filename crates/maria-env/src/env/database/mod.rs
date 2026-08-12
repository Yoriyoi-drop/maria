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
pub use micd::{project_id_for, open_database, default_database_root, database_root_for};
pub use symbol_db::{locate_symbol, symbol_names, symbol_count};
pub use graph_db::{def_of, deps_of, file_dep_count};
pub use metadata_db::{file_meta, recompiled_count, file_count};
pub use diagnostics_db::{diag_count, total_diag_count};
