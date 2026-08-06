//! txn.rs — transaction + journal untuk MICD (Kritik 4 & 5 db.md).
//!
//! Save multi-store bersifat all-or-nothing per file (temp + rename sudah
//! atomik), tapi menulis N store berurutan masih bisa membuat database
//! setengah ter-update bila proses crash di tengah jalan. Journal menyelesaikan
//! dua masalah sekaligus:
//!
//! 1. **Transaction intent** (Kritik 4): sebelum menulis, simpan daftar file
//!    yang ikut transaksi. Pemulihan tahu store mana yang harus konsisten.
//! 2. **Crash recovery** (Kritik 5): bila proses mati saat commit (renames),
//!    journal tersisa di disk. Saat open berikutnya, recovery memvalidasi
//!    setiap store via checksum MDB1; store yang corrupt dibuang (dibangun
//!    ulang di save berikutnya), sisanya dipertahankan. Journal lalu dihapus.
//!
//! Format journal: magic `MJRN` + version + bincode `Journal` (daftar path).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::micd::format::MdbReader;

/// Magic header file journal.
pub const JOURNAL_MAGIC: [u8; 4] = *b"MJRN";
/// Versi format journal.
pub const JOURNAL_VERSION: u32 = 1;

/// Intent log: daftar store yang ikut transaksi (relatif terhadap root bila
/// memungkinkan, absolut bila berada di luar root).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Journal {
    pub files: Vec<PathBuf>,
}

impl Journal {
    pub fn new(files: Vec<PathBuf>) -> Self {
        Journal { files }
    }
}

/// Tulis journal secara atomik (temp + rename) + sync ke disk. Journal harus
/// sudah ter-persist sebelum store pertama ditulis — itu jaminan bahwa recovery
/// selalu tahu transaksi yang belum selesai.
pub fn write_journal(path: &Path, files: &[PathBuf]) -> std::io::Result<()> {
    let j = Journal::new(files.to_vec());
    let bytes = bincode::serialize(&j).map_err(std::io::Error::other)?;
    let mut out = Vec::with_capacity(JOURNAL_MAGIC.len() + 4 + bytes.len());
    out.extend_from_slice(&JOURNAL_MAGIC);
    out.extend_from_slice(&JOURNAL_VERSION.to_le_bytes());
    out.extend_from_slice(&bytes);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("mdb.jrn");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&out)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Baca journal. `None` bila tidak ada, corrupt, atau versi tak dikenal.
pub fn read_journal(path: &Path) -> Option<Journal> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < JOURNAL_MAGIC.len() + 4 {
        return None;
    }
    if &bytes[0..4] != &JOURNAL_MAGIC {
        return None;
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    if version != JOURNAL_VERSION {
        return None;
    }
    bincode::deserialize(&bytes[8..]).ok()
}

/// Crash recovery (Kritik 5). Untuk setiap store di journal: validasi checksum
/// MDB1. Store valid dipertahankan; store corrupt/hilang dihapus sehingga
/// dibangun ulang di save berikutnya. Journal dihapus setelah diproses.
/// Mengembalikan jumlah store yang dibuang (corrupt).
pub fn recover(root: &Path, journal_path: &Path) -> usize {
    let Some(j) = read_journal(journal_path) else {
        // Journal korup/partial → tidak bisa dipercaya → buang saja.
        // Store sendiri punya checksum, jadi yang rusak tetap terdeteksi saat
        // dibuka (pemanggil open() sudah fallback ke store kosong).
        let _ = std::fs::remove_file(journal_path);
        return 0;
    };
    let mut discarded = 0;
    for f in &j.files {
        let full = if f.is_absolute() {
            f.clone()
        } else {
            root.join(f)
        };
        if MdbReader::open(&full).is_err() {
            let _ = std::fs::remove_file(&full);
            discarded += 1;
        }
    }
    let _ = std::fs::remove_file(journal_path);
    discarded
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "maria_micd_txn_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_journal_roundtrip() {
        let dir = test_dir("roundtrip");
        let path = dir.join("journal.mdb");
        let files = vec![
            PathBuf::from("metadata.mdb"),
            PathBuf::from("graph.mdb"),
        ];
        write_journal(&path, &files).unwrap();
        let j = read_journal(&path).expect("journal terbaca");
        assert_eq!(j.files, files);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_journal_missing() {
        let dir = test_dir("missing");
        assert!(read_journal(&dir.join("nope.mdb")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_journal_corrupt() {
        let dir = test_dir("corrupt");
        let path = dir.join("journal.mdb");
        std::fs::write(&path, b"garbage-not-journal").unwrap();
        assert!(read_journal(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_recover_discards_corrupt_keeps_valid() {
        let dir = test_dir("recover");
        let journal_path = dir.join("journal.mdb");

        // Store valid: tulis MDB1 asli.
        let mut w = crate::micd::format::MdbWriter::new();
        w.put(1, crate::micd::format::KIND_STRING, b"ok".to_vec());
        let valid = dir.join("valid.mdb");
        w.write_to(&valid).unwrap();

        // Store corrupt: bytes acak.
        let corrupt = dir.join("corrupt.mdb");
        std::fs::write(&corrupt, vec![0xFF; 128]).unwrap();

        write_journal(&journal_path, &[valid.clone(), corrupt.clone()]).unwrap();
        let discarded = recover(&dir, &journal_path);

        assert_eq!(discarded, 1, "store corrupt harus dibuang");
        assert!(valid.exists(), "store valid dipertahankan");
        assert!(!corrupt.exists(), "store corrupt dihapus");
        assert!(!journal_path.exists(), "journal dihapus setelah recovery");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_recover_missing_journal_noop() {
        let dir = test_dir("nojournal");
        let journal_path = dir.join("journal.mdb");
        assert_eq!(recover(&dir, &journal_path), 0);
        assert!(!journal_path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
