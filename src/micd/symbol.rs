//! symbol.mdb — index simbol (module/package → file) untuk lookup cepat.
//!
//! Menyimpan daftar simbol yang didefinisikan tiap file. Query O(1) oleh
//! IDE/LSP: "di file mana module `uart` didefinisikan?" tanpa compile.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Index simbol: nama simbol → (jenis, file definisi).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SymbolIndex {
    /// Nama simbol → daftar (jenis, file).
    pub index: BTreeMap<String, Vec<(String, PathBuf)>>,
}

impl SymbolIndex {
    pub fn new() -> Self {
        SymbolIndex::default()
    }

    /// Daftarkan simbol `name` bertipe `kind` yang didefinisikan di `file`.
    pub fn add(&mut self, name: String, kind: String, file: PathBuf) {
        let entry = self.index.entry(name).or_default();
        if !entry.iter().any(|(k, f)| *k == kind && *f == file) {
            entry.push((kind, file));
        }
    }

    /// File tempat `name` (dengan `kind`) didefinisikan.
    pub fn locate(&self, name: &str, kind: &str) -> Option<&PathBuf> {
        self.index
            .get(name)?
            .iter()
            .find(|(k, _)| k == kind)
            .map(|(_, f)| f)
    }

    /// Semua nama simbol.
    pub fn names(&self) -> Vec<&String> {
        self.index.keys().collect()
    }

    /// Banyak simbol.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_index() {
        let mut s = SymbolIndex::new();
        s.add("uart".into(), "module".into(), PathBuf::from("uart.sv"));
        s.add("pkg_a".into(), "package".into(), PathBuf::from("pkg_a.sv"));
        assert_eq!(
            s.locate("uart", "module"),
            Some(&PathBuf::from("uart.sv"))
        );
        assert!(s.locate("uart", "package").is_none());
        assert_eq!(s.len(), 2);
    }
}
