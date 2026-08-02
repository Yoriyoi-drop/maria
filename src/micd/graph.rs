//! graph.mdb — dependency graph file-level (inti incremental compile).
//!
//! Menyimpan edge: file A bergantung pada file B (A menginstansiasi module
//! yang didefinisikan di B, atau import package dari B). Reverse edge
//! (dependents) diturunkan saat load. Saat B berubah, seluruh transitive
//! closure dependents B = set file yang perlu di-rebuild — tanpa scan
//! seluruh project.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Representasi persistent dependency graph.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileGraph {
    /// file → file yang menjadi dependensinya.
    deps: HashMap<PathBuf, Vec<PathBuf>>,
    /// Reverse index (dibangun saat load/save).
    dependents: HashMap<PathBuf, Vec<PathBuf>>,
    /// Reverse index perlu dibangun ulang (set_deps tidak rebuild per-call).
    #[serde(skip)]
    dirty_reverse: bool,
}

impl FileGraph {
    pub fn new() -> Self {
        FileGraph::default()
    }

    /// Set dependensi `file` (menimpa seluruh entry lama). Reverse index
    /// dibangun ulang secara lazy (lihat [`FileGraph::rebuild`]).
    pub fn set_deps(&mut self, file: PathBuf, deps: Vec<PathBuf>) {
        self.deps.insert(file, deps);
        self.dirty_reverse = true;
    }

    /// Tambah satu edge: `file` bergantung pada `dep`.
    pub fn add_dep(&mut self, file: PathBuf, dep: PathBuf) {
        if file == dep {
            return; // self-edge tidak bermakna
        }
        self.deps
            .entry(file)
            .or_default()
            .push(dep);
        self.dirty_reverse = true;
    }

    /// Bangun reverse index. O(total deps) — panggil SEKALI setelah batch
    /// set_deps, bukan per-call.
    pub fn rebuild(&mut self) {
        let mut rev: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        for (file, deps) in self.deps.iter() {
            for d in deps {
                let list = rev.entry(d.clone()).or_default();
                if !list.contains(file) {
                    list.push(file.clone());
                }
            }
        }
        self.dependents = rev;
        self.dirty_reverse = false;
    }

    /// Pastikan reverse index segar (lazy rebuild).
    fn ensure_reverse(&mut self) {
        if self.dirty_reverse {
            self.rebuild();
        }
    }

    /// Semua file yang terdampak bila `changed` berubah (transitive reverse
    /// closure, termasuk dirinya sendiri bila terdaftar).
    pub fn affected(&mut self, changed: &[PathBuf]) -> Vec<PathBuf> {
        self.ensure_reverse();
        let mut out = Vec::new();
        let mut visited = HashSet::new();
        let mut worklist: VecDeque<PathBuf> = changed.iter().cloned().collect();
        while let Some(cur) = worklist.pop_front() {
            if !visited.insert(cur.clone()) {
                continue;
            }
            out.push(cur.clone());
            if let Some(deps) = self.dependents.get(&cur) {
                for d in deps {
                    if !visited.contains(d) {
                        worklist.push_back(d.clone());
                    }
                }
            }
        }
        out
    }

    /// Dependensi langsung sebuah file.
    pub fn deps_of(&self, file: &Path) -> Vec<PathBuf> {
        self.deps
            .get(file)
            .cloned()
            .unwrap_or_default()
    }

    /// Banyak node terdaftar.
    pub fn len(&self) -> usize {
        self.deps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.deps.is_empty()
    }

    /// Semua (file, deps).
    pub fn iter(&self) -> impl Iterator<Item = (&PathBuf, &Vec<PathBuf>)> {
        self.deps.iter()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_affected_chain() {
        let mut g = FileGraph::new();
        let defines = PathBuf::from("defines.svh");
        let uart = PathBuf::from("uart.sv");
        let cpu = PathBuf::from("cpu.sv");
        let dma = PathBuf::from("dma.sv");
        g.set_deps(uart.clone(), vec![defines.clone()]);
        g.set_deps(cpu.clone(), vec![uart.clone()]);
        g.set_deps(dma.clone(), vec![defines.clone()]);

        let affected = g.affected(&[defines.clone()]);
        assert!(affected.contains(&defines));
        assert!(affected.contains(&uart));
        assert!(affected.contains(&dma));
        // cpu → uart → defines: terdampak transitif
        assert!(affected.contains(&cpu));

        let affected_uart = g.affected(&[uart.clone()]);
        assert!(affected_uart.contains(&uart));
        assert!(affected_uart.contains(&cpu));
    }

    #[test]
    fn test_self_edge_ignored() {
        let mut g = FileGraph::new();
        let f = PathBuf::from("a.sv");
        g.add_dep(f.clone(), f.clone());
        assert!(g.is_empty());
        assert!(g.deps_of(&f).is_empty());
    }

    #[test]
    fn test_serialize_roundtrip() {
        let mut g = FileGraph::new();
        g.set_deps(PathBuf::from("b.sv"), vec![PathBuf::from("a.sv")]);
        g.rebuild();
        let bytes = bincode::serialize(&g).unwrap();
        let mut g2: FileGraph = bincode::deserialize(&bytes).unwrap();
        assert_eq!(g2.deps_of(&Path::new("b.sv")), vec![PathBuf::from("a.sv")]);
        assert_eq!(g2.len(), 1);
        // affected (setelah deserialize, lazy rebuild otomatis)
        let a = g2.affected(&[PathBuf::from("a.sv")]);
        assert!(a.contains(&PathBuf::from("b.sv")));
    }
}
