//! precompiled.rs — Per-module analysis database (analogue: VCS `AN.DB/`,
//! Questa `_info`, Xcelium `INCA_libs/`, Vivado `.Xil/ip/`).
//!
//! Menyimpan hasil analisis per module/package/class/interface sebagai
//! artefak build yang bisa dibaca ulang oleh tool downstream tanpa
//! compile ulang. Setiap module punya fingerprint (content hash) untuk
//! deteksi perubahan cepat: hash sama → skip compile, hash beda → recompile.
//!
//! Direktori layout:
//! ```text
//! precompiled/<pid>/
//!     manifest.mdb          — daftar semua module yang sudah dianalisis
//!     <module_name>/
//!         <hash>.mdb        — hasil analisis module (CAS, immutable)
//! ```
//!
//! Integrisasi: tool (`mlint`, `melab`, `msim`, `mcov`) cukup baca
//! `precompiled/` untuk dapat hasil analisis tanpa menjalankan pipeline
//! penuh.指纹 (fingerprint) per-module disimpan di `metadata.mdb` sebagai
//! field tambahan pada `FileMeta`.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::format::{MdbReader, MdbWriter, KIND_META};
use crate::cache::checksum::compute_checksum;

// ── Versi format precompiled module (Kritik 3 db.md: bump saat skema berubah) ──

/// Versi format serialisasi `PrecompiledModule`. Naikkan bila field ditambah/
/// diubah agar database lama dianggap tidak kompatibel.
#[allow(dead_code)]
pub const PRECOMPILED_FORMAT_VERSION: u64 = 1;

/// Nama file manifest di root precompiled.
pub const MANIFEST_FILE: &str = "manifest.mdb";

// ── Data structures ──

/// Info port satu module (untuk query cepat tanpa deserialize AST penuh).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortInfo {
    pub name: String,
    pub dir: String, // "input" / "output" / "inout" / "ref"
    pub width: usize,
    pub is_signed: bool,
}

/// Hasil analisis satu module yang sudah di-compile (immutable artifact).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrecompiledModule {
    /// Nama module (unique key).
    pub name: String,
    /// Content hash dari source file module ini (untuk fingerprint).
    pub content_hash: u64,
    /// Hash AST serialized — untuk multi-level reuse (Kritik 1 db.md).
    pub ast_hash: u64,
    /// Signature tipe (hash dari nama + port + param — cepat dibanding).
    pub type_signature: u64,
    /// Source file path (dari mana module ini berasal).
    pub source_file: PathBuf,
    /// Waktu analisis (unix ns).
    pub analyzed_at_ns: u64,
    /// Nama library (default: "work" — sejalan Questa/VCS convention).
    pub library: String,

    // ── Port info (query-friendly, tanpa deserialize AST penuh) ──
    pub ports: Vec<PortInfo>,

    // ── Dependensi ──
    /// Module lain yang di-import/di-instansiasi oleh module ini.
    pub depends_on: Vec<String>,
    /// Module lain yang menginstansiasi module ini (reverse deps).
    pub depended_by: Vec<String>,

    // ── Metadata kompilasi ──
    /// Jumlah token (lexer summary).
    pub token_count: u64,
    /// Jumlah proses (always_comb, always_ff, initial, dll).
    pub process_count: usize,
    /// Jumlah signal (post-elaboration bila tersedia).
    pub signal_count: usize,
    /// Jumlah error saat kompilasi (0 = clean).
    pub error_count: usize,
    /// Jumlah warning saat kompilasi.
    pub warn_count: usize,

    // ── Artefak serialized (opsional, untuk full restore) ──
    /// AST serialized (bincode). Kosong bila tidak disimpan.
    #[serde(default)]
    pub ast_bytes: Vec<u8>,
    /// IR serialized (bincode) — hasil elaborasi. Kosong bila belum dielaborasi.
    #[serde(default)]
    pub ir_bytes: Vec<u8>,

    // ── Checksum ──
    /// Checksum dari seluruh field (untuk integritas saat load).
    pub checksum: u64,
}

