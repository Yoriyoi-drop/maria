//! Remote Cache — pluggable backend untuk cache bersama (shared across CI builds).
//!
//! Content-addressed storage: setiap entry disimpan berdasarkan checksum datanya.
//! `RemoteCacheBackend` trait memungkinkan implementasi backend berbeda
//! (filesystem, S3, GCS, HTTP, etc).
//!
//! ## FilesystemCache
//!
//! Layout direktori:
//!   <root>/<cache_type>/<prefix>/<key_hash_hex>.dat   — data bytes
//!   <root>/<cache_type>/<prefix>/<key_hash_hex>.meta   — JSON metadata
//!
//! Dengan prefix = 2 karakter hex pertama dari key_hash (untuk menghindari
//! direktori terlalu besar).
//!
//! ## Penggunaan
//!
//! ```rust,ignore
//! let backend = FilesystemCache::new("/mnt/shared/cache/maria");
//! let key = CacheKey::FileContent(12345);
//! backend.put(&key, b"cache data").unwrap();
//! if let Some(data) = backend.get(&key).unwrap() {
//!     println!("Got {} bytes from remote cache", data.len());
//! }
//! ```

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use super::cache_manager::CacheKey;
use super::checksum::{combine_checksum, compute_checksum};

// ─── Cache Error ───

/// Error type for cache operations.
#[derive(Debug)]
pub enum CacheError {
    /// I/O error (disk, network, etc.)
    Io(std::io::Error),
    /// Serialization/deserialization error
    Format(String),
    /// Entry not found
    NotFound,
    /// Backend-specific error
    Backend(String),
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CacheError::Io(e) => write!(f, "cache I/O error: {}", e),
            CacheError::Format(s) => write!(f, "cache format error: {}", s),
            CacheError::NotFound => write!(f, "cache entry not found"),
            CacheError::Backend(s) => write!(f, "cache backend error: {}", s),
        }
    }
}

impl From<std::io::Error> for CacheError {
    fn from(e: std::io::Error) -> Self {
        CacheError::Io(e)
    }
}

// ─── RemoteCacheBackend Trait ───

/// Pluggable remote cache backend.
///
/// Content-addressed: data disimpan dan diambil berdasarkan `CacheKey`.
/// Setiap backend harus thread-safe (`Send + Sync`).
pub trait RemoteCacheBackend: Send + Sync {
    /// Get cached data for a key.
    fn get(&self, key: &CacheKey) -> Result<Option<Vec<u8>>, CacheError>;

    /// Store data for a key.
    fn put(&self, key: &CacheKey, data: &[u8]) -> Result<(), CacheError>;

    /// Check if key exists in cache.
    fn contains(&self, key: &CacheKey) -> Result<bool, CacheError>;

    /// Remove entry for a key.
    fn remove(&self, key: &CacheKey) -> Result<(), CacheError>;

    /// Clear all entries.
    fn clear(&self) -> Result<(), CacheError>;

    /// Get statistics for this backend.
    fn stats(&self) -> RemoteCacheStats;

    /// Human-readable name for this backend type.
    fn backend_name(&self) -> &'static str;
}

/// Statistics for remote cache backend.
#[derive(Debug, Clone, Default)]
pub struct RemoteCacheStats {
    pub entries: usize,
    pub size_bytes: u64,
    pub hits: u64,
    pub misses: u64,
    pub puts: u64,
}

impl std::fmt::Display for RemoteCacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} entries, {:.1}KB, {} hits, {} misses, {} puts",
            self.entries,
            self.size_bytes as f64 / 1024.0,
            self.hits,
            self.misses,
            self.puts,
        )
    }
}

// ─── Cache Key Metadata (JSON) ───

/// Metadata untuk entry di remote cache (disimpan sebagai JSON).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CacheEntryMeta {
    /// Cache key type name
    key_type: String,
    /// Key hash
    key_hash: u64,
    /// Data checksum (xxhash3-64)
    data_checksum: u64,
    /// Data size in bytes
    data_size: u64,
    /// Timestamp of creation (unix ns)
    created_ns: u64,
    /// Cache type identifier (ast, hir, inc, mac, pkg)
    cache_type: String,
}

// ─── Key Hash Helper ───

