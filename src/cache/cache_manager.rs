//! CacheManager — unified cache for AST, HIR, includes, macros.
//!
//! Menyediakan content-based caching dengan LRU eviction.
//! Semua cache entries di-index oleh checksum → invalidation otomatis saat file berubah.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use dashmap::DashMap;

use super::checksum::compute_checksum;
use crate::intern::Symbol;

// ─── Cache Key ───

/// Cache key untuk semua jenis cache entries.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum CacheKey {
    /// File content hash (xxhash3-64)
    FileContent(u64),
    /// File path (interned)
    FilePath(Symbol),
    /// Module name + parameter signature
    Module {
        name: Symbol,
        param_hash: u64,
        dependency_hash: u64,
    },
    /// Package name
    Package(Symbol),
    /// Macro invocation: name + argument hash
    Macro {
        name: Symbol,
        arg_hash: u64,
        definition_hash: u64,
    },
    /// Include: resolved path + content hash
    Include {
        resolved_path: Symbol,
        content_hash: u64,
    },
}

// ─── Cache Entry ───

/// Single cache entry with metadata for LRU eviction.
#[derive(Debug)]
pub struct CacheEntry<V> {
    pub value: V,
    pub size: u64,
    pub created: Instant,
    pub accessed: AtomicU64,
    pub checksum: u64,
}

impl<V> CacheEntry<V> {
    pub fn new(value: V, size: u64, checksum: u64) -> Self {
        CacheEntry {
            value,
            size,
            created: Instant::now(),
            accessed: AtomicU64::new(0),
            checksum,
        }
    }

    pub fn touch(&self) {
        let now = Instant::now()
            .duration_since(Instant::now() - std::time::Duration::from_secs(1))
            .as_nanos() as u64;
        self.accessed.store(now, Ordering::Relaxed);
    }
}

// ─── Cache Store ───

/// Thread-safe cache store with LRU eviction.
pub struct CacheStore<V> {
    primary: DashMap<CacheKey, CacheEntry<V>>,
    budget: AtomicU64,
    used: AtomicU64,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl<V> CacheStore<V> {
    pub fn new(memory_budget_bytes: u64) -> Self {
        CacheStore {
            primary: DashMap::new(),
            budget: AtomicU64::new(memory_budget_bytes),
            used: AtomicU64::new(0),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        }
    }

    pub fn get(
        &self,
        key: &CacheKey,
    ) -> Option<dashmap::mapref::one::Ref<CacheKey, CacheEntry<V>>> {
        let entry = self.primary.get(key)?;
        entry.touch();
        self.hits.fetch_add(1, Ordering::Relaxed);
        Some(entry)
    }

    pub fn insert(&self, key: CacheKey, value: V, size: u64, checksum: u64) {
        // Check budget
        let current_used = self.used.load(Ordering::Relaxed);
        let budget = self.budget.load(Ordering::Relaxed);
        if current_used + size > budget {
            self.evict_lru(size);
        }

        let entry = CacheEntry::new(value, size, checksum);
        self.used.fetch_add(size, Ordering::Relaxed);
        self.primary.insert(key, entry);
    }

    pub fn remove(&self, key: &CacheKey) -> Option<u64> {
        let entry = self.primary.remove(key).map(|(_, e)| e.size);
        if let Some(size) = entry {
            self.used.fetch_sub(size, Ordering::Relaxed);
        }
        entry
    }

    pub fn contains(&self, key: &CacheKey) -> bool {
        self.primary.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.primary.len()
    }

    pub fn memory_used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }

    pub fn hit_rate(&self) -> f64 {
        let hits = self.hits.load(Ordering::Relaxed) as f64;
        let misses = self.misses.load(Ordering::Relaxed) as f64;
        let total = hits + misses;
        if total == 0.0 {
            0.0
        } else {
            hits / total
        }
    }

    pub fn clear(&self) {
        self.primary.clear();
        self.used.store(0, Ordering::Relaxed);
    }

    fn evict_lru(&self, needed: u64) {
        // Simple eviction: remove oldest entries until we have enough space
        let budget = self.budget.load(Ordering::Relaxed);
        let current = self.used.load(Ordering::Relaxed);
        let mut to_remove = Vec::new();

        for entry in self.primary.iter() {
            if current - (to_remove.len() as u64 * entry.value().size) + needed <= budget {
                break;
            }
            to_remove.push(entry.key().clone());
        }

        for key in to_remove {
            self.remove(&key);
        }
    }
}

impl<V> Default for CacheStore<V> {
    fn default() -> Self {
        // Default: 256MB budget
        Self::new(256 * 1024 * 1024)
    }
}

// ─── Cache Manager ───

