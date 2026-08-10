//! snapshots/ — snapshot build (mirip commit git) untuk rollback.
//!
//! Setiap build yang berhasil menyimpan snapshot state (metadata + graph +
//! verify + symbol index) ke `snapshots/build-NNN` sebagai satu blob biner.
//! Rollback mengembalikan state tersebut, lalu seluruh artefak AST yang
//! cocok dengan metadata di-reuse pada build berikutnya.
//!
//! Sejak Kritik 13 db.md, snapshot membentuk **DAG** (bukan rantai linear):
//! setiap snapshot menyimpan `parents` (biasanya satu, bisa beberapa untuk
//! merge/multi-parent). `parent = []` adalah snapshot akar (build pertama).
//! Mirip commit Git: `snapshot_from(parents, ...)` membuat snapshot turunan
//! dari snapshot yang dipilih; `snapshot(...)` otomatis mengambil parent
//! terakhir. Linear tetap mungkin (tiap snapshot ber-parent satu).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::diag::FileDiags;
use super::graph::FileGraph;
use super::metadata::FileMeta;
use super::symbol::SymbolIndex;
use super::verify::VerifyResult;

/// Isi satu snapshot build.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    /// Nomor build (incrementing).
    pub id: u64,
    /// Waktu pembuatan (unix ns).
    pub created_ns: u64,
    /// Parent snapshot (DAG, Kritik 13). Kosong = snapshot akar.
    /// Umumnya 1 (linear); bisa >1 untuk merge (branch digabung).
    pub parents: Vec<u64>,
    /// Metadata seluruh file.
    pub files: Vec<FileMeta>,
    /// Dependency graph.
    pub graph: FileGraph,
    /// Verification cache.
    pub verify: Vec<(u64, VerifyResult)>,
    /// Index simbol.
    pub symbols: SymbolIndex,
    /// Diagnostic per file.
    pub diags: Vec<FileDiags>,
    /// Hash flags kompilasi saat snapshot.
    pub flags_hash: u64,
    /// Pesan ringkas (opsional).
    pub note: String,
}

impl Snapshot {
    pub fn root() -> Self {
        Snapshot::default()
    }

    /// Snapshot yang punya parent `parent`.
    pub fn with_parent(parent: u64) -> Self {
        Snapshot {
            parents: vec![parent],
            ..Snapshot::default()
        }
    }
}

/// Nama file snapshot untuk sebuah build id.
pub fn snapshot_path(root: &Path, id: u64) -> PathBuf {
    root.join("snapshots").join(format!("build-{:03}", id))
}

/// List snapshot id yang ada di root (scan direktori snapshots/).
pub fn list_snapshots(root: &Path) -> Vec<u64> {
    let dir = root.join("snapshots");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if let Some(rest) = name.strip_prefix("build-") {
            if let Ok(id) = rest.parse::<u64>() {
                ids.push(id);
            }
        }
    }
    ids.sort_unstable();
    ids
}

/// Baca satu snapshot dari disk. `None` bila tidak ada / corrupt.
pub fn read_snapshot(root: &Path, id: u64) -> Option<Snapshot> {
    let path = snapshot_path(root, id);
    let bytes = std::fs::read(&path).ok()?;
    bincode::deserialize(&bytes).ok()
}

/// Ambil `id` terbesar dari daftar yang tersedia (atau 0 bila kosong).
pub fn last_snapshot_id(ids: &[u64]) -> u64 {
    ids.iter().copied().max().unwrap_or(0)
}

/// Parent snapshot (DAG, Kritik 13). `None` bila snapshot tidak ada.
pub fn parents_of(root: &Path, id: u64) -> Option<Vec<u64>> {
    read_snapshot(root, id).map(|s| s.parents)
}

/// Seluruh riwayat (ancestor chain) dari snapshot `id` ke akar, DFS.
/// Termasuk `id` itu sendiri di posisi pertama. Menghasilkan DAG, bukan
/// rantai — bila ada merge, kedua cabang ikut.
pub fn history_of(root: &Path, id: u64) -> Vec<u64> {
    let mut out = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut stack = vec![id];
    while let Some(cur) = stack.pop() {
        if !visited.insert(cur) {
            continue;
        }
        out.push(cur);
        if let Some(parents) = parents_of(root, cur) {
            for p in parents {
                if !visited.contains(&p) {
                    stack.push(p);
                }
            }
        }
    }
    out
}

/// Common ancestor terdekat (merge-base) dari dua snapshot. `None` bila
/// tidak ada yang berbagi garis keturunan.
pub fn merge_base(root: &Path, a: u64, b: u64) -> Option<u64> {
    let hist_a: std::collections::HashSet<u64> =
        history_of(root, a).into_iter().collect();
    let mut stack = vec![b];
    let mut visited = std::collections::HashSet::new();
    while let Some(cur) = stack.pop() {
        if !visited.insert(cur) {
            continue;
        }
        if hist_a.contains(&cur) {
            return Some(cur);
        }
        if let Some(parents) = parents_of(root, cur) {
            for p in parents {
                if !visited.contains(&p) {
                    stack.push(p);
                }
            }
        }
    }
    None
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_roundtrip() {
        let mut s = Snapshot::default();
        s.id = 1;
        s.files.push(FileMeta {
            path: PathBuf::from("a.sv"),
            content_hash: 5,
            mtime_ns: 0,
            size: 10,
            status: super::super::metadata::FileStatus::Unchanged,
            flags_hash: 0,
            deps: vec![],
            include_hashes: vec![],
            compiled_at_ns: 0,
            ast_format_version: 1,
        });
        let bytes = bincode::serialize(&s).unwrap();
        let s2: Snapshot = bincode::deserialize(&bytes).unwrap();
        assert_eq!(s2.files[0].content_hash, 5);
    }

    // ── Kritik 13 db.md: Snapshot DAG ──

    fn dag_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "maria_micd_snap_dag_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("snapshots")).unwrap();
        dir
    }

    fn write_snap(root: &Path, id: u64, parents: Vec<u64>) {
        let s = Snapshot {
            id,
            parents,
            ..Snapshot::default()
        };
        std::fs::write(
            snapshot_path(root, id),
            bincode::serialize(&s).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn test_history_and_parents() {
        let root = dag_root("history");
        // DAG: 1 → 2 → 4, dan 1 → 3 → 4 (merge).
        write_snap(&root, 1, vec![]);
        write_snap(&root, 2, vec![1]);
        write_snap(&root, 3, vec![1]);
        write_snap(&root, 4, vec![2, 3]);

        assert_eq!(parents_of(&root, 4).unwrap(), vec![2, 3]);
        assert_eq!(parents_of(&root, 1).unwrap(), Vec::<u64>::new());

        let mut hist = history_of(&root, 4);
        hist.sort_unstable();
        assert_eq!(hist, vec![1, 2, 3, 4], "riwayat mencakup kedua cabang");

        // Merge-base 4 dan 2 = 2.
        assert_eq!(merge_base(&root, 4, 2), Some(2));
        // Merge-base 4 dan 3 = 3.
        assert_eq!(merge_base(&root, 4, 3), Some(3));
        // Merge-base 2 dan 3 = 1 (common ancestor).
        assert_eq!(merge_base(&root, 2, 3), Some(1));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_read_missing_returns_none() {
        let root = dag_root("missing");
        assert!(read_snapshot(&root, 99).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
