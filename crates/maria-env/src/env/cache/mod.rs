//! Cache context — content-based caching (AST/HIR/includes/makro).
//!
//! Mengatur token cache, AST cache, HIR cache, dependency cache. Cache TIDAK
//! boleh memanggil Compiler (dependency rule satu arah).

mod artifact;
mod cache;
mod fingerprint;
mod incremental;

pub use artifact::ArtifactPaths;
pub use cache::CacheContext;
pub use fingerprint::Fingerprint;
pub use incremental::IncrementalHandle;
