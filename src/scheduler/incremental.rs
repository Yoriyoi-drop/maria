//! Incremental Task Tracking.
//!
//! Melacak task mana yang perlu di-rebuild saat file berubah.
//! Menggunakan dirty flag propagation melalui dependency graph.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use super::dag::NodeId;

// ─── Incremental Tracker ───

/// Tracks which files/modules are dirty and need recompilation.
pub struct IncrementalTracker {
    /// File path → set of module nodes it contains
    file_modules: Mutex<HashMap<PathBuf, HashSet<NodeId>>>,
    /// Module node → set of file paths that contribute to it
    module_files: Mutex<HashMap<NodeId, HashSet<PathBuf>>>,
    /// Dirty nodes (need recompilation)
    dirty: Mutex<HashSet<NodeId>>,
    /// File checksums (path → checksum)
    checksums: Mutex<HashMap<PathBuf, u64>>,
    /// Total invalidations
    pub invalidations: AtomicUsize,
}

impl IncrementalTracker {
    pub fn new() -> Self {
        IncrementalTracker {
            file_modules: Mutex::new(HashMap::new()),
            module_files: Mutex::new(HashMap::new()),
            dirty: Mutex::new(HashSet::new()),
            checksums: Mutex::new(HashMap::new()),
            invalidations: AtomicUsize::new(0),
        }
    }

    /// Register a file and the modules it contains.
    pub fn register_file(&self, path: &Path, modules: Vec<NodeId>, checksum: u64) {
        let mut file_modules = self.file_modules.lock().unwrap();
        let mut module_files = self.module_files.lock().unwrap();

        let module_set: HashSet<NodeId> = modules.into_iter().collect();
        file_modules.insert(path.to_path_buf(), module_set.clone());

        for module in &module_set {
            module_files
                .entry(*module)
                .or_default()
                .insert(path.to_path_buf());
        }

        self.checksums
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), checksum);
    }

    /// Mark a file as changed → propagate dirty to dependent modules.
    pub fn mark_changed(&self, path: &Path) {
        self.invalidations.fetch_add(1, Ordering::Relaxed);

        let file_modules = self.file_modules.lock().unwrap();
        let mut dirty = self.dirty.lock().unwrap();

        if let Some(modules) = file_modules.get(path) {
            let mut worklist: VecDeque<NodeId> = modules.iter().copied().collect();
            let mut visited = HashSet::new();

            while let Some(node) = worklist.pop_front() {
                if !visited.insert(node) {
                    continue;
                }
                dirty.insert(node);
            }
        }
    }

    /// Get all dirty nodes (modules needing recompilation).
    pub fn take_dirty(&self) -> Vec<NodeId> {
        let mut dirty = self.dirty.lock().unwrap();
        dirty.drain().collect()
    }

    /// Is a specific node dirty?
    pub fn is_dirty(&self, node: NodeId) -> bool {
        self.dirty.lock().unwrap().contains(&node)
    }

    /// Number of dirty nodes.
    pub fn dirty_count(&self) -> usize {
        self.dirty.lock().unwrap().len()
    }

    /// Get files for a module.
    pub fn files_for_module(&self, module: NodeId) -> Vec<PathBuf> {
        self.module_files
            .lock()
            .unwrap()
            .get(&module)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    /// Get checksum for a file.
    pub fn checksum(&self, path: &Path) -> Option<u64> {
        self.checksums.lock().unwrap().get(path).copied()
    }
}

impl Default for IncrementalTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::work_stealing::Task;
    use crate::scheduler::dag::DependencyGraph;

    #[test]
    fn test_incremental_register() {
        let tracker = IncrementalTracker::new();
        let path = Path::new("/tmp/test.sv");
        let graph = DependencyGraph::new();
        let node = graph.add_node(Task::ParseFile("test.sv".into()));

        tracker.register_file(path, vec![node], 12345);
        assert_eq!(tracker.files_for_module(node), vec![path.to_path_buf()]);
    }

    #[test]
    fn test_incremental_mark_changed() {
        let tracker = IncrementalTracker::new();
        let path = Path::new("/tmp/dirty.sv");
        let graph = DependencyGraph::new();
        let node = graph.add_node(Task::ParseFile("dirty.sv".into()));

        tracker.register_file(path, vec![node], 100);
        assert!(!tracker.is_dirty(node));

        tracker.mark_changed(path);
        assert!(tracker.is_dirty(node));
    }

    #[test]
    fn test_incremental_take_dirty() {
        let tracker = IncrementalTracker::new();
        let path = Path::new("/tmp/take.sv");
        let graph = DependencyGraph::new();
        let node = graph.add_node(Task::ParseFile("take.sv".into()));

        tracker.register_file(path, vec![node], 100);
        tracker.mark_changed(path);

        let dirty = tracker.take_dirty();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0], node);

        // After taking, no more dirty
        assert_eq!(tracker.take_dirty().len(), 0);
    }
}