/// Unified cache manager — single entry point untuk semua caching.
pub struct CacheManager {
    /// File content checksums
    file_checksums: DashMap<PathBuf, u64>,
    /// AST cache (file content checksum → AST)
    pub ast_cache: CacheStore<Vec<u8>>,
    /// HIR cache (module key → HIR)
    pub hir_cache: CacheStore<Vec<u8>>,
    /// Include cache (path → content)
    pub include_cache: CacheStore<String>,
    /// Macro cache (macro key → expanded output)
    pub macro_cache: CacheStore<String>,
    /// Dependency cache (file → dependency list)
    pub dep_cache: CacheStore<Vec<PathBuf>>,
    /// Global stats
    pub total_invalidations: AtomicUsize,
}

impl CacheManager {
    /// Create a new cache manager with default budgets.
    pub fn new() -> Self {
        Self::with_budgets(
            64 * 1024 * 1024,
            64 * 1024 * 1024,
            32 * 1024 * 1024,
            16 * 1024 * 1024,
        )
    }

    pub fn with_budgets(
        ast_budget: u64,
        hir_budget: u64,
        include_budget: u64,
        macro_budget: u64,
    ) -> Self {
        CacheManager {
            file_checksums: DashMap::new(),
            ast_cache: CacheStore::new(ast_budget),
            hir_cache: CacheStore::new(hir_budget),
            include_cache: CacheStore::new(include_budget),
            macro_cache: CacheStore::new(macro_budget),
            dep_cache: CacheStore::new(16 * 1024 * 1024),
            total_invalidations: AtomicUsize::new(0),
        }
    }

    /// Check if a file has been registered.
    pub fn has_file(&self, path: &std::path::Path) -> bool {
        self.file_checksums.contains_key(path)
    }

    /// Register a file and compute its checksum.
    pub fn register_file(&self, path: &std::path::Path, content: &[u8]) -> u64 {
        let checksum = compute_checksum(content);
        self.file_checksums.insert(path.to_path_buf(), checksum);
        checksum
    }

    /// Check if file has changed since last registration.
    pub fn file_changed(&self, path: &std::path::Path, content: &[u8]) -> bool {
        let new_hash = compute_checksum(content);
        match self.file_checksums.get(path) {
            Some(old) => *old != new_hash,
            None => true, // Not registered = changed
        }
    }

    /// Called when a file changes → invalidate all dependent cache entries.
    pub fn on_file_changed(&self, path: &std::path::Path) {
        // Remove file's AST/HIR cache entries
        if let Some((_path, old_checksum)) = self.file_checksums.remove(path) {
            self.total_invalidations.fetch_add(1, Ordering::Relaxed);

            // Invalidate AST cache entries that reference this checksum
            let keys_to_remove: Vec<CacheKey> = self
                .ast_cache
                .primary
                .iter()
                .filter(|e| e.value().checksum == old_checksum)
                .map(|e| e.key().clone())
                .collect();
            for key in keys_to_remove {
                self.ast_cache.remove(&key);
            }

            // Invalidate HIR cache
            let keys_to_remove: Vec<CacheKey> = self
                .hir_cache
                .primary
                .iter()
                .filter(|e| e.value().checksum == old_checksum)
                .map(|e| e.key().clone())
                .collect();
            for key in keys_to_remove {
                self.hir_cache.remove(&key);
            }
        }
    }

    /// Get cached AST by file checksum.
    pub fn get_ast(&self, checksum: u64) -> Option<Vec<u8>> {
        let key = CacheKey::FileContent(checksum);
        self.ast_cache.get(&key).map(|e| e.value.clone())
    }

    /// Cache AST result.
    pub fn cache_ast(&self, checksum: u64, ast: Vec<u8>, size: u64) {
        let key = CacheKey::FileContent(checksum);
        self.ast_cache.insert(key, ast, size, checksum);
    }

    /// Get cached HIR by module key.
    pub fn get_hir(&self, name: Symbol, param_hash: u64, dep_hash: u64) -> Option<Vec<u8>> {
        let key = CacheKey::Module {
            name,
            param_hash,
            dependency_hash: dep_hash,
        };
        self.hir_cache.get(&key).map(|e| e.value.clone())
    }

    /// Cache HIR result.
    pub fn cache_hir(&self, name: Symbol, param_hash: u64, dep_hash: u64, hir: Vec<u8>, size: u64) {
        let key = CacheKey::Module {
            name,
            param_hash,
            dependency_hash: dep_hash,
        };
        self.hir_cache.insert(key, hir, size, 0);
    }

    /// Get cached include content.
    pub fn get_include(&self, path: Symbol, content_hash: u64) -> Option<String> {
        let key = CacheKey::Include {
            resolved_path: path,
            content_hash,
        };
        self.include_cache.get(&key).map(|e| e.value.clone())
    }

