//! Cache — content-based caching untuk AST, HIR, includes, macros.
//!
//! Phase 2 implementation. Menyediakan invalidation otomatis berbasis checksum.

pub mod ast_cache;
pub mod cache_manager;
pub mod dep_cache;
pub mod hir_cache;
pub mod remote;

pub use ast_cache::AstCache;
pub use cache_manager::{CacheEntry, CacheKey, CacheManager, CacheStats, CacheStore, RemoteSyncMode};
pub use dep_cache::DepCache;
pub use hir_cache::HirCache;
pub use remote::{FilesystemCache, RemoteCacheBackend, RemoteCacheStats};

// checksum.rs pindah ke maria-core (crate 5/6 depend padanya). Re-export agar
// `crate::cache::checksum::*` dan `crate::cache::compute_checksum` tetap valid.
pub use maria_core::checksum;
pub use maria_core::checksum::{checksum_fold, combine_checksum, compute_checksum, compute_file_checksum, compute_str_checksum};