/// Compute a unique hash for a CacheKey, used as filename.
pub fn cache_key_hash(key: &CacheKey) -> u64 {
    match key {
        CacheKey::FileContent(c) => *c,
        CacheKey::FilePath(s) => compute_checksum(s.as_str().as_bytes()),
        CacheKey::Module {
            name,
            param_hash,
            dependency_hash,
        } => combine_checksum(
            compute_checksum(name.as_str().as_bytes()),
            combine_checksums(*param_hash, *dependency_hash),
        ),
        CacheKey::Package(s) => compute_checksum(s.as_str().as_bytes()),
        CacheKey::Macro {
            name,
            arg_hash,
            definition_hash,
        } => combine_checksum(
            combine_checksums(
                compute_checksum(name.as_str().as_bytes()),
                *arg_hash,
            ),
            *definition_hash,
        ),
        CacheKey::Include {
            resolved_path,
            content_hash,
        } => combine_checksums(
            compute_checksum(resolved_path.as_str().as_bytes()),
            *content_hash,
        ),
    }
}

/// Short cache type directory name for a CacheKey.
pub fn cache_key_type_dir(key: &CacheKey) -> &'static str {
    match key {
        CacheKey::FileContent(_) | CacheKey::FilePath(_) => "ast",
        CacheKey::Module { .. } => "hir",
        CacheKey::Package(_) => "pkg",
        CacheKey::Macro { .. } => "mac",
        CacheKey::Include { .. } => "inc",
    }
}

fn combine_checksums(a: u64, b: u64) -> u64 {
    a.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(b) ^ (b >> 31)
}

// ─── FilesystemCache ───

/// Content-addressed filesystem cache.
///
/// Layout:
///   <root>/<cache_type>/<2-char prefix>/<key_hash_hex>.dat   — data
///   <root>/<cache_type>/<2-char prefix>/<key_hash_hex>.meta   — JSON metadata
///
/// Thread-safe via `Mutex` pada operasi tulis (atomic file rename).
pub struct FilesystemCache {
    root: PathBuf,
    stats_hits: AtomicU64,
    stats_misses: AtomicU64,
    stats_puts: AtomicU64,
    stats_entries: AtomicUsize,
    stats_size: AtomicU64,
}

impl FilesystemCache {
    /// Create a new filesystem cache at the given root directory.
    ///
    /// Creates the directory structure if it doesn't exist.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, CacheError> {
        let root: PathBuf = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(FilesystemCache {
            root,
            stats_hits: AtomicU64::new(0),
            stats_misses: AtomicU64::new(0),
            stats_puts: AtomicU64::new(0),
            stats_entries: AtomicUsize::new(0),
            stats_size: AtomicU64::new(0),
        })
    }

    /// Resolve path for data file.
    fn data_path(&self, key: &CacheKey) -> PathBuf {
        let hash = cache_key_hash(key);
        let type_dir = cache_key_type_dir(key);
        let prefix = format!("{:02x}", (hash >> 56) as u8);
        self.root
            .join(type_dir)
            .join(&prefix)
            .join(format!("{:016x}.dat", hash))
    }

    /// Resolve path for metadata file.
    fn meta_path(&self, key: &CacheKey) -> PathBuf {
        let hash = cache_key_hash(key);
        let type_dir = cache_key_type_dir(key);
        let prefix = format!("{:02x}", (hash >> 56) as u8);
        self.root
            .join(type_dir)
            .join(&prefix)
            .join(format!("{:016x}.meta", hash))
    }

    /// Read metadata file.
    fn read_meta(&self, key: &CacheKey) -> Result<Option<CacheEntryMeta>, CacheError> {
        let path = self.meta_path(key);
        if !path.exists() {
            return Ok(None);
        }
        let json_str = std::fs::read_to_string(&path)?;
        let meta: CacheEntryMeta =
            serde_json::from_str(&json_str).map_err(|e| CacheError::Format(e.to_string()))?;
        Ok(Some(meta))
    }

    /// Write metadata file.
    fn write_meta(&self, key: &CacheKey, data: &[u8]) -> Result<(), CacheError> {
        let path = self.meta_path(key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let hash = cache_key_hash(key);
        let meta = CacheEntryMeta {
            key_type: format!("{:?}", key),
            key_hash: hash,
            data_checksum: compute_checksum(data),
            data_size: data.len() as u64,
            created_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
            cache_type: cache_key_type_dir(key).to_string(),
        };
        let json_str =
            serde_json::to_string(&meta).map_err(|e| CacheError::Format(e.to_string()))?;
        // Write atomically via temp file + rename
        let tmp_path = path.with_extension("meta.tmp");
        std::fs::write(&tmp_path, &json_str)?;
        std::fs::rename(&tmp_path, &path)?;
        Ok(())
    }
}

