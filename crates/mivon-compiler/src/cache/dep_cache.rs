//! Dependency Cache — tracks file → dependency relationships.
//!
//! Menyimpan hasil dependency scan sehingga tidak perlu scan ulang
//! saat file tidak berubah.

use std::path::{Path, PathBuf};

use dashmap::DashMap;

use super::checksum::compute_checksum;

/// Cached dependency information for a single file.
#[derive(Debug, Clone)]
pub struct DepEntry {
    pub checksum: u64,
    pub depends_on: Vec<PathBuf>,
    pub depended_by: Vec<PathBuf>,
}

/// Thread-safe dependency cache.
pub struct DepCache {
    entries: DashMap<PathBuf, DepEntry>,
}

impl DepCache {
    pub fn new() -> Self {
        DepCache {
            entries: DashMap::new(),
        }
    }

    /// Register a file's dependencies.
    pub fn register(&self, path: &Path, content: &[u8], depends_on: Vec<PathBuf>) {
        let checksum = compute_checksum(content);
        let entry = DepEntry {
            checksum,
            depends_on,
            depended_by: Vec::new(),
        };
        self.entries.insert(path.to_path_buf(), entry);
    }

    /// Check if file needs dependency re-scan.
    pub fn needs_rescan(&self, path: &Path, content: &[u8]) -> bool {
        let new_checksum = compute_checksum(content);
        match self.entries.get(path) {
            Some(entry) => entry.checksum != new_checksum,
            None => true,
        }
    }

    /// Get dependencies for a file.
    pub fn get_dependencies(&self, path: &Path) -> Option<Vec<PathBuf>> {
        self.entries.get(path).map(|e| e.depends_on.clone())
    }

    /// Get reverse dependencies (files that depend on this file).
    pub fn get_reverse_dependencies(&self, path: &Path) -> Option<Vec<PathBuf>> {
        self.entries.get(path).map(|e| e.depended_by.clone())
    }

    /// Mark a file as depending on another.
    pub fn add_reverse_dep(&self, path: &Path, depended_by: &Path) {
        if let Some(mut entry) = self.entries.get_mut(path) {
            if !entry.depended_by.contains(&depended_by.to_path_buf()) {
                entry.depended_by.push(depended_by.to_path_buf());
            }
        }
    }

    /// Get all files that need recompilation when a file changes.
    pub fn get_affected_files(&self, changed: &Path) -> Vec<PathBuf> {
        let mut affected = Vec::new();
        let mut worklist = vec![changed.to_path_buf()];
        let mut visited = std::collections::HashSet::new();

        while let Some(current) = worklist.pop() {
            if !visited.insert(current.clone()) {
                continue;
            }
            affected.push(current.clone());

            // Add reverse dependencies to worklist
            if let Some(deps) = self.get_reverse_dependencies(&current) {
                for dep in deps {
                    if !visited.contains(&dep) {
                        worklist.push(dep);
                    }
                }
            }
        }

        affected
    }

    /// Clear all entries.
    pub fn clear(&self) {
        self.entries.clear();
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for DepCache {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dep_cache_register() {
        let cache = DepCache::new();
        let path = Path::new("/tmp/a.sv");
        let content = b"`include \"b.sv\"";

        cache.register(path, content, vec![PathBuf::from("/tmp/b.sv")]);

        assert!(!cache.needs_rescan(path, content));
        assert!(cache.needs_rescan(path, b"changed content"));
    }

    #[test]
    fn test_dep_cache_reverse() {
        let cache = DepCache::new();
        let a = Path::new("/tmp/a.sv");
        let b = Path::new("/tmp/b.sv");

        cache.register(a, b"module a; endmodule", vec![]);
        cache.register(b, b"module b; endmodule", vec![]);
        cache.add_reverse_dep(a, b);

        let rev = cache.get_reverse_dependencies(a).unwrap();
        assert_eq!(rev, vec![b.to_path_buf()]);
    }

    #[test]
    fn test_dep_cache_affected() {
        let cache = DepCache::new();
        let a = Path::new("/tmp/a.sv");
        let b = Path::new("/tmp/b.sv");
        let c = Path::new("/tmp/c.sv");

        cache.register(a, b"a", vec![]);
        cache.register(b, b"b", vec![]);
        cache.register(c, b"c", vec![]);
        cache.add_reverse_dep(a, b);
        cache.add_reverse_dep(b, c);

        let affected = cache.get_affected_files(a);
        assert!(affected.contains(&a.to_path_buf()));
        assert!(affected.contains(&b.to_path_buf()));
        assert!(affected.contains(&c.to_path_buf()));
    }
}