impl PrecompiledModule {
    /// Hitung checksum dari field-field utama.
    pub fn compute_checksum(&self) -> u64 {
        let mut h = 0u64;
        h = h
            .wrapping_mul(31)
            .wrapping_add(compute_checksum(self.name.as_bytes()));
        h = h.wrapping_mul(31).wrapping_add(self.content_hash);
        h = h.wrapping_mul(31).wrapping_add(self.ast_hash);
        h = h.wrapping_mul(31).wrapping_add(self.type_signature);
        h = h.wrapping_mul(31).wrapping_add(compute_checksum(
            self.source_file.to_string_lossy().as_bytes(),
        ));
        h = h.wrapping_mul(31).wrapping_add(self.token_count);
        h = h.wrapping_mul(31).wrapping_add(self.process_count as u64);
        h = h.wrapping_mul(31).wrapping_add(self.signal_count as u64);
        for p in &self.ports {
            h = h
                .wrapping_mul(31)
                .wrapping_add(compute_checksum(p.name.as_bytes()));
            h = h
                .wrapping_mul(31)
                .wrapping_add(compute_checksum(p.dir.as_bytes()));
            h = h.wrapping_mul(31).wrapping_add(p.width as u64);
        }
        for d in &self.depends_on {
            h = h
                .wrapping_mul(31)
                .wrapping_add(compute_checksum(d.as_bytes()));
        }
        h
    }

    /// Verifikasi checksum — return false bila data corrupt.
    pub fn verify_checksum(&self) -> bool {
        self.checksum == self.compute_checksum()
    }
}

// ── Precompiled Database ──

/// Database artefak precompiled per module (mirip VCS AN.DB / Questa _info).
pub struct PrecompiledDb {
    /// Root precompiled: `<db_root>/precompiled/<pid>/`.
    pub root: PathBuf,
    /// Module name → PrecompiledModule.
    pub modules: HashMap<String, PrecompiledModule>,
    /// Content hash → nama module (reverse index untuk lookup cepat).
    pub by_hash: HashMap<u64, String>,
    /// Ada perubahan belum disimpan.
    pub dirty: bool,
}

impl PrecompiledDb {
    /// Buka (atau buat) precompiled database.
    pub fn open(root: &Path) -> Self {
        let _ = std::fs::create_dir_all(root);
        let mut db = PrecompiledDb {
            root: root.to_path_buf(),
            modules: HashMap::new(),
            by_hash: HashMap::new(),
            dirty: false,
        };
        db.load_manifest();
        db
    }

    /// Path direktori untuk satu module.
    fn module_dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Path file CAS untuk satu module + hash.
    fn module_path(&self, name: &str, hash: u64) -> PathBuf {
        self.module_dir(name).join(format!("{:016x}.mdb", hash))
    }

    /// Path manifest.
    fn manifest_path(&self) -> PathBuf {
        self.root.join(MANIFEST_FILE)
    }

    /// Muat manifest dari disk.
    fn load_manifest(&mut self) {
        let path = self.manifest_path();
        let Ok(r) = MdbReader::open(&path) else {
            return;
        };
        // Load semua module dari manifest keys.
        for (key, _kind) in r.keys() {
            if key == super::KEY_SINGLETON {
                continue;
            }
            if let Some(bytes) = r.get(key) {
                if let Ok(m) = bincode::deserialize::<PrecompiledModule>(&bytes) {
                    if m.verify_checksum() {
                        self.by_hash.insert(m.content_hash, m.name.clone());
                        self.modules.insert(m.name.clone(), m);
                    }
                    // Module corrupt → skip (best-effort).
                }
            }
        }
    }

    /// Simpan manifest ke disk.
    fn save_manifest(&self) -> io::Result<()> {
        let mut w = MdbWriter::new();
        for (name, module) in &self.modules {
            let key = super::metadata::path_hash(Path::new(name));
            let bytes = bincode::serialize(module).map_err(io::Error::other)?;
            w.put(key, KIND_META, bytes);
        }
        w.write_to(&self.manifest_path())
    }

    /// Lookup module berdasarkan nama.
    pub fn get(&self, name: &str) -> Option<&PrecompiledModule> {
        self.modules.get(name)
    }

    /// Lookup module berdasarkan content hash.
    pub fn get_by_hash(&self, hash: u64) -> Option<&PrecompiledModule> {
        self.by_hash
            .get(&hash)
            .and_then(|name| self.modules.get(name))
    }

    /// Cek apakah module dengan hash tertentu sudah di-precompile.
    pub fn has_valid(&self, name: &str, content_hash: u64) -> bool {
        self.modules
            .get(name)
            .map(|m| m.content_hash == content_hash && m.verify_checksum())
            .unwrap_or(false)
    }