impl RemoteCacheBackend for FilesystemCache {
    fn get(&self, key: &CacheKey) -> Result<Option<Vec<u8>>, CacheError> {
        let path = self.data_path(key);
        if !path.exists() {
            self.stats_misses.fetch_add(1, Ordering::Relaxed);
            return Ok(None);
        }
        let data = std::fs::read(&path)?;
        // Verify checksum from metadata
        if let Some(meta) = self.read_meta(key)? {
            let actual_checksum = compute_checksum(&data);
            if actual_checksum != meta.data_checksum {
                // Corrupted entry — remove and return None
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::remove_file(self.meta_path(key));
                self.stats_misses.fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
        }
        self.stats_hits.fetch_add(1, Ordering::Relaxed);
        Ok(Some(data))
    }

    fn put(&self, key: &CacheKey, data: &[u8]) -> Result<(), CacheError> {
        let data_path = self.data_path(key);
        if let Some(parent) = data_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Write atomically via temp file + rename
        let tmp_path = data_path.with_extension("dat.tmp");
        std::fs::write(&tmp_path, data)?;
        std::fs::rename(&tmp_path, &data_path)?;
        // Write metadata
        self.write_meta(key, data)?;
        self.stats_puts.fetch_add(1, Ordering::Relaxed);
        self.stats_entries.fetch_add(1, Ordering::Relaxed);
        self.stats_size.fetch_add(data.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    fn contains(&self, key: &CacheKey) -> Result<bool, CacheError> {
        Ok(self.data_path(key).exists())
    }

    fn remove(&self, key: &CacheKey) -> Result<(), CacheError> {
        let data_path = self.data_path(key);
        let meta_path = self.meta_path(key);
        let removed = data_path.exists();
        let _ = std::fs::remove_file(&data_path);
        let _ = std::fs::remove_file(&meta_path);
        if removed {
            self.stats_entries.fetch_sub(1, Ordering::Relaxed);
        }
        Ok(())
    }

    fn clear(&self) -> Result<(), CacheError> {
        // Remove all cache type directories
        for cache_type in &["ast", "hir", "pkg", "mac", "inc"] {
            let dir = self.root.join(cache_type);
            if dir.exists() {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
        self.stats_entries.store(0, Ordering::Relaxed);
        self.stats_size.store(0, Ordering::Relaxed);
        Ok(())
    }

    fn stats(&self) -> RemoteCacheStats {
        RemoteCacheStats {
            entries: self.stats_entries.load(Ordering::Relaxed),
            size_bytes: self.stats_size.load(Ordering::Relaxed),
            hits: self.stats_hits.load(Ordering::Relaxed),
            misses: self.stats_misses.load(Ordering::Relaxed),
            puts: self.stats_puts.load(Ordering::Relaxed),
        }
    }

    fn backend_name(&self) -> &'static str {
        "filesystem"
    }
}

impl std::fmt::Debug for FilesystemCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilesystemCache")
            .field("root", &self.root)
            .field("stats", &self.stats())
            .finish()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intern::Symbol;

    fn test_backend() -> FilesystemCache {
        let pid = std::process::id();
        let tid = std::thread::current().id();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir()
            .join(format!("maria_cache_test_{}_{:?}_{}", pid, tid, ts));
        let _ = std::fs::remove_dir_all(&dir);
        FilesystemCache::new(&dir).unwrap()
    }

    #[test]
    fn test_filesystem_cache_put_get() {
        let cache = test_backend();
        let key = CacheKey::FileContent(42);
        let data = b"hello remote cache";

        cache.put(&key, data).unwrap();
        let result = cache.get(&key).unwrap();
        assert_eq!(result, Some(data.to_vec()));
    }

    #[test]
    fn test_filesystem_cache_miss() {
        let cache = test_backend();
        let key = CacheKey::FileContent(999);
        let result = cache.get(&key).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_filesystem_cache_contains() {
        let cache = test_backend();
        let key = CacheKey::FileContent(100);

        assert!(!cache.contains(&key).unwrap());
        cache.put(&key, b"data").unwrap();
        assert!(cache.contains(&key).unwrap());
    }

    #[test]
    fn test_filesystem_cache_remove() {
        let cache = test_backend();
        let key = CacheKey::FileContent(200);

        cache.put(&key, b"data").unwrap();
        assert!(cache.contains(&key).unwrap());
        cache.remove(&key).unwrap();
        assert!(!cache.contains(&key).unwrap());
    }

    #[test]
    fn test_filesystem_cache_clear() {
        let cache = test_backend();

        cache.put(&CacheKey::FileContent(1), b"a").unwrap();
        cache.put(&CacheKey::FileContent(2), b"b").unwrap();
        assert_eq!(cache.stats().entries, 2);

        cache.clear().unwrap();
        assert_eq!(cache.stats().entries, 0);
    }

    #[test]
    fn test_filesystem_cache_overwrite() {
        let cache = test_backend();
        let key = CacheKey::FileContent(300);

        cache.put(&key, b"original").unwrap();
        assert_eq!(cache.get(&key).unwrap(), Some(b"original".to_vec()));

        cache.put(&key, b"updated").unwrap();
        assert_eq!(cache.get(&key).unwrap(), Some(b"updated".to_vec()));
    }

    #[test]
    fn test_filesystem_cache_corruption_detection() {
        let cache = test_backend();
        let key = CacheKey::FileContent(400);

        cache.put(&key, b"valid data").unwrap();

        // Corrupt the data file
        let data_path = cache.data_path(&key);
        std::fs::write(&data_path, b"corrupted").unwrap();

        // Should detect checksum mismatch and return None
        let result = cache.get(&key).unwrap();
        assert!(result.is_none(), "corrupted entry should return None");
    }

    #[test]
    fn test_cache_key_hash_uniqueness() {
        let k1 = CacheKey::FileContent(1);
        let k2 = CacheKey::FileContent(2);
        let k3 = CacheKey::FilePath(Symbol::intern("hello"));
        let k4 = CacheKey::FilePath(Symbol::intern("world"));

        assert_ne!(cache_key_hash(&k1), cache_key_hash(&k2));
        assert_ne!(cache_key_hash(&k3), cache_key_hash(&k4));
        assert_eq!(cache_key_hash(&k1), cache_key_hash(&k1));
    }

    #[test]
    fn test_cache_key_type_dir() {
        assert_eq!(cache_key_type_dir(&CacheKey::FileContent(1)), "ast");
        assert_eq!(
            cache_key_type_dir(&CacheKey::Module {
                name: Symbol::intern("test"),
                param_hash: 0,
                dependency_hash: 0,
            }),
            "hir"
        );
        assert_eq!(cache_key_type_dir(&CacheKey::Package(Symbol::intern("p"))), "pkg");
        assert_eq!(
            cache_key_type_dir(&CacheKey::Macro {
                name: Symbol::intern("m"),
                arg_hash: 0,
                definition_hash: 0,
            }),
            "mac"
        );
        assert_eq!(
            cache_key_type_dir(&CacheKey::Include {
                resolved_path: Symbol::intern("f"),
                content_hash: 1,
            }),
            "inc"
        );
    }

    #[test]
    fn test_filesystem_cache_multiple_types() {
        let cache = test_backend();
        let ast_key = CacheKey::FileContent(1);
        let hir_key = CacheKey::Module {
            name: Symbol::intern("mod"),
            param_hash: 0,
            dependency_hash: 0,
        };

        cache.put(&ast_key, b"ast data").unwrap();
        cache.put(&hir_key, b"hir data").unwrap();

        assert_eq!(cache.get(&ast_key).unwrap(), Some(b"ast data".to_vec()));
        assert_eq!(cache.get(&hir_key).unwrap(), Some(b"hir data".to_vec()));
    }

    #[test]
    fn test_filesystem_cache_persistence() {
        let dir = std::env::temp_dir().join("maria_remote_cache_persist");
        let _ = std::fs::remove_dir_all(&dir);

        // First instance: write
        {
            let cache = FilesystemCache::new(&dir).unwrap();
            cache.put(&CacheKey::FileContent(500), b"persistent").unwrap();
        }

        // Second instance: read back
        {
            let cache = FilesystemCache::new(&dir).unwrap();
            let result = cache.get(&CacheKey::FileContent(500)).unwrap();
            assert_eq!(result, Some(b"persistent".to_vec()));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
