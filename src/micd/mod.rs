//! MICD — Maria Incremental Compilation Database.
//!
//! Object database biner (bukan SQL) di `project/.maria/database/` yang
//! membuat compile lintas run menjadi incremental: file yang tidak berubah
//! tidak di-lex/di-parse/di-verifikasi ulang. Terintegrasi otomatis ke
//! `run` dan `run_fast` — bukan flag tambahan.
//!
//! ```text
//! .maria/database/
//!     metadata.mdb      — per-file: hash konten, mtime, size, status, deps
//!     graph.mdb         — dependency graph file-level (CSR + reverse index)
//!     ast.mdb           — Design terserialisasi per file (bincode)
//!     preproc.mdb       — hasil preprocess per file (combined source)
//!     verify.mdb        — verification cache (by content hash)
//!     diagnostics.mdb   — diagnostic per file (query IDE tanpa compile)
//!     symbol.mdb        — index simbol (module/package → file)
//!     types.mdb         — index tipe/signature module
//!     cache/{lexer,parser,semantic,verify,optimize}/
//!     snapshots/build-NNN — snapshot build (rollback)
//! ```

pub mod ast;
pub mod diag;
pub mod format;
pub mod graph;
pub mod metadata;
pub mod snapshot;
pub mod symbol;
pub mod verify;

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use ast::{deserialize_design, serialize_design, AST_FORMAT_VERSION};
pub use diag::{DiagEntry, DiagSeverity, FileDiags};
pub use format::{MdbReader, MdbWriter};
pub use graph::FileGraph;
pub use metadata::{
    FileMeta, FileStatus, MetadataManifest, flags_hash, path_hash,
};
pub use snapshot::{Snapshot, list_snapshots, snapshot_path};
pub use symbol::SymbolIndex;
pub use verify::{VerifyResult, now_ns};

/// Versi compiler yang menulis database.
pub const COMPILER_VERSION: &str = "Maria 0.9";

/// Nama file database.
pub const DB_METADATA: &str = "metadata.mdb";
pub const DB_GRAPH: &str = "graph.mdb";
pub const DB_AST: &str = "ast.mdb";
pub const DB_PREPROC: &str = "preproc.mdb";
pub const DB_VERIFY: &str = "verify.mdb";
pub const DB_DIAG: &str = "diagnostics.mdb";
pub const DB_SYMBOL: &str = "symbol.mdb";
pub const DB_TYPE: &str = "types.mdb";

/// Key singleton untuk store berisi satu objek besar (graph/symbol).
const KEY_SINGLETON: u64 = 0x0000_0000_0000_0001;

/// Entry cache preprocessed (combined source) per file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreprocEntry {
    pub content_hash: u64,
    pub combined: String,
    pub timescale: Option<(String, String)>,
}

/// Statistik MICD setelah `save()`.
#[derive(Debug, Clone, Default)]
pub struct MicdStats {
    /// Jumlah file terdaftar di database.
    pub files: usize,
    /// File yang AST-nya di-restore (parse di-skip).
    pub restored_designs: usize,
    /// File yang berubah pada build ini.
    pub changed_files: usize,
    /// Hit verification cache.
    pub verify_hits: usize,
    /// Miss verification cache.
    pub verify_misses: usize,
    /// ID snapshot terakhir.
    pub snapshot_id: u64,
    /// Bytes yang ditulis ke disk.
    pub bytes_written: u64,
}

