//! COMP-20: Incremental Compilation — change tracking and partial recompilation.
//!
//! Tracks which files have changed and determines which modules
//! need recompilation. Uses dependency graph to propagate changes.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// File metadata for change detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub path: String,
    pub content_hash: u64,
    pub last_modified: u64,
    pub size: u64,
}

/// Module dependency.
#[derive(Debug, Clone)]
pub struct ModuleDep {
    pub name: String,
    pub source_file: String,
    pub depends_on: Vec<String>,
}

/// Change detection result.
#[derive(Debug, Clone)]
pub struct ChangeSet {
    pub changed_files: Vec<String>,
    pub affected_modules: Vec<String>,
    pub needs_recompile: Vec<String>,
    pub skipped_modules: Vec<String>,
}

/// Incremental compiler tracker.
pub struct IncrementalTracker {
    files: HashMap<String, FileRecord>,
    modules: HashMap<String, ModuleDep>,
}

impl IncrementalTracker {
    pub fn new() -> Self {
        IncrementalTracker {
            files: HashMap::new(),
            modules: HashMap::new(),
        }
    }

    /// Record a file's metadata.
    pub fn record_file(&mut self, record: FileRecord) {
        self.files.insert(record.path.clone(), record);
    }

    /// Record a module and its dependencies.
    pub fn record_module(&mut self, module: ModuleDep) {
        self.modules.insert(module.name.clone(), module);
    }

    /// Detect which files have changed since last check.
    pub fn detect_changes(&self, current_files: &HashMap<String, FileRecord>) -> Vec<String> {
        let mut changed = Vec::new();
        for (path, current) in current_files {
            match self.files.get(path) {
                Some(prev) => {
                    if prev.content_hash != current.content_hash
                        || prev.size != current.size
                        || prev.last_modified != current.last_modified
                    {
                        changed.push(path.clone());
                    }
                }
                None => changed.push(path.clone()),
            }
        }
        changed
    }

    /// Find all modules affected by file changes (transitive closure).
    pub fn affected_modules(&self, changed_files: &[String]) -> Vec<String> {
        let file_to_module: HashMap<&str, &str> = self
            .modules
            .iter()
            .map(|(name, dep)| (dep.source_file.as_str(), name.as_str()))
            .collect();

        // Directly affected modules
        let mut affected: HashSet<String> = HashSet::new();
        for file in changed_files {
            if let Some(&module) = file_to_module.get(file.as_str()) {
                affected.insert(module.to_string());
            }
        }

        // Propagate: find modules that depend on affected modules
        let mut queue: VecDeque<String> = affected.iter().cloned().collect();
        let mut visited: HashSet<String> = affected.clone();

        while let Some(module) = queue.pop_front() {
            for (name, dep) in &self.modules {
                if dep.depends_on.contains(&module) && !visited.contains(name) {
                    visited.insert(name.clone());
                    queue.push_back(name.clone());
                }
            }
        }

        visited.into_iter().collect()
    }

    /// Generate full change set: files → modules → recompile list.
    pub fn compute_change_set(&self, current_files: &HashMap<String, FileRecord>) -> ChangeSet {
        let changed = self.detect_changes(current_files);
        let affected = self.affected_modules(&changed);

        let all_modules: Vec<String> = self.modules.keys().cloned().collect();
        let needs_recompile: Vec<String> = affected
            .iter()
            .filter(|m| self.modules.contains_key(*m))
            .cloned()
            .collect();
        let skipped: Vec<String> = all_modules
            .iter()
            .filter(|m| !needs_recompile.contains(m))
            .cloned()
            .collect();

        ChangeSet {
            changed_files: changed,
            affected_modules: affected,
            needs_recompile,
            skipped_modules: skipped,
        }
    }

    /// Update file records after recompilation.
    pub fn update_files(&mut self, records: Vec<FileRecord>) {
        for record in records {
            self.files.insert(record.path.clone(), record);
        }
    }

    /// Summary.
    pub fn summary(&self) -> String {
        format!(
            "IncrementalTracker: {} files tracked, {} modules",
            self.files.len(),
            self.modules.len()
        )
    }
}

impl Default for IncrementalTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_file(path: &str, hash: u64) -> FileRecord {
        FileRecord {
            path: path.to_string(),
            content_hash: hash,
            last_modified: 1000,
            size: 100,
        }
    }

    #[test]
    fn test_detect_changes() {
        let mut tracker = IncrementalTracker::new();
        tracker.record_file(make_file("a.sv", 100));
        tracker.record_file(make_file("b.sv", 200));

        let mut current = HashMap::new();
        current.insert("a.sv".to_string(), make_file("a.sv", 100)); // same
        current.insert("b.sv".to_string(), make_file("b.sv", 999)); // changed
        current.insert("c.sv".to_string(), make_file("c.sv", 300)); // new

        let changes = tracker.detect_changes(&current);
        assert!(changes.contains(&"b.sv".to_string()));
        assert!(changes.contains(&"c.sv".to_string()));
        assert!(!changes.contains(&"a.sv".to_string()));
    }

    #[test]
    fn test_affected_modules() {
        let mut tracker = IncrementalTracker::new();
        tracker.record_module(ModuleDep {
            name: "base".into(),
            source_file: "base.sv".into(),
            depends_on: vec![],
        });
        tracker.record_module(ModuleDep {
            name: "top".into(),
            source_file: "top.sv".into(),
            depends_on: vec!["base".into()],
        });
        tracker.record_module(ModuleDep {
            name: "other".into(),
            source_file: "other.sv".into(),
            depends_on: vec![],
        });

        let affected = tracker.affected_modules(&["base.sv".to_string()]);
        assert!(affected.contains(&"base".to_string()));
        assert!(affected.contains(&"top".to_string()));
    }

    #[test]
    fn test_change_set() {
        let mut tracker = IncrementalTracker::new();
        tracker.record_file(make_file("a.sv", 100));
        tracker.record_module(ModuleDep {
            name: "mod_a".into(),
            source_file: "a.sv".into(),
            depends_on: vec![],
        });

        let mut current = HashMap::new();
        current.insert("a.sv".to_string(), make_file("a.sv", 999));

        let cs = tracker.compute_change_set(&current);
        assert!(!cs.changed_files.is_empty());
        assert!(cs.needs_recompile.contains(&"mod_a".to_string()));
    }

    #[test]
    fn test_summary() {
        let tracker = IncrementalTracker::new();
        assert!(tracker.summary().contains("0 files"));
    }
}
