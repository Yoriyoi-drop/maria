//! lock.rs — concurrency model MICD (Kritik 7 db.md).
//!
//! Siapa reader, siapa writer:
//!
//! * **Reader** (IDE, LSP, GUI, compiler saat `open()`): TANPA lock.
//!   Aman karena setiap store `.mdb` ditulis atomik (temp + rename) dan
//!   pembaca menahan mmap — pembaca selalu melihat file utuh (versi lama
//!   atau baru, tidak pernah setengah).
//! * **Writer** (compiler saat `save()`): LOCK EXCLUSIVE via lockfile.
//!   Dua writer tidak boleh menulis berbarengan (race pada multiple-store
//!   commit). Writer menunggu (poll) sampai lock tersedia atau timeout.
//!
//! Lock adalah advisory file lock: `locks/<pid>.lock` dibuat atomik
//! (`create_new` = O_EXCL). Bila sudah ada, writer lain menunggu. Lock yang
//! basi (lebih tua dari `stale_after`) dianggap milik proses yang mati dan
//! dibersihkan agar tidak deadlock selamanya.
//!
//! Model ini menjawab Kritik 7: tidak ada deadlock karena lock selalu
//! sequential (satu file, pemilik menunggu), dan tidak ada lock read yang
//! bisa tersangkut (reader tidak pernah lock).

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Nama file lock default relatif terhadap database root.
pub const LOCK_FILE: &str = "locks/writer.lock";

/// Error saat memperoleh lock.
#[derive(Debug)]
pub enum LockError {
    /// Timeout menunggu writer lain selesai.
    Timeout(PathBuf),
    /// I/O gagal membuat lockfile.
    Io(io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Timeout(p) => {
                write!(f, "MICD lock timeout menunggu: {}", p.display())
            }
            LockError::Io(e) => write!(f, "MICD lock I/O: {}", e),
        }
    }
}

impl std::error::Error for LockError {}

/// Guard lock writer. Melepas lock (hapus lockfile) saat di-drop.
pub struct WriteLock {
    path: PathBuf,
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Peroleh lock writer exclusive pada `path` (path lengkap file lock, mis.
/// `locks/<pid>.lock`). Menunggu sampai `timeout`, polling setiap
/// `poll_interval`. Lock basi (umur > `stale_after`) dianggap mati dan
/// diambil alih.
pub fn acquire_write_lock(
    path: &Path,
    timeout: Duration,
    poll_interval: Duration,
    stale_after: Duration,
) -> Result<WriteLock, LockError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(LockError::Io)?;
    }
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match std::fs::File::options()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(_) => {
                return Ok(WriteLock {
                    path: path.to_path_buf(),
                });
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                // Lock basi? Metadata mtime jauh di masa lalu → proses pemilik
                // mati. Ambil alih.
                if let Ok(meta) = std::fs::metadata(&path) {
                    if let Ok(modified) = meta.modified() {
                        if modified.elapsed().unwrap_or_default() > stale_after {
                            let _ = std::fs::remove_file(&path);
                            continue;
                        }
                    }
                }
                if std::time::Instant::now() >= deadline {
                    return Err(LockError::Timeout(path.to_path_buf()));
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => return Err(LockError::Io(e)),
        }
    }
}

/// Adakah lock writer sedang aktif pada `path` (path lengkap file lock)?
pub fn is_writer_locked(path: &Path) -> bool {
    path.exists()
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "maria_micd_lock_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn lockfile(root: &Path) -> PathBuf {
        root.join("locks").join("test.lock")
    }

    #[test]
    fn test_acquire_and_release() {
        let root = test_root("basic");
        let path = lockfile(&root);
        {
            let lock = acquire_write_lock(&path, Duration::from_millis(100), Duration::from_millis(5), Duration::from_secs(1)).unwrap();
            assert!(is_writer_locked(&path));
            drop(lock);
        }
        assert!(!is_writer_locked(&path), "lock dilepas saat guard drop");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_exclusive_between_writers() {
        let root = test_root("excl");
        let path = lockfile(&root);
        let _lock1 = acquire_write_lock(&path, Duration::from_millis(100), Duration::from_millis(5), Duration::from_secs(1)).unwrap();
        // Writer kedua tidak bisa masuk selagi yang pertama aktif.
        let err = acquire_write_lock(&path, Duration::from_millis(50), Duration::from_millis(5), Duration::from_secs(1));
        assert!(matches!(err, Err(LockError::Timeout(_))));
        // Setelah lock1 drop, writer berikutnya berhasil.
        drop(_lock1);
        let lock2 = acquire_write_lock(&path, Duration::from_millis(100), Duration::from_millis(5), Duration::from_secs(1)).unwrap();
        drop(lock2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_stale_lock_taken_over() {
        let root = test_root("stale");
        // Lockfile basi (buat langsung tanpa guard; mtime = sekarang, tapi
        // stale_after sangat pendek → dianggap mati).
        let path = lockfile(&root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"stale").unwrap();
        let lock = acquire_write_lock(&path, Duration::from_millis(200), Duration::from_millis(5), Duration::from_millis(1)).unwrap();
        assert!(is_writer_locked(&path));
        drop(lock);
        let _ = std::fs::remove_dir_all(&root);
    }
}