/// MICD object database.
pub struct MicdDatabase {
    pub root: PathBuf,
    pub flags_hash: u64,
    pub compiler_version: String,
    pub created_ns: u64,
    /// Per-file metadata (kunci = path).
    pub files: HashMap<PathBuf, FileMeta>,
    /// Dependency graph file-level.
    pub graph: FileGraph,
    /// Verification cache (kunci = content hash).
    pub verify: HashMap<u64, VerifyResult>,
    /// Diagnostic per file.
    pub diags: HashMap<PathBuf, FileDiags>,
    /// Index simbol.
    pub symbols: SymbolIndex,
    /// AST cache: path → (content_hash, bytes bincode).
    pub ast_cache: HashMap<PathBuf, (u64, Vec<u8>)>,
    /// Preprocess cache: path → combined source.
    pub preproc_cache: HashMap<PathBuf, PreprocEntry>,
    /// Index tipe/signature: module → signature hash.
    pub type_index: HashMap<String, u64>,
    /// Ada perubahan belum disimpan.
    pub dirty: bool,
    /// ast.mdb perlu ditulis ulang (ada AST baru/berubah).
    pub dirty_ast: bool,
    /// preproc.mdb perlu ditulis ulang.
    pub dirty_preproc: bool,
    /// Snapshot yang tersedia.
    pub snapshots: Vec<u64>,
    /// Jumlah restore AST pada sesi ini.
    pub restored: usize,
    /// Jumlah file berubah pada sesi ini (kumulatif sejak open).
    pub changed: usize,
    /// Jumlah file berubah pada snapshot terakhir (dedup snapshot).
    pub last_snapshotted_changed: usize,
}

