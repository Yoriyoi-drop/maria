//! stringpool — interning + dedup string (Kritik 11 db.md).
//!
//! Setiap string disimpan SEKALI dalam arena bytes (tiap entry: u32 length +
//! UTF-8), dirujuk oleh `u32` id. Dedup otomatis saat `intern`. Ramah mmap:
//! `data` + `offsets` cukup dibaca langsung, tidak perlu menahan `HashMap`
//! (index dibangun ulang on-demand). Dipakai SymbolIndex agar jutaan nama
//! simbol tidak duplikat di memori.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// String pool dengan interning.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StringPool {
    /// Arena bytes: tiap string diawali u32 length (LE) lalu UTF-8.
    data: Vec<u8>,
    /// id string → offset di `data`.
    offsets: Vec<u32>,
    /// string → id (dedup). Dibangun ulang setelah deserialize.
    #[serde(skip)]
    index: HashMap<String, u32>,
}

impl StringPool {
    pub fn new() -> Self {
        StringPool::default()
    }

    /// Intern string: dedup, kembalikan id. String yang sama selalu id sama.
    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.index.get(s) {
            return id;
        }
        let id = self.offsets.len() as u32;
        self.offsets.push(self.data.len() as u32);
        self.data.extend_from_slice(&(s.len() as u32).to_le_bytes());
        self.data.extend_from_slice(s.as_bytes());
        self.index.insert(s.to_string(), id);
        id
    }

    /// Resolve id → string. `None` bila id di luar rentang.
    pub fn get(&self, id: u32) -> Option<&str> {
        let off = *self.offsets.get(id as usize)? as usize;
        let len = u32::from_le_bytes(self.data[off..off + 4].try_into().ok()?) as usize;
        let end = off + 4 + len;
        if end > self.data.len() {
            return None;
        }
        std::str::from_utf8(&self.data[off + 4..end]).ok()
    }

    /// Apakah string sudah ter-intern.
    pub fn contains(&self, s: &str) -> bool {
        self.index.contains_key(s)
    }

    /// Cari id string tanpa intern. Prioritas: index (bila sudah dibangun).
    /// Fallback scan arena (O(n)) — cukup untuk pool kecil; untuk pool besar
    /// panggil `rebuild_index` setelah deserialize agar O(1).
    pub fn id_of(&self, s: &str) -> Option<u32> {
        if let Some(&id) = self.index.get(s) {
            return Some(id);
        }
        let mut id = 0u32;
        while id < self.offsets.len() as u32 {
            if self.get(id) == Some(s) {
                return Some(id);
            }
            id += 1;
        }
        None
    }

    /// Banyak string unik.
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Ukuran arena bytes (indikator hemat memori).
    pub fn bytes_len(&self) -> usize {
        self.data.len()
    }

    /// Bangun ulang `index` dari `data`/`offsets` — wajib dipanggil setelah
    /// deserialize (index tidak diserialisasi).
    pub fn rebuild_index(&mut self) {
        self.index.clear();
        let mut id = 0u32;
        while id < self.offsets.len() as u32 {
            if let Some(s) = self.get(id) {
                self.index.entry(s.to_string()).or_insert(id);
            }
            id += 1;
        }
    }

    /// Clone yang index-nya sudah siap (untuk deserialisasi).
    pub fn with_index(&self) -> Self {
        let mut c = self.clone();
        c.rebuild_index();
        c
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_dedup() {
        let mut p = StringPool::new();
        let a = p.intern("logic");
        let b = p.intern("logic");
        assert_eq!(a, b, "string sama → id sama (dedup)");
        let c = p.intern("wire");
        assert_ne!(a, c);
        assert_eq!(p.len(), 2);
        assert!(p.contains("logic"));
        assert!(!p.contains("reg"));
    }

    #[test]
    fn test_get_roundtrip() {
        let mut p = StringPool::new();
        let samples = ["module", "counter", "clk", "reset"];
        let ids: Vec<u32> = samples.iter().map(|s| p.intern(s)).collect();
        for (id, s) in ids.iter().zip(samples.iter()) {
            assert_eq!(p.get(*id), Some(*s));
        }
        assert!(p.get(9999).is_none());
    }

    #[test]
    fn test_serialize_roundtrip_rebuilds_index() {
        let mut p = StringPool::new();
        let id = p.intern("dma");
        p.intern("cpu");
        let bytes = bincode::serialize(&p).unwrap();
        let mut p2: StringPool = bincode::deserialize(&bytes).unwrap();
        // Index tidak ikut serialize → get tetap jalan, intern harus dedup lagi.
        assert_eq!(p2.get(id), Some("dma"));
        p2.rebuild_index();
        let again = p2.intern("dma");
        assert_eq!(again, id, "rebuild index → dedup kembali");
        assert_eq!(p2.len(), 2);
    }

    #[test]
    fn test_arena_compact_for_many_strings() {
        // 10k string pendek unik → total bytes jauh di bawah representasi
        // String (header + pointer + heap). Verifikasi arena <= 3x payload.
        let mut p = StringPool::new();
        let mut payload = 0usize;
        for i in 0..10_000 {
            let s = format!("sym_{}", i);
            payload += 4 + s.len();
            p.intern(&s);
        }
        assert_eq!(p.len(), 10_000);
        assert!(
            p.bytes_len() <= payload,
            "arena tidak boleh lebih besar dari payload"
        );
    }
}
