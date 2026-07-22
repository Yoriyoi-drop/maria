//! Cache — content-based caching untuk AST, HIR, includes, macros.
//!
//! Phase 2 implementation. Menyediakan invalidation otomatis berbasis checksum.

pub mod ast_cache;
pub mod cache_manager;
pub mod checksum;
pub mod dep_cache;
pub mod hir_cache;

pub use ast_cache::AstCache;
pub use cache_manager::{CacheEntry, CacheKey, CacheManager, CacheStats, CacheStore};
pub use checksum::{compute_checksum, compute_file_checksum, compute_str_checksum};
pub use dep_cache::DepCache;
pub use hir_cache::HirCache;