impl MicdDatabase {
    /// Root default: `<cwd>/.maria/database` (override via env MARIA_MICD_DIR).
    pub fn default_root() -> PathBuf {
        std::env::var("MARIA_MICD_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(".maria").join("database"))
    }

    /// Buka database. File corrupt/missing → database kosong (best-effort,
    /// tidak pernah gagal total).
    pub fn open(root: &Path) -> MicdDatabase {
        let mut db = MicdDatabase {
            root: root.to_path_buf(),
            flags_hash: 0,
            compiler_version: COMPILER_VERSION.to_string(),
            created_ns: now_ns(),
            files: HashMap::new(),
            graph: FileGraph::new(),
            verify: HashMap::new(),
            diags: HashMap::new(),
            symbols: SymbolIndex::new(),
            ast_cache: HashMap::new(),
            preproc_cache: HashMap::new(),
            type_index: HashMap::new(),
            dirty: false,
            dirty_ast: false,
            dirty_preproc: false,
            snapshots: Vec::new(),
            restored: 0,
            changed: 0,
            last_snapshotted_changed: 0,
        };

        db.snapshots = list_snapshots(root);

        // metadata.mdb
        if let Ok(r) = MdbReader::open(&root.join(DB_METADATA)) {
            if let Some(manifest) = r.get(KEY_SINGLETON).and_then(|b| {
                bincode::deserialize::<MetadataManifest>(&b).ok()
            }) {
                db.flags_hash = manifest.flags_hash;
                db.compiler_version = manifest.compiler_version;
                db.created_ns = manifest.created_ns;
                for p in manifest.paths {
                    let key = path_hash(&p);
                    if let Some(b) = r.get(key) {
                        if let Ok(meta) = bincode::deserialize::<FileMeta>(&b) {
                            db.files.insert(p, meta);
                        }
                    }
                }
            }
        }

        // graph.mdb
        if let Ok(r) = MdbReader::open(&root.join(DB_GRAPH)) {
            if let Some(b) = r.get(KEY_SINGLETON) {
                if let Ok(g) = bincode::deserialize::<FileGraph>(&b) {
                    db.graph = g;
                }
            }
        }

        // verify.mdb
        if let Ok(r) = MdbReader::open(&root.join(DB_VERIFY)) {
            for (key, _kind) in r.keys() {
                if key == KEY_SINGLETON {
                    continue;
                }
                if let Some(b) = r.get(key) {
                    if let Ok(v) = bincode::deserialize::<VerifyResult>(&b) {
                        db.verify.insert(v.content_hash, v);
                    }
                }
            }
        }

        // diagnostics.mdb
        if let Ok(r) = MdbReader::open(&root.join(DB_DIAG)) {
            for (key, _kind) in r.keys() {
                if key == KEY_SINGLETON {
                    continue;
                }
                if let Some(b) = r.get(key) {
                    if let Ok(d) = bincode::deserialize::<FileDiags>(&b) {
                        db.diags.insert(d.path.clone(), d);
                    }
                }
            }
        }

        // symbol.mdb
        if let Ok(r) = MdbReader::open(&root.join(DB_SYMBOL)) {
            if let Some(b) = r.get(KEY_SINGLETON) {
                if let Ok(s) = bincode::deserialize::<SymbolIndex>(&b) {
                    db.symbols = s;
                }
            }
        }

        // ast.mdb — self-contained: path disimpan di value (tidak bergantung
        // metadata store).
        if let Ok(r) = MdbReader::open(&root.join(DB_AST)) {
            for (key, _kind) in r.keys() {
                if let Some(b) = r.get(key) {
                    if let Ok((path_str, hash, ver, bytes)) =
                        bincode::deserialize::<(String, u64, u64, Vec<u8>)>(&b)
                    {
                        if ver == AST_FORMAT_VERSION {
                            db.ast_cache.insert(PathBuf::from(path_str), (hash, bytes));
                        }
                    }
                }
            }
        }

        // preproc.mdb — self-contained.
        if let Ok(r) = MdbReader::open(&root.join(DB_PREPROC)) {
            for (key, _kind) in r.keys() {
                if let Some(b) = r.get(key) {
                    if let Ok((path_str, hash, p)) =
                        bincode::deserialize::<(String, u64, PreprocEntry)>(&b)
                    {
                        if p.content_hash == hash {
                            db.preproc_cache.insert(PathBuf::from(path_str), p);
                        }
                    }
                }
            }
        }

        // types.mdb (signature index)
        if let Ok(r) = MdbReader::open(&root.join(DB_TYPE)) {
            if let Some(b) = r.get(KEY_SINGLETON) {
                if let Ok(m) = bincode::deserialize::<HashMap<String, u64>>(&b) {
                    db.type_index = m;
                }
            }
        }

        db
    }

    /// Path tersimpan di metadata.
    pub fn known_paths(&self) -> Vec<PathBuf> {
        self.files.keys().cloned().collect()
    }

    pub fn get_file_meta(&self, path: &Path) -> Option<&FileMeta> {
        self.files.get(path)
    }

    /// Apakah AST file `path` valid untuk `content_hash` ini?
    pub fn has_valid_ast(&self, path: &Path, content_hash: u64) -> bool {
        self.ast_cache
            .get(path)
            .map(|(h, _)| *h == content_hash)
            .unwrap_or(false)
    }

    /// Ambil AST terserialisasi bila hash cocok.
    pub fn get_ast(&self, path: &Path, content_hash: u64) -> Option<Vec<u8>> {
        let (h, bytes) = self.ast_cache.get(path)?;
        if *h == content_hash {
            Some(bytes.clone())
        } else {
            None
        }
    }

    pub fn cache_ast(&mut self, path: PathBuf, content_hash: u64, bytes: Vec<u8>) {
        let changed = match self.ast_cache.get(&path) {
            Some((h, b)) => *h != content_hash || *b != bytes,
            None => true,
        };
        self.ast_cache.insert(path, (content_hash, bytes));
        if changed {
            self.dirty = true;
            self.dirty_ast = true;
        }
    }

    /// Ambil combined source bila hash cocok.
    pub fn get_preprocessed(&self, path: &Path, content_hash: u64) -> Option<PreprocEntry> {
        let e = self.preproc_cache.get(path)?;
        if e.content_hash == content_hash {
            Some(e.clone())
        } else {
            None
        }
    }

    pub fn cache_preprocessed(&mut self, path: PathBuf, entry: PreprocEntry) {
        let changed = match self.preproc_cache.get(&path) {
            Some(old) => old.content_hash != entry.content_hash || old.combined != entry.combined,
            None => true,
        };
        self.preproc_cache.insert(path, entry);
        if changed {
            self.dirty = true;
            self.dirty_preproc = true;
        }
    }

    /// Daftarkan hasil kompilasi satu file.
    pub fn record_file(
        &mut self,
        path: PathBuf,
        content_hash: u64,
        deps: Vec<PathBuf>,
        status: FileStatus,
        flags_hash: u64,
        size: u64,
        include_hashes: Vec<(PathBuf, u64)>,
    ) {
        let prev = self.files.get(&path).map(|m| m.content_hash);
        if prev != Some(content_hash) {
            self.changed += 1;
        }
        let meta = FileMeta {
            path: path.clone(),
            content_hash,
            mtime_ns: std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| {
                    t.duration_since(std::time::UNIX_EPOCH).ok()
                })
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
            size,
            status,
            flags_hash,
            deps,
            include_hashes,
            compiled_at_ns: now_ns(),
            ast_format_version: AST_FORMAT_VERSION,
        };
        self.files.insert(path, meta);
        self.dirty = true;
    }

