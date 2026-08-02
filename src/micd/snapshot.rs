//! snapshots/ — snapshot build (mirip commit git) untuk rollback.
//!
//! Setiap build yang berhasil menyimpan snapshot state (metadata + graph +
//! verify + symbol index) ke `snapshots/build-NNN` sebagai satu blob biner.
//! Rollback mengembalikan state tersebut, lalu seluruh artefak AST yang
//! cocok dengan metadata di-reuse pada build berikutnya.

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
}
