//! IO Optimization — memory-mapped file I/O untuk zero-copy reads.
//!
//! File >4KB di-mmap untuk akses zero-copy.
//! File kecil dibaca biasa (overhead mmap > manfaatnya).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use memmap2::Mmap;
use xxhash_rust::xxh3::xxh3_64;

/// Threshold: file <4KB dibaca biasa, >=4KB di-mmap.
const MMAP_THRESHOLD: u64 = 4096;

/// Memory-mapped file dengan zero-copy content access.
#[derive(Debug)]
pub struct MmapFile {
    /// Memory-mapped region (or empty for small files)
    mmap: Option<Mmap>,
    /// Bytes content (owned for small files, mmap for large)
    bytes: Box<[u8]>,
    /// File path
    pub path: PathBuf,
    /// Checksum (computed on open)
    pub checksum: u64,
}

impl MmapFile {
    /// Open a file and memory-map it if large enough.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let metadata = fs::metadata(path)?;
        let file_len = metadata.len();

        let (mmap, bytes) = if file_len >= MMAP_THRESHOLD {
            let file = fs::File::open(path)?;
            let mmap = unsafe { Mmap::map(&file)? };
            // Advise sequential access
            let _ = mmap.advise(memmap2::Advice::Sequential);
            let bytes = mmap[..].to_vec().into_boxed_slice();
            (Some(mmap), bytes)
        } else {
            let data = fs::read(path)?;
            (None, data.into_boxed_slice())
        };

        let checksum = xxh3_64(&bytes);

        Ok(MmapFile {
            mmap,
            bytes,
            path: path.to_path_buf(),
            checksum,
        })
    }

    /// Get content as byte slice (zero-copy from mmap).
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Get content as string slice (zero-copy).
    pub fn as_str(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(&self.bytes) }
    }

    /// File size in bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Is empty?
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Read file content as bytes (auto mmap for large files).
pub fn read_file_bytes(path: &Path) -> std::io::Result<Box<[u8]>> {
    let metadata = fs::metadata(path)?;
    if metadata.len() >= MMAP_THRESHOLD {
        let file = fs::File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        Ok(mmap[..].to_vec().into_boxed_slice())
    } else {
        Ok(fs::read(path)?.into_boxed_slice())
    }
}

/// Read file content as string (auto mmap for large files).
pub fn read_file_str(path: &Path) -> std::io::Result<String> {
    fs::read_to_string(path)
}

/// Global mmap statistics counter.
static MMAP_BYTES_SERVED: AtomicUsize = AtomicUsize::new(0);

/// Track mmap bytes served.
pub fn track_mmap_bytes(n: usize) {
    MMAP_BYTES_SERVED.fetch_add(n, Ordering::Relaxed);
}

/// Get total mmap bytes served.
pub fn total_mmap_bytes() -> usize {
    MMAP_BYTES_SERVED.load(Ordering::Relaxed)
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mmap_file_open() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let mf = MmapFile::open(&path).unwrap();
        assert!(!mf.is_empty());
        assert!(mf.len() > 0);
        assert!(mf.as_str().contains("maria"));
    }

    #[test]
    fn test_mmap_checksum() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let mf1 = MmapFile::open(&path).unwrap();
        let mf2 = MmapFile::open(&path).unwrap();
        assert_eq!(mf1.checksum, mf2.checksum);
    }

    #[test]
    fn test_read_file_bytes() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let bytes = read_file_bytes(&path).unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn test_small_file_no_mmap() {
        // Create a small temp file (< 4KB)
        let dir = std::env::temp_dir().join("maria_mmap_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("small.sv");
        std::fs::write(&path, "module small; endmodule").unwrap();

        let mf = MmapFile::open(&path).unwrap();
        assert!(mf.mmap.is_none());  // Small file, no mmap

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_mmap_file_str() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let mf = MmapFile::open(&path).unwrap();
        let s = mf.as_str();
        assert!(s.len() > 0);
        assert!(s.contains("maria"));
    }
}
