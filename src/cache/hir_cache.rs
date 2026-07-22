//! HIR Cache — module-level HIR caching.
//!
//! Setiap module yang di-elaborate di-cache berdasarkan (name, param_hash, dep_hash).
//! Incremental: hanya re-elaborate jika parameter atau dependency berubah.

use super::cache_manager::CacheManager;
use crate::intern::Symbol;

/// HIR cache wrapper untuk module-level caching.
pub struct HirCache<'a> {
    manager: &'a CacheManager,
}

impl<'a> HirCache<'a> {
    pub fn new(manager: &'a CacheManager) -> Self {
        HirCache { manager }
    }

    /// Get cached HIR for a module.
    pub fn get(&self, name: Symbol, param_hash: u64, dep_hash: u64) -> Option<Vec<u8>> {
        self.manager.get_hir(name, param_hash, dep_hash)
    }

    /// Insert HIR into cache.
    pub fn insert(&self, name: Symbol, param_hash: u64, dep_hash: u64, hir: Vec<u8>) {
        let size = hir.len() as u64;
        self.manager
            .cache_hir(name, param_hash, dep_hash, hir, size);
    }

    /// Check if a module needs re-elaboration.
    pub fn needs_reelaboration(&self, name: Symbol, param_hash: u64, dep_hash: u64) -> bool {
        self.get(name, param_hash, dep_hash).is_none()
    }

    /// Get cache hit rate.
    pub fn hit_rate(&self) -> f64 {
        self.manager.hir_cache.hit_rate()
    }

    /// Invalidate all HIR entries (full rebuild).
    pub fn invalidate_all(&self) {
        self.manager.hir_cache.clear();
    }
}

/// Cached HIR data for an elaborated module.
#[derive(Debug, Clone)]
pub struct CachedHir {
    pub name: Symbol,
    pub param_hash: u64,
    pub dep_hash: u64,
    pub hir_data: Vec<u8>,
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheManager;

    #[test]
    fn test_hir_cache_basic() {
        let cm = CacheManager::new();
        let cache = HirCache::new(&cm);
        let name = Symbol::intern("test_module");

        assert!(cache.get(name, 0, 0).is_none());

        cache.insert(name, 0, 0, vec![1, 2, 3]);
        assert_eq!(cache.get(name, 0, 0), Some(vec![1, 2, 3]));
    }

    #[test]
    fn test_hir_cache_different_params() {
        let cm = CacheManager::new();
        let cache = HirCache::new(&cm);
        let name = Symbol::intern("param_module");

        cache.insert(name, 100, 0, vec![1]);
        cache.insert(name, 200, 0, vec![2]);

        assert_eq!(cache.get(name, 100, 0), Some(vec![1]));
        assert_eq!(cache.get(name, 200, 0), Some(vec![2]));
        assert!(cache.get(name, 300, 0).is_none());
    }

    #[test]
    fn test_hir_cache_invalidate() {
        let cm = CacheManager::new();
        let cache = HirCache::new(&cm);
        let name = Symbol::intern("inv_module");

        cache.insert(name, 0, 0, vec![1, 2, 3]);
        assert!(cache.get(name, 0, 0).is_some());

        cache.invalidate_all();
        assert!(cache.get(name, 0, 0).is_none());
    }
}
