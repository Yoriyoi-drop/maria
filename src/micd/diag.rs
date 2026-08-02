//! diagnostics.mdb — database diagnostic per file.
//!
//! IDE/LSP/GUI cukup query file → daftar diagnostic tanpa compile ulang.
//! Format append-friendly: satu objek per file berisi snapshot diagnostic
//! terbaru file tersebut (level, posisi, pesan).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Tingkat keparahan diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagSeverity {
    Error,
    Warning,
    Hint,
    Note,
    Info,
}

/// Satu diagnostic per baris.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagEntry {
    pub line: usize,
    pub col: usize,
    pub severity: DiagSeverity,
    pub message: String,
    /// Kode diagnostic (kosong bila tidak ada).
    pub code: String,
}

/// Diagnostic satu file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileDiags {
    pub path: PathBuf,
    pub entries: Vec<DiagEntry>,
    /// Hash konten file saat diagnostic diambil.
    pub content_hash: u64,
}

impl FileDiags {
    pub fn error_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.severity == DiagSeverity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.severity == DiagSeverity::Warning)
            .count()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diag_counts() {
        let d = FileDiags {
            path: PathBuf::from("a.sv"),
            entries: vec![
                DiagEntry {
                    line: 1,
                    col: 0,
                    severity: DiagSeverity::Error,
                    message: "boom".into(),
                    code: "ER001".into(),
                },
                DiagEntry {
                    line: 2,
                    col: 0,
                    severity: DiagSeverity::Warning,
                    message: "warn".into(),
                    code: "WR010".into(),
                },
            ],
            content_hash: 1,
        };
        assert_eq!(d.error_count(), 1);
        assert_eq!(d.warning_count(), 1);
    }
}
