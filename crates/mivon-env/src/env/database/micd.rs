//! Helpers buka/identifikasi MICD — membungkus `MicdDatabase` root & project id.

use mivon_compiler::micd::MicdDatabase;
use std::path::{Path, PathBuf};

/// Root default database (`.mivon/database`, override `MIVON_MICD_DIR`)
/// berdasarkan CWD. F37 fix: `MicdDatabase::default_root()` (CWD-relative)
/// diganti `database_root_for` (root eksplisit) di jalur produksi — fungsi ini
/// didelegasikan agar tidak ada dua sumber kebenaran yang divergen.
pub fn default_database_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    database_root_for(&cwd)
}

/// F37 fix: root database untuk project tertentu — CWD-independen. Sebelumnya
/// `default_database_root()` memakai `.mivon` RELATIF cwd → saat cwd berada di
/// direktori crate (mis. `cargo test -p mivon-env`), database MICD dibuat di
/// `crates/mivon-env/.mivon/` — salah tempat. Root dihitung dari workspace
/// root eksplisit. Prioritas:
/// 1. env `MIVON_MICD_DIR` (override manual, selalu dipakai).
/// 2. `<root>/.mivon/database` bila `<root>/.mivon` kosong/berupa folder.
/// 3. `<root>/.mivon_db/database` bila `<root>/.mivon` adalah FILE (project
///    file `-f` — `.mivon` tidak bisa jadi folder database sekaligus).
pub fn database_root_for(root: &Path) -> PathBuf {
    if let Ok(p) = std::env::var("MIVON_MICD_DIR") {
        return PathBuf::from(p);
    }
    let mivon_path = root.join(".mivon");
    match std::fs::metadata(&mivon_path) {
        Ok(m) if m.is_file() => root.join(".mivon_db").join("database"),
        _ => mivon_path.join("database"),
    }
}

/// Project id dari cwd + sources + incdirs + defines.
pub fn project_id_for(
    root: &Path,
    sources: &[PathBuf],
    incdirs: &[PathBuf],
    defines: &[(String, String)],
) -> String {
    MicdDatabase::project_id(root, sources, incdirs, defines)
}

/// Buka database project. Membuat direktori bila belum ada.
pub fn open_database(db_root: &Path, pid: &str) -> MicdDatabase {
    MicdDatabase::open_project(db_root, pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_root_nonempty() {
        let r = default_database_root();
        assert!(!r.as_os_str().is_empty());
    }

    // Test memanipulasi env var MIVON_MICD_DIR — kunci global agar tidak
    // racy dengan test lain yang membaca env var yang sama (test paralel).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_database_root_for_is_root_based() {
        // F37: root database harus relatif ke workspace root, bukan cwd.
        // MIVON_MICD_DIR di-unset (dalam lock) agar tidak memengaruhi test.
        let _guard = ENV_LOCK.lock().unwrap();
        let saved = std::env::var("MIVON_MICD_DIR").ok();
        std::env::remove_var("MIVON_MICD_DIR");
        let r = database_root_for(Path::new("/some/project"));
        if let Some(v) = saved {
            std::env::set_var("MIVON_MICD_DIR", v);
        }
        assert_eq!(
            r,
            PathBuf::from("/some/project/.mivon/database"),
            "root harus berdasarkan workspace root, bukan cwd"
        );
    }

    #[test]
    fn test_project_id_deterministic() {
        let root = Path::new("/proj");
        let sources = vec![PathBuf::from("a.sv"), PathBuf::from("b.sv")];
        let a = project_id_for(root, &sources, &[], &[]);
        let b = project_id_for(root, &sources, &[], &[]);
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }
}