    /// Simpan hasil analisis satu module.
    pub fn put(&mut self, module: PrecompiledModule) {
        let hash = module.content_hash;
        let name = module.name.clone();
        // Tulis CAS object ke disk.
        let dir = self.module_dir(&name);
        let _ = std::fs::create_dir_all(&dir);
        let path = self.module_path(&name, hash);
        if !path.exists() {
            let payload = bincode::serialize(&module).map_err(io::Error::other);
            if let Ok(bytes) = payload {
                let _ = super::format::write_tmp(&path, &bytes);
                let _ = super::format::commit_tmp(&path);
            }
        }
        // Update indeks.
        self.by_hash.insert(hash, name.clone());
        self.modules.insert(name, module);
        self.dirty = true;
    }

    /// Hapus module dari database.
    pub fn remove(&mut self, name: &str) -> bool {
        if let Some(m) = self.modules.remove(name) {
            self.by_hash.remove(&m.content_hash);
            // Hapus file CAS.
            let path = self.module_path(name, m.content_hash);
            let _ = std::fs::remove_file(path);
            let dir = self.module_dir(name);
            let _ = std::fs::remove_dir(&dir); // hanya bila kosong
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Simpan manifest + sweep objek yatim.
    pub fn save(&mut self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        self.save_manifest()?;
        self.sweep_orphaned();
        self.dirty = false;
        Ok(())
    }

    /// Buang objek CAS yang tidak lagi dirujuk index.
    fn sweep_orphaned(&self) {
        // Scan semua subdirektori module — bila nama tidak ada di index, hapus.
        let _ = std::fs::read_dir(&self.root).map(|entries| {
            for entry in entries.flatten() {
                if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let module_name = entry.file_name().to_string_lossy().to_string();
                if self.modules.contains_key(&module_name) {
                    continue; // module masih hidup
                }
                // Module tidak ada di index → hapus direktorinya.
                let _ = std::fs::remove_dir_all(entry.path());
            }
        });
    }

    /// Banyak module yang tersimpan.
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// Clear seluruh database.
    pub fn clear(&mut self) -> io::Result<()> {
        let _ = std::fs::remove_dir_all(&self.root);
        let _ = std::fs::create_dir_all(&self.root);
        self.modules.clear();
        self.by_hash.clear();
        self.dirty = false;
        Ok(())
    }

    /// Statistik ringkas.
    pub fn stats(&self) -> PrecompiledStats {
        let total_tokens: u64 = self.modules.values().map(|m| m.token_count).sum();
        let total_errors: usize = self.modules.values().map(|m| m.error_count).sum();
        let total_warnings: usize = self.modules.values().map(|m| m.warn_count).sum();
        let clean = self.modules.values().filter(|m| m.error_count == 0).count();
        PrecompiledStats {
            modules: self.modules.len(),
            clean_modules: clean,
            total_tokens,
            total_errors,
            total_warnings,
        }
    }
}

/// Ringkasan statistik precompiled database.
#[derive(Debug, Clone, Default)]
pub struct PrecompiledStats {
    pub modules: usize,
    pub clean_modules: usize,
    pub total_tokens: u64,
    pub total_errors: usize,
    pub total_warnings: usize,
}

impl std::fmt::Display for PrecompiledStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "precompiled: {} modules ({} clean), {} tokens, {} errors, {} warnings",
            self.modules,
            self.clean_modules,
            self.total_tokens,
            self.total_errors,
            self.total_warnings
        )
    }
}

// ─── Fingerprint helpers ──

/// Hitung fingerprint per-module dari content hash + AST hash + type signature.
/// Digunakan untuk deteksi perubahan cepat tanpa deserialize AST penuh.
pub fn module_fingerprint(content_hash: u64, ast_hash: u64, type_signature: u64) -> u64 {
    let mut h = 0u64;
    h = h.wrapping_mul(31).wrapping_add(content_hash);
    h = h.wrapping_mul(31).wrapping_add(ast_hash);
    h = h.wrapping_mul(31).wrapping_add(type_signature);
    h
}