    /// Verifikasi bahwa seluruh dependensi file (termasuk include) tidak
    /// berubah sejak hash `content_hash` tersimpan. `None` jika file tidak
    /// terdaftar. Ini koreksi correctness: preprocessed output meng-embed
    /// konten header — bila header berubah, cache tidak boleh di-reuse.
    pub fn deps_unchanged(&self, path: &Path, content_hash: u64) -> Option<bool> {
        let meta = self.files.get(path)?;
        if meta.content_hash != content_hash {
            return Some(false);
        }
        for (dep, stored_hash) in &meta.include_hashes {
            let current = std::fs::read(dep)
                .ok()
                .map(|b| crate::cache::checksum::compute_checksum(&b));
            match current {
                // Header hilang → tidak bisa diverifikasi → jangan reuse.
                None => return Some(false),
                Some(h) if h != *stored_hash => return Some(false),
                Some(_) => {}
            }
        }
        Some(true)
    }

    pub fn get_verify(&self, content_hash: u64) -> Option<&VerifyResult> {
        self.verify.get(&content_hash)
    }

    pub fn set_verify(&mut self, result: VerifyResult) {
        self.verify.insert(result.content_hash, result);
        self.dirty = true;
    }

    pub fn get_diags(&self, path: &Path) -> Option<&FileDiags> {
        self.diags.get(path)
    }

    pub fn set_diags(&mut self, diags: FileDiags) {
        self.diags.insert(diags.path.clone(), diags);
        self.dirty = true;
    }

    /// File yang terdampak bila `changed` berubah (via dependency graph).
    pub fn affected(&mut self, changed: &[PathBuf]) -> Vec<PathBuf> {
        self.graph.affected(changed)
    }

