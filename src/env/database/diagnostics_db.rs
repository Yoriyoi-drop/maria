use crate::micd::MicdDatabase;
use std::path::Path;

/// Jumlah diagnostic tersimpan per file (untuk query IDE tanpa compile ulang).
pub fn diag_count(db: &MicdDatabase, path: &Path) -> usize {
    db.diags
        .get(path)
        .map(|d| d.error_count() + d.warning_count())
        .unwrap_or(0)
}

/// Total diagnostic di seluruh file.
pub fn total_diag_count(db: &MicdDatabase) -> usize {
    db.diags
        .values()
        .map(|d| d.error_count() + d.warning_count())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::micd::MicdDatabase;
    use std::path::Path;

    #[test]
    fn test_diag_queries_empty() {
        let root = std::env::temp_dir().join("maria_db_diag");
        let _ = std::fs::create_dir_all(&root);
        let db = MicdDatabase::open_project(&root, "pid-d");
        assert_eq!(diag_count(&db, Path::new("a.sv")), 0);
        assert_eq!(total_diag_count(&db), 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