    /// Cache include content.
    pub fn cache_include(&self, path: Symbol, content: String, content_hash: u64) {
        let size = content.len() as u64;
        let key = CacheKey::Include {
            resolved_path: path,
            content_hash,
        };
        self.include_cache.insert(key, content, size, content_hash);
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            ast_entries: self.ast_cache.len(),
            ast_memory: self.ast_cache.memory_used(),
            ast_hit_rate: self.ast_cache.hit_rate(),
            hir_entries: self.hir_cache.len(),
            hir_memory: self.hir_cache.memory_used(),
            hir_hit_rate: self.hir_cache.hit_rate(),
            include_entries: self.include_cache.len(),
            include_memory: self.include_cache.memory_used(),
            include_hit_rate: self.include_cache.hit_rate(),
            macro_entries: self.macro_cache.len(),
            macro_memory: self.macro_cache.memory_used(),
            macro_hit_rate: self.macro_cache.hit_rate(),
            total_invalidations: self.total_invalidations.load(Ordering::Relaxed),
        }
    }

    /// Clear all caches.
    pub fn clear(&self) {
        self.ast_cache.clear();
        self.hir_cache.clear();
        self.include_cache.clear();
        self.macro_cache.clear();
        self.dep_cache.clear();
        self.file_checksums.clear();
    }
}

impl Default for CacheManager {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Stats ───

/// Cache statistics for profiling.
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub ast_entries: usize,
    pub ast_memory: u64,
    pub ast_hit_rate: f64,
    pub hir_entries: usize,
    pub hir_memory: u64,
    pub hir_hit_rate: f64,
    pub include_entries: usize,
    pub include_memory: u64,
    pub include_hit_rate: f64,
    pub macro_entries: usize,
    pub macro_memory: u64,
    pub macro_hit_rate: f64,
    pub total_invalidations: usize,
}

impl std::fmt::Display for CacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Cache Statistics:")?;
        writeln!(
            f,
            "  AST:   {} entries, {:.1}KB, hit rate {:.1}%",
            self.ast_entries,
            self.ast_memory as f64 / 1024.0,
            self.ast_hit_rate * 100.0
        )?;
        writeln!(
            f,
            "  HIR:   {} entries, {:.1}KB, hit rate {:.1}%",
            self.hir_entries,
            self.hir_memory as f64 / 1024.0,
            self.hir_hit_rate * 100.0
        )?;
        writeln!(
            f,
            "  Inc:   {} entries, {:.1}KB, hit rate {:.1}%",
            self.include_entries,
            self.include_memory as f64 / 1024.0,
            self.include_hit_rate * 100.0
        )?;
        writeln!(
            f,
            "  Macro: {} entries, {:.1}KB, hit rate {:.1}%",
            self.macro_entries,
            self.macro_memory as f64 / 1024.0,
            self.macro_hit_rate * 100.0
        )?;
        writeln!(f, "  Total invalidations: {}", self.total_invalidations)?;
        Ok(())
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_manager_register_and_change() {
        let cm = CacheManager::new();
        let path = std::path::Path::new("/tmp/test_cache.sv");
        let content = b"module test; endmodule";

        cm.register_file(path, content);
        assert!(!cm.file_changed(path, content));
        assert!(cm.file_changed(path, b"module test; endmodule_v2"));
    }

    #[test]
    fn test_cache_ast() {
        let cm = CacheManager::new();
        let checksum = 12345u64;
        let ast = vec![1u8, 2, 3, 4];
        cm.cache_ast(checksum, ast.clone(), 4);
        assert_eq!(cm.get_ast(checksum), Some(ast));
    }

    #[test]
    fn test_cache_invalidation() {
        let cm = CacheManager::new();
        let path = std::path::Path::new("/tmp/test_inv.sv");
        let content = b"module test; endmodule";

        let checksum = cm.register_file(path, content);
        cm.cache_ast(checksum, vec![1, 2, 3], 3);
        assert!(cm.get_ast(checksum).is_some());

        cm.on_file_changed(path);
        assert!(cm.get_ast(checksum).is_none());
    }

    #[test]
    fn test_cache_stats() {
        let cm = CacheManager::new();
        let stats = cm.stats();
        assert_eq!(stats.ast_entries, 0);
        assert_eq!(stats.total_invalidations, 0);
    }

    #[test]
    fn test_cache_store_eviction() {
        let store = CacheStore::<Vec<u8>>::new(100); // 100 bytes budget
        store.insert(CacheKey::FileContent(1), vec![0u8; 60], 60, 0);
        store.insert(CacheKey::FileContent(2), vec![0u8; 60], 60, 0);
        // Should have evicted first entry
        assert!(store.memory_used() <= 100);
    }
}