    /// Simpan seluruh store ke disk (atomik per file).
    pub fn save(&mut self) -> io::Result<MicdStats> {
        if !self.dirty {
            let stats = MicdStats {
                files: self.files.len(),
                restored_designs: self.restored,
                changed_files: self.changed,
                verify_hits: self.verify_hits(),
                verify_misses: self.verify_misses(),
                snapshot_id: self.snapshots.last().copied().unwrap_or(0),
                bytes_written: 0,
            };
            return Ok(stats);
        }

        std::fs::create_dir_all(&self.root)?;
        for sub in ["cache/lexer", "cache/parser", "cache/semantic", "cache/verify", "cache/optimize"] {
            let _ = std::fs::create_dir_all(self.root.join(sub));
        }

        let mut bytes_written = 0u64;

        // metadata.mdb
        let manifest = MetadataManifest {
            paths: self.files.keys().cloned().collect(),
            flags_hash: self.flags_hash,
            compiler_version: self.compiler_version.clone(),
            created_ns: self.created_ns,
        };
        {
            let mut w = MdbWriter::new();
            w.put(
                KEY_SINGLETON,
                format::KIND_MANIFEST,
                bincode::serialize(&manifest).map_err(io::Error::other)?,
            );
            for (path, meta) in self.files.iter() {
                let key = path_hash(path);
                w.put(
                    key,
                    format::KIND_META,
                    bincode::serialize(meta).map_err(io::Error::other)?,
                );
            }
            let bytes = w.serialize().len() as u64;
            w.write_to(&self.root.join(DB_METADATA))?;
            bytes_written += bytes;
        }

        // graph.mdb — rebuild reverse index sebelum serialize (set_deps
        // tidak rebuild per-call).
        self.graph.rebuild();
        {
            let mut w = MdbWriter::new();
            w.put(
                KEY_SINGLETON,
                format::KIND_GRAPH,
                bincode::serialize(&self.graph).map_err(io::Error::other)?,
            );
            let bytes = w.serialize().len() as u64;
            w.write_to(&self.root.join(DB_GRAPH))?;
            bytes_written += bytes;
        }

        // verify.mdb
        {
            let mut w = MdbWriter::new();
            for (_, v) in self.verify.iter() {
                w.put(
                    v.content_hash,
                    format::KIND_VERIFY,
                    bincode::serialize(v).map_err(io::Error::other)?,
                );
            }
            let bytes = w.serialize().len() as u64;
            w.write_to(&self.root.join(DB_VERIFY))?;
            bytes_written += bytes;
        }

        // diagnostics.mdb
        {
            let mut w = MdbWriter::new();
            for (path, d) in self.diags.iter() {
                w.put(
                    path_hash(path),
                    format::KIND_DIAG,
                    bincode::serialize(d).map_err(io::Error::other)?,
                );
            }
            let bytes = w.serialize().len() as u64;
            w.write_to(&self.root.join(DB_DIAG))?;
            bytes_written += bytes;
        }

        // symbol.mdb
        {
            let mut w = MdbWriter::new();
            w.put(
                KEY_SINGLETON,
                format::KIND_SYMBOL,
                bincode::serialize(&self.symbols).map_err(io::Error::other)?,
            );
            let bytes = w.serialize().len() as u64;
            w.write_to(&self.root.join(DB_SYMBOL))?;
            bytes_written += bytes;
        }

        // types.mdb
        {
            let mut w = MdbWriter::new();
            w.put(
                KEY_SINGLETON,
                format::KIND_TYPE,
                bincode::serialize(&self.type_index).map_err(io::Error::other)?,
            );
            let bytes = w.serialize().len() as u64;
            w.write_to(&self.root.join(DB_TYPE))?;
            bytes_written += bytes;
        }

        // ast.mdb — hanya ditulis bila ada AST baru/berubah (warm run dengan
        // semua file di-restore tidak menyentuh file ini).
        if self.dirty_ast {
            let mut w = MdbWriter::new();
            for (path, (hash, bytes)) in self.ast_cache.iter() {
                let path_str = path.to_string_lossy().to_string();
                let val = bincode::serialize(&(path_str, *hash, AST_FORMAT_VERSION, bytes))
                    .map_err(io::Error::other)?;
                w.put(path_hash(path), format::KIND_AST, val);
            }
            let bytes = w.serialize().len() as u64;
            w.write_to(&self.root.join(DB_AST))?;
            bytes_written += bytes;
        }

        // preproc.mdb — hanya ditulis bila ada perubahan.
        if self.dirty_preproc {
            let mut w = MdbWriter::new();
            for (path, entry) in self.preproc_cache.iter() {
                let path_str = path.to_string_lossy().to_string();
                let val = bincode::serialize(&(path_str, entry.content_hash, entry))
                    .map_err(io::Error::other)?;
                w.put(path_hash(path), format::KIND_PREPROC, val);
            }
            let bytes = w.serialize().len() as u64;
            w.write_to(&self.root.join(DB_PREPROC))?;
            bytes_written += bytes;
        }

        let stats = MicdStats {
            files: self.files.len(),
            restored_designs: self.restored,
            changed_files: self.changed,
            verify_hits: self.verify_hits(),
            verify_misses: self.verify_misses(),
            snapshot_id: self.snapshots.last().copied().unwrap_or(0),
            bytes_written,
        };
        self.dirty = false;
        self.dirty_ast = false;
        self.dirty_preproc = false;
        self.changed = 0;
        Ok(stats)
    }