/// Hitung content hash dari source bytes.
pub fn content_hash(bytes: &[u8]) -> u64 {
    compute_checksum(bytes)
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::super::verify::now_ns;
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("maria_precompiled_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn sample_module(name: &str, hash: u64) -> PrecompiledModule {
        let mut m = PrecompiledModule {
            name: name.to_string(),
            content_hash: hash,
            ast_hash: hash ^ 0xA,
            type_signature: hash ^ 0xB,
            source_file: PathBuf::from(format!("{}.sv", name)),
            analyzed_at_ns: now_ns(),
            library: "work".to_string(),
            ports: vec![PortInfo {
                name: "clk".to_string(),
                dir: "input".to_string(),
                width: 1,
                is_signed: false,
            }],
            depends_on: vec![],
            depended_by: vec![],
            token_count: 100,
            process_count: 2,
            signal_count: 5,
            error_count: 0,
            warn_count: 0,
            ast_bytes: vec![],
            ir_bytes: vec![],
            checksum: 0,
        };
        m.checksum = m.compute_checksum();
        m
    }

    #[test]
    fn test_roundtrip() {
        let root = test_root("roundtrip");
        let mut db = PrecompiledDb::open(&root);
        let m = sample_module("counter", 42);
        db.put(m);
        assert_eq!(db.len(), 1);
        assert!(db.has_valid("counter", 42));
        assert!(!db.has_valid("counter", 99));
        assert!(!db.has_valid("other", 42));
        db.save().unwrap();

        // Reload.
        drop(db);
        let db2 = PrecompiledDb::open(&root);
        assert_eq!(db2.len(), 1);
        assert!(db2.has_valid("counter", 42));
        let loaded = db2.get("counter").unwrap();
        assert_eq!(loaded.ports.len(), 1);
        assert_eq!(loaded.ports[0].name, "clk");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_get_by_hash() {
        let root = test_root("byhash");
        let mut db = PrecompiledDb::open(&root);
        db.put(sample_module("uart", 100));
        db.put(sample_module("dma", 200));
        assert_eq!(db.get_by_hash(100).unwrap().name, "uart");
        assert_eq!(db.get_by_hash(200).unwrap().name, "dma");
        assert!(db.get_by_hash(999).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_remove() {
        let root = test_root("remove");
        let mut db = PrecompiledDb::open(&root);
        db.put(sample_module("a", 1));
        assert!(db.remove("a"));
        assert!(!db.remove("a"));
        assert!(db.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_checksum_detect_corrupt() {
        let root = test_root("corrupt");
        let mut db = PrecompiledDb::open(&root);
        let mut m = sample_module("x", 50);
        db.put(m.clone());
        // Corrupt: ubah content_hash tanpa update checksum.
        m.content_hash = 999;
        // data asli di disk masih valid (hash 50).
        assert!(db.has_valid("x", 50));
        // Tapi module di memori di-corrupt.
        db.modules.get_mut("x").unwrap().content_hash = 999;
        assert!(!db.has_valid("x", 999)); // checksum mismatch
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_stats() {
        let root = test_root("stats");
        let mut db = PrecompiledDb::open(&root);
        let mut m1 = sample_module("a", 1);
        m1.token_count = 50;
        m1.error_count = 2;
        db.put(m1);
        let mut m2 = sample_module("b", 2);
        m2.token_count = 75;
        db.put(m2);
        let st = db.stats();
        assert_eq!(st.modules, 2);
        assert_eq!(st.clean_modules, 1); // b clean, a has errors
        assert_eq!(st.total_tokens, 125);
        assert_eq!(st.total_errors, 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_clear() {
        let root = test_root("clear");
        let mut db = PrecompiledDb::open(&root);
        db.put(sample_module("a", 1));
        db.put(sample_module("b", 2));
        db.save().unwrap();
        db.clear().unwrap();
        assert!(db.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_fingerprint_deterministic() {
        let fp1 = module_fingerprint(100, 200, 300);
        let fp2 = module_fingerprint(100, 200, 300);
        assert_eq!(fp1, fp2);
        let fp3 = module_fingerprint(100, 200, 301);
        assert_ne!(fp1, fp3);
    }

    #[test]
    fn test_content_hash_deterministic() {
        let h1 = content_hash(b"module counter; endmodule");
        let h2 = content_hash(b"module counter; endmodule");
        assert_eq!(h1, h2);
        let h3 = content_hash(b"module other; endmodule");
        assert_ne!(h1, h3);
    }
}
