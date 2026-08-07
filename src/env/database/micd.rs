//! Helpers buka/identifikasi MICD — membungkus `MicdDatabase` root & project id.

use crate::micd::MicdDatabase;
use std::path::{Path, PathBuf};

/// Root default database (`.maria/database`, override `MARIA_MICD_DIR`).
pub fn default_database_root() -> PathBuf {
    MicdDatabase::default_root()
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
