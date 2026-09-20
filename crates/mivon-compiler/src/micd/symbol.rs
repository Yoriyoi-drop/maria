//! symbol.mdb — index simbol (module/package → file) untuk lookup cepat.
//!
//! Menyimpan daftar simbol yang didefinisikan tiap file. Query O(1) oleh
//! IDE/LSP: "di file mana module `uart` didefinisikan?" tanpa compile.
//!
//! Sejak Kritik 11 db.md, nama simbol & jenisnya di-intern via [`StringPool`]
//! (u32 id, dedup) — bukan string mentah — agar memori hemat untuk proyek
//! besar. API publik tetap menerima/mengembalikan `&str`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::stringpool::StringPool;

/// Index simbol: nama (id) → daftar (jenis-id, file definisi).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SymbolIndex {
    /// Nama simbol (id pool) → daftar (jenis id, file).
    pub index: BTreeMap<u32, Vec<(u32, PathBuf)>>,
    /// String pool berisi seluruh nama & jenis.
    pub pool: StringPool,
}

impl SymbolIndex {
    pub fn new() -> Self {
        SymbolIndex::default()
    }

    /// Pastikan pool siap dipakai (index dedup) setelah deserialize.
    pub fn prepare(&mut self) {
        self.pool.rebuild_index();
    }

    /// Daftarkan simbol `name` bertipe `kind` yang didefinisikan di `file`.
    pub fn add(&mut self, name: String, kind: String, file: PathBuf) {
        let nid = self.pool.intern(&name);
        let kid = self.pool.intern(&kind);
        let entry = self.index.entry(nid).or_default();
        if !entry.iter().any(|(k, f)| *k == kid && *f == file) {
            entry.push((kid, file));
        }
    }

    /// File tempat `name` (dengan `kind`) didefinisikan.
    pub fn locate(&self, name: &str, kind: &str) -> Option<&PathBuf> {
        let nid = self.pool.id_of(name)?;
        let kid = self.pool.id_of(kind)?;
        self.index
            .get(&nid)?
            .iter()
            .find(|(k, _)| *k == kid)
            .map(|(_, f)| f)
    }

    /// Semua nama simbol (di-resolve dari pool).
    pub fn names(&self) -> Vec<String> {
        self.index
            .keys()
            .filter_map(|id| self.pool.get(*id).map(|s| s.to_string()))
            .collect()
    }

    /// Banyak simbol unik.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Total string unik di pool (indikator dedup).
    pub fn pool_len(&self) -> usize {
        self.pool.len()
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
        assert_eq!(s.locate("uart", "module"), Some(&PathBuf::from("uart.sv")));
        assert!(s.locate("uart", "package").is_none());
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn test_dedup_pool() {
        let mut s = SymbolIndex::new();
        // 3 simbol, 2 nama unik + 1 jenis unik.
        s.add("a".into(), "module".into(), "a.sv".into());
        s.add("b".into(), "module".into(), "b.sv".into());
        s.add("a".into(), "package".into(), "p.sv".into());
        assert_eq!(s.pool_len(), 4, "a,b,module,package = 4 string unik");
        assert!(s.names().contains(&"a".to_string()));
        assert!(s.names().contains(&"b".to_string()));
    }

    #[test]
    fn test_serialize_roundtrip() {
        let mut s = SymbolIndex::new();
        s.add("uart".into(), "module".into(), "uart.sv".into());
        s.add("crc".into(), "function".into(), "pkg_a.sv".into());
        let bytes = bincode::serialize(&s).unwrap();
        let mut s2: SymbolIndex = bincode::deserialize(&bytes).unwrap();
        s2.prepare();
        assert_eq!(s2.locate("uart", "module"), Some(&PathBuf::from("uart.sv")));
        assert_eq!(
            s2.locate("crc", "function").map(|f| f.to_str().unwrap()),
            Some("pkg_a.sv")
        );
    }
}