    /// Buat snapshot build baru (mirip commit). Menyimpan state saat ini.
    pub fn snapshot(&mut self, note: String) -> io::Result<u64> {
        let id = self.snapshots.last().copied().unwrap_or(0) + 1;
        let snap = Snapshot {
            id,
            created_ns: now_ns(),
            files: self.files.values().cloned().collect(),
            graph: self.graph.clone(),
            verify: self.verify.iter().map(|(k, v)| (*k, v.clone())).collect(),
            symbols: self.symbols.clone(),
            diags: self.diags.values().cloned().collect(),
            flags_hash: self.flags_hash,
            note,
        };
        let path = snapshot_path(&self.root, id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = bincode::serialize(&snap).map_err(io::Error::other)?;
        std::fs::write(&path, bytes)?;
        self.snapshots.push(id);
        // Pertahankan maksimal 16 snapshot.
        while self.snapshots.len() > 16 {
            if let Some(old) = self.snapshots.first().copied() {
                let _ = std::fs::remove_file(snapshot_path(&self.root, old));
                self.snapshots.remove(0);
            }
        }
        Ok(id)
    }

    /// Rollback ke snapshot `id`: restore metadata + graph + verify + symbols.
    /// AST cache dibiarkan (objek yang tidak cocok diabaikan otomatis).
    pub fn rollback(&mut self, id: u64) -> io::Result<()> {
        let path = snapshot_path(&self.root, id);
        let bytes = std::fs::read(&path)?;
        let snap: Snapshot = bincode::deserialize(&bytes).map_err(io::Error::other)?;
        self.files = snap
            .files
            .into_iter()
            .map(|m| (m.path.clone(), m))
            .collect();
        self.graph = snap.graph;
        self.verify = snap.verify.into_iter().collect();
        self.symbols = snap.symbols;
        self.diags = snap
            .diags
            .into_iter()
            .map(|d| (d.path.clone(), d))
            .collect();
        self.flags_hash = snap.flags_hash;
        self.dirty = true;
        Ok(())
    }

    /// Bersihkan seluruh database.
    pub fn clear(&mut self) -> io::Result<()> {
        for name in [
            DB_METADATA,
            DB_GRAPH,
            DB_AST,
            DB_PREPROC,
            DB_VERIFY,
            DB_DIAG,
            DB_SYMBOL,
            DB_TYPE,
        ] {
            let _ = std::fs::remove_file(self.root.join(name));
        }
        let _ = std::fs::remove_dir_all(self.root.join("snapshots"));
        let _ = std::fs::remove_dir_all(self.root.join("cache"));
        self.files.clear();
        self.graph = FileGraph::new();
        self.verify.clear();
        self.diags.clear();
        self.symbols = SymbolIndex::new();
        self.ast_cache.clear();
        self.preproc_cache.clear();
        self.type_index.clear();
        self.snapshots.clear();
        self.dirty = false;
        self.dirty_ast = false;
        self.dirty_preproc = false;
        Ok(())
    }

    fn verify_hits(&self) -> usize {
        self.verify.values().filter(|v| v.ok()).count()
    }
    fn verify_misses(&self) -> usize {
        self.files.len().saturating_sub(self.verify_hits())
    }

    /// Statistik ringkas untuk CLI.
    pub fn summary(&self) -> String {
        format!(
            "files={} restored_ast={} changed={} verify_hits={} snapshots={}",
            self.files.len(),
            self.restored,
            self.changed,
            self.verify_hits(),
            self.snapshots.len()
        )
    }
}

/// Kebalikan path_hash tidak bisa; key numerik tidak perlu dipetakan kembali
/// karena ast/preproc store di-load dengan iterasi metadata store (path →
/// key → objek).

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "maria_micd_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn test_open_empty_db() {
        let root = test_root("empty");
        let db = MicdDatabase::open(&root);
        assert!(db.files.is_empty());
        assert!(db.graph.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_save_reload_roundtrip() {
        let root = test_root("roundtrip");
        let path = PathBuf::from("test/counter.sv");
        let hash = 0x1234_5678;
        {
            let mut db = MicdDatabase::open(&root);
            db.flags_hash = 42;
            db.record_file(path.clone(), hash, vec![], FileStatus::Unchanged, 42, 100, vec![]);
            db.cache_ast(path.clone(), hash, vec![1, 2, 3, 4]);
            db.cache_preprocessed(
                path.clone(),
                PreprocEntry {
                    content_hash: hash,
                    combined: "`line 1 \"test/counter.sv\"\nmodule c;\nendmodule".into(),
                    timescale: None,
                },
            );
            db.set_verify(VerifyResult {
                content_hash: hash,
                parse_ok: true,
                elab_ok: true,
                err_count: 0,
                warn_count: 0,
                info_count: 0,
                parse_ms: 5,
                elab_ms: 3,
                result_hash: 7,
                verified_at_ns: 0,
            });
            db.symbols.add("counter".into(), "module".into(), path.clone());
            db.save().unwrap();
        }
        {
            let mut db = MicdDatabase::open(&root);
            assert_eq!(db.files.len(), 1);
            let meta = db.get_file_meta(&path).unwrap();
            assert_eq!(meta.content_hash, hash);
            assert!(db.has_valid_ast(&path, hash));
            assert_eq!(db.get_ast(&path, hash).unwrap(), vec![1, 2, 3, 4]);
            assert!(!db.has_valid_ast(&path, 999));
            assert_eq!(
                db.get_preprocessed(&path, hash).unwrap().combined,
                "`line 1 \"test/counter.sv\"\nmodule c;\nendmodule"
            );
            assert_eq!(db.get_verify(hash).unwrap().result_hash, 7);
            assert!(db.symbols.locate("counter", "module").is_some());
            let id = db.snapshot("build1".into()).unwrap();
            assert_eq!(id, 1);
            db.save().unwrap();
        }
        {
            let db = MicdDatabase::open(&root);
            assert_eq!(db.snapshots, vec![1]);
            assert!(snapshot_path(&root, 1).exists());
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_rollback_restores_state() {
        let root = test_root("rollback");
        let path = PathBuf::from("a.sv");
        {
            let mut db = MicdDatabase::open(&root);
            db.record_file(path.clone(), 111, vec![], FileStatus::Unchanged, 0, 10, vec![]);
            db.save().unwrap();
            let id = db.snapshot("s1".into()).unwrap();
            assert_eq!(id, 1);
            db.record_file(path.clone(), 222, vec![], FileStatus::Recompiled, 0, 10, vec![]);
            db.save().unwrap();
            assert_eq!(db.get_file_meta(&path).unwrap().content_hash, 222);
            db.rollback(1).unwrap();
            db.save().unwrap();
        }
        {
            let db = MicdDatabase::open(&root);
            assert_eq!(db.get_file_meta(&path).unwrap().content_hash, 111);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_affected_via_graph() {
        let root = test_root("graph");
        let mut db = MicdDatabase::open(&root);
        db.graph.set_deps(PathBuf::from("cpu.sv"), vec![PathBuf::from("uart.sv")]);
        db.graph.set_deps(PathBuf::from("uart.sv"), vec![PathBuf::from("defines.svh")]);
        let affected = db.affected(&[PathBuf::from("defines.svh")]);
        assert!(affected.contains(&PathBuf::from("cpu.sv")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
