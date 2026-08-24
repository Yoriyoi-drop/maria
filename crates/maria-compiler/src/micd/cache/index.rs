//! index — index cache kategori: key → entry objek (db.md "index/: FileID atau
//! NodeID → objek cache").
//!
//! Memetakan key string (path file, nama module, hash hex) ke entry objek yang
//! disimpan secara content-addressed di `objects/`/`blobs/`. Persisten di
//! `index/index.mdb` (format MDB1 — mmap, lookup O(1)). Lookup utama dihitung
//! dari checksum key; payload objek tidak pernah disalin ke index (hanya
//! hash + metadata).

use std::collections::HashMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::super::format::{MdbReader, MdbWriter, KIND_STRING};
use crate::cache::compute_checksum;

/// Satu entry index: bagaimana menemukan + memulihkan objek.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CacheIndexEntry {
    /// Content hash objek (kunci CAS di `objects/`/`blobs/`).
    pub content_hash: u64,
    /// Ukuran payload (byte).
    pub size: u64,
    /// Disimpan di `blobs/` (true, besar) atau `objects/` (false).
    pub large: bool,
    /// Kind byte kategori (debug/diskriminasi).
    pub kind: u8,
    pub created_ns: u64,
    /// Waktu akses terakhir (LRU, Kritik 6 db.md).
    pub accessed_ns: u64,
}

/// Index key string → entry objek.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheIndex {
    pub entries: HashMap<String, CacheIndexEntry>,
}

impl CacheIndex {
    pub fn new() -> Self {
        CacheIndex::default()
    }

    pub fn get(&self, key: &str) -> Option<&CacheIndexEntry> {
        self.entries.get(key)
    }

    pub fn put(&mut self, key: String, entry: CacheIndexEntry) {
        self.entries.insert(key, entry);
    }

    pub fn remove(&mut self, key: &str) -> Option<CacheIndexEntry> {
        self.entries.remove(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total bytes payload yang direferensikan.
    pub fn bytes(&self) -> u64 {
        self.entries.values().map(|e| e.size).sum()
    }

    /// Kunci objek CAS (u64) untuk key string — checksum key.
    pub fn key_hash(key: &str) -> u64 {
        compute_checksum(key.as_bytes())
    }

    /// Muat index dari file MDB. Hilang/corrupt → kosong (best-effort).
    ///
    /// Payload objek menyimpan pasangan `(String, CacheIndexEntry)` — key
    /// string asli ikut disimpan karena hash u64 (kunci MDB) tidak bisa
    /// dibalik menjadi string.
    pub fn load(path: &Path) -> CacheIndex {
        let mut idx = CacheIndex::new();
        let Ok(r) = MdbReader::open(path) else {
            return idx;
        };
        for (key, _kind) in r.keys() {
            if let Some(b) = r.get(key) {
                if let Ok((k, e)) = bincode::deserialize::<(String, CacheIndexEntry)>(&b) {
                    if Self::key_hash(&k) == key {
                        idx.entries.insert(k, e);
                    }
                }
            }
        }
        idx
    }

    /// Simpan index ke file MDB (atomik, temp + rename).
    pub fn save(
        &self,
        path: &Path,
        compression: super::super::format::Compression,
    ) -> io::Result<()> {
        let mut w = MdbWriter::with_compression(compression);
        for (key, e) in &self.entries {
            let payload = bincode::serialize(&(key, e)).map_err(io::Error::other)?;
            w.put(Self::key_hash(key), KIND_STRING, payload);
        }
        w.write_to(path)
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_put_get_remove() {
        let mut idx = CacheIndex::new();
        assert!(idx.is_empty());
        idx.put(
            "a.sv".into(),
            CacheIndexEntry {
                content_hash: 1,
                size: 5,
                large: false,
                kind: 3,
                created_ns: 0,
                accessed_ns: 0,
            },
        );
        idx.put(
            "b.sv".into(),
            CacheIndexEntry {
                content_hash: 2,
                size: 10,
                large: true,
                kind: 3,
                created_ns: 0,
                accessed_ns: 0,
            },
        );
        assert_eq!(idx.len(), 2);
        assert!(idx.contains("a.sv"));
        assert_eq!(idx.get("a.sv").unwrap().content_hash, 1);
        assert!(idx.get("a.sv").unwrap().large == false);
        assert!(idx.get("b.sv").unwrap().large);
        assert_eq!(idx.bytes(), 15);
        assert_eq!(idx.remove("a.sv").unwrap().size, 5);
        assert_eq!(idx.len(), 1);
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("maria_cache_index_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("index.mdb");
        {
            let mut idx = CacheIndex::new();
            for i in 0..50u64 {
                idx.put(
                    format!("mod_{}.sv", i),
                    CacheIndexEntry {
                        content_hash: i * 3,
                        size: 100,
                        large: i % 2 == 0,
                        kind: 1,
                        created_ns: i,
                        accessed_ns: i,
                    },
                );
            }
            idx.save(&path, crate::micd::format::Compression::Lz4)
                .unwrap();
        }
        let idx = CacheIndex::load(&path);
        assert_eq!(idx.len(), 50);
        assert_eq!(idx.get("mod_7.sv").unwrap().content_hash, 21);
        assert!(idx.get("mod_8.sv").unwrap().large);
        assert!(idx.get("mod_9.sv").unwrap().large == false);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_corrupt_empty() {
        let dir =
            std::env::temp_dir().join(format!("maria_cache_index_corrupt_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("index.mdb");
        std::fs::write(&path, vec![0xFF; 64]).unwrap();
        let idx = CacheIndex::load(&path);
        assert!(idx.is_empty(), "corrupt → kosong (best-effort)");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
