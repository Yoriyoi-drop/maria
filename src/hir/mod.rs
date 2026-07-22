//! High-Level IR — sedang dalam migrasi dari src/ir/ ke struktur baru.
//!
//! Phase 3: HIR layer yang immutable dan cacheable.

pub mod builder;
pub mod hir;
pub mod lazy_elab;

pub use builder::HirBuilder;
pub use hir::*;
pub use lazy_elab::LazyElaborator;

// Re-export existing IR types for compatibility
pub use crate::ir::*;
