//! AST Cache — file-level AST caching by content checksum.
//!
//! Setiap file yang di-parse di-cache berdasarkan checksum kontennya.
//! Jika file tidak berubah, AST dari cache langsung dipakai tanpa re-parse.

use std::path::PathBuf;

use super::cache_manager::CacheManager;
use super::checksum::compute_checksum;
use crate::intern::Symbol;

/// AST cache wrapper untuk file-level caching.
pub struct AstCache<'a> {
    manager: &'a CacheManager,
}

impl<'a> AstCache<'a> {
    pub fn new(manager: &'a CacheManager) -> Self {
        AstCache { manager }
    }

    /// Get cached AST for a file. Returns (checksum, Option<ast_bytes>).
    pub fn get(&self, path: &std::path::Path, content: &str) -> Option<(u64, Vec<u8>)> {
        let checksum = compute_checksum(content.as_bytes());
        // Check if file's content checksum has been registered
        if !self.manager.has_file(path) {
            return None;
        }
        self.manager.get_ast(checksum).map(|ast| (checksum, ast))
    }

    /// Insert AST into cache.
    pub fn insert(&self, path: &std::path::Path, content: &str, ast: Vec<u8>) {
        let content_checksum = self.manager.register_file(path, content.as_bytes());
        let size = ast.len() as u64;
        self.manager.cache_ast(content_checksum, ast, size);
    }

    /// Check if file has changed since last cache.
    pub fn is_fresh(&self, path: &std::path::Path, content: &str) -> bool {
        !self.manager.file_changed(path, content.as_bytes())
    }

    /// Get cache statistics.
    pub fn hit_rate(&self) -> f64 {
        self.manager.ast_cache.hit_rate()
    }
}

/// Cached AST data for a parsed file.
#[derive(Debug, Clone)]
pub struct CachedAst {
    pub path: PathBuf,
    pub checksum: u64,
    pub module_names: Vec<Symbol>,
    pub package_names: Vec<Symbol>,
    pub ast_data: Vec<u8>,
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheManager;

    #[test]
    fn test_ast_cache_get_insert() {
        let cm = CacheManager::new();
        let cache = AstCache::new(&cm);
        let path = std::path::Path::new("/tmp/test_ast.sv");
        let content = "module test; endmodule";

        assert!(cache.get(path, content).is_none());

        cache.insert(path, content, vec![1, 2, 3]);
        let result = cache.get(path, content);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, vec![1, 2, 3]);
    }

    #[test]
    fn test_ast_cache_freshness() {
        let cm = CacheManager::new();
        let cache = AstCache::new(&cm);
        let path = std::path::Path::new("/tmp/test_fresh.sv");
        let content = "module test; endmodule";

        cache.insert(path, content, vec![1, 2, 3]);
        assert!(cache.is_fresh(path, content));
        assert!(!cache.is_fresh(path, "module changed; endmodule"));
    }
}
