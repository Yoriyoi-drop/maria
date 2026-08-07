use crate::cache::{CacheManager, CacheStats};
use crate::scheduler::incremental::IncrementalTracker;
use std::path::PathBuf;

/// CacheContext — pintu cache content-based + tracker incremental.
pub struct CacheContext {
    pub manager: CacheManager,
    pub incremental: IncrementalTracker,
    pub root: PathBuf,
}

impl std::fmt::Debug for CacheContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheContext")
            .field("root", &self.root)
            .field("stats", &self.stats())
            .finish()
    }
}

impl CacheContext {
    pub fn new(root: PathBuf) -> Self {
        CacheContext {
            manager: CacheManager::new(),
            incremental: IncrementalTracker::new(),
            root,
        }
    }

    pub fn register_file(&self, path: &std::path::Path, bytes: &[u8]) {
        self.manager.register_file(path, bytes);
    }

    pub fn mark_changed(&self, path: &std::path::Path) {
        self.manager.on_file_changed(path);
        self.incremental.mark_changed(path);
    }

    pub fn stats(&self) -> CacheStats {
        self.manager.stats()
    }

    pub fn clear(&self) {
        self.manager.clear();
    }

    /// Jumlah entry AST di cache.
    pub fn ast_entries(&self) -> usize {
        self.stats().ast_entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_context() {
        let ctx = CacheContext::new(PathBuf::from("/tmp/maria-cache-test"));
        ctx.register_file(std::path::Path::new("a.sv"), b"module a; endmodule");
        assert_eq!(ctx.ast_entries(), 0); // terisi saat compile, bukan register
        ctx.clear();
    }
}
