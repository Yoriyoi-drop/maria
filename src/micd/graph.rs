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
    /// Simbol yang digunakan tiap file (Kritik 2 db.md). Lebih halus dari
    /// dependency file-level: bila file definisi simbol berubah, hanya file
    /// yang benar-benar memakai simbol itu yang perlu rebuild.
    symbol_uses: HashMap<PathBuf, Vec<String>>,
    /// File definisi per simbol.
    symbol_defs: HashMap<String, PathBuf>,
    /// Reverse: simbol → file yang menggunakannya.
    symbol_users: HashMap<String, Vec<PathBuf>>,
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
        // Reverse symbol index (Kritik 2).
        let mut users: HashMap<String, Vec<PathBuf>> = HashMap::new();
        for (file, symbols) in self.symbol_uses.iter() {
            for s in symbols {
                let list = users.entry(s.clone()).or_default();
                if !list.contains(file) {
                    list.push(file.clone());
                }
            }
        }
        self.symbol_users = users;
        self.dirty_reverse = false;
    }

    /// Catat bahwa `file` memakai simbol bernama `symbol` (Kritik 2).
    pub fn add_symbol_use(&mut self, file: PathBuf, symbol: String) {
        let list = self.symbol_uses.entry(file).or_default();
        if !list.contains(&symbol) {
            list.push(symbol);
            self.dirty_reverse = true;
        }
    }

    /// Set file definisi sebuah simbol (Kritik 2).
    pub fn set_symbol_def(&mut self, symbol: String, file: PathBuf) {
        self.symbol_defs.insert(symbol, file);
        self.dirty_reverse = true;
    }

    /// Hapus sebuah file dari graph (file tidak lagi aktif dalam project).
    /// Menghapus edge-nya + penggunaan simbolnya. Reverse index dibangun
    /// ulang lazy.
    pub fn remove_file(&mut self, file: &Path) {
        self.deps.remove(file);
        self.symbol_uses.remove(file);
        let removed = self
            .dependents
            .iter_mut()
            .filter(|(_, v)| v.iter().any(|f| f == file))
            .map(|(_, v)| v.retain(|f| f != file))
            .count();
        let _ = removed;
        self.symbol_defs.retain(|_, f| f != file);
        self.symbol_users
            .iter_mut()
            .for_each(|(_, v)| v.retain(|f| f != file));
        self.dirty_reverse = true;
    }

    /// File yang menggunakan `symbols` langsung + seluruh dependentnya
    /// (transitive via file graph). Inilah hasil dependency resolution yang
    /// lebih halus: perubahan pada definisi simbol tidak menyapu semua file
    /// yang hanya men-import package.
    pub fn affected_by_symbols(&mut self, symbols: &[&str]) -> Vec<PathBuf> {
        self.ensure_reverse();
        let mut changed = Vec::new();
        for s in symbols {
            if let Some(users) = self.symbol_users.get(*s) {
                changed.extend(users.iter().cloned());
            }
        }
        self.affected(&changed)
    }

    /// Simbol yang digunakan sebuah file.
    pub fn symbols_used_by(&self, file: &Path) -> Vec<String> {
        self.symbol_uses
            .get(file)
            .cloned()
            .unwrap_or_default()
    }

    /// File tempat simbol didefinisikan.
    pub fn def_of(&self, symbol: &str) -> Option<&PathBuf> {
        self.symbol_defs.get(symbol)
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

    // ── Kritik 2 db.md: dependency level simbol ──

    #[test]
    fn test_symbol_level_depends_only_users() {
        let mut g = FileGraph::new();
        // uart dan dma dua-duanya import pkg_a, tapi hanya uart pakai crc.
        g.set_symbol_def("pkg_a".into(), PathBuf::from("pkg_a.sv"));
        g.set_symbol_def("crc".into(), PathBuf::from("pkg_a.sv"));
        g.add_symbol_use(PathBuf::from("uart.sv"), "crc".into());
        g.add_symbol_use(PathBuf::from("uart.sv"), "pkg_a".into());
        g.add_symbol_use(PathBuf::from("dma.sv"), "pkg_a".into());
        g.rebuild();

        // crc berubah → hanya uart terdampak, dma tidak.
        let affected = g.affected_by_symbols(&["crc"]);
        assert!(affected.contains(&PathBuf::from("uart.sv")));
        assert!(!affected.contains(&PathBuf::from("dma.sv")));
        assert_eq!(g.def_of("crc"), Some(&PathBuf::from("pkg_a.sv")));
    }

    #[test]
    fn test_symbol_level_transitive_via_file_graph() {
        let mut g = FileGraph::new();
        // top → cpu → uart; uart pakai crc dari pkg_a.
        g.set_symbol_def("crc".into(), PathBuf::from("pkg_a.sv"));
        g.add_symbol_use(PathBuf::from("uart.sv"), "crc".into());
        g.set_deps(PathBuf::from("cpu.sv"), vec![PathBuf::from("uart.sv")]);
        g.set_deps(PathBuf::from("top.sv"), vec![PathBuf::from("cpu.sv")]);
        g.rebuild();

        let affected = g.affected_by_symbols(&["crc"]);
        assert!(affected.contains(&PathBuf::from("uart.sv")));
        assert!(affected.contains(&PathBuf::from("cpu.sv")), "dependent cpu ikut");
        assert!(affected.contains(&PathBuf::from("top.sv")), "dependent top ikut");
    }

    #[test]
    fn test_symbol_use_dedup_and_lazy_rebuild() {
        let mut g = FileGraph::new();
        let uart = PathBuf::from("uart.sv");
        g.add_symbol_use(uart.clone(), "crc".into());
        g.add_symbol_use(uart.clone(), "crc".into()); // dup diabaikan
        assert_eq!(g.symbols_used_by(&uart), vec!["crc".to_string()]);
        let affected = g.affected_by_symbols(&["crc"]); // lazy rebuild
        assert!(affected.contains(&uart));
    }
}
